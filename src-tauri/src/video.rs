use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::{Notify, OwnedSemaphorePermit};

use crate::client::XtClient;
use crate::courses;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoTaskParams {
    pub user_id: i64,
    pub classroom_id: i64,
    pub sku_id: i64,
    pub course_id: i64,
    pub sign: String,
    pub leaf_id: i64,
    pub video_ccid: String,
    pub duration: f64,
    /// 起始播放位置（秒）。None=从头开始
    pub start_position: Option<f64>,
    /// 播放倍速，默认 1.0
    pub speed: Option<f32>,
    /// 心跳间隔（毫秒），默认 5000
    pub interval_ms: Option<u64>,
    /// 用于 UI 展示的节点名（leaf 名称）
    pub leaf_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoTaskStatus {
    pub task_id: String,
    pub leaf_id: i64,
    pub leaf_name: Option<String>,
    pub classroom_id: i64,
    pub current_pos: f64,
    pub duration: f64,
    pub finished: bool,
    pub error: Option<String>,
    /// 是否用户主动取消（区别于正常播完）。
    /// cancelled=true 时前端不应把 leaf 标记为已完成。
    pub cancelled: bool,
    /// 是否在等待队列中（未实际开始执行）
    pub queued: bool,
}

pub struct VideoTaskHandle {
    pub task_id: String,
    pub params: VideoTaskParams,
    pub cancel: Arc<Notify>,
    pub status: Mutex<VideoTaskStatus>,
}

fn rand_pg(leaf_id: i64) -> String {
    let mut rng = rand::thread_rng();
    let chars: String = (0..5)
        .map(|_| {
            let c = rng.gen_range(0..36u8);
            if c < 10 {
                (b'0' + c) as char
            } else {
                (b'a' + c - 10) as char
            }
        })
        .collect();
    format!("{leaf_id}_{chars}")
}

fn build_event(
    et: &str,
    cp: f64,
    fp: f64,
    tp: f64,
    sp: f32,
    sq: u64,
    pg: &str,
    p: &VideoTaskParams,
) -> Value {
    json!({
        "i": 5,
        "et": et,
        "p": "web",
        "n": "ali-cdn.xuetangx.com",
        "lob": "plat2",
        "cp": (cp * 10.0).round() / 10.0,
        "fp": (fp * 10.0).round() / 10.0,
        "tp": (tp * 10.0).round() / 10.0,
        "sp": sp,
        "ts": format!("{}", chrono::Utc::now().timestamp_millis()),
        "u": p.user_id,
        "uip": "",
        "c": p.course_id,
        "v": p.leaf_id,
        "skuid": p.sku_id,
        "classroomid": p.classroom_id.to_string(),
        "cc": p.video_ccid,
        "d": p.duration,
        "pg": pg,
        "sq": sq,
        "t": "video",
        "cards_id": 0,
        "slide": 0,
        "v_url": ""
    })
}

async fn send_heartbeat(client: &XtClient, events: Vec<Value>) -> Result<()> {
    let body = json!({ "heart_data": events });
    client.post_json("/video-log/heartbeat/", &body).await?;
    Ok(())
}

pub async fn run_video_task(
    app: AppHandle<Wry>,
    handle: Arc<VideoTaskHandle>,
    client: Arc<XtClient>,
    permit: OwnedSemaphorePermit,
) {
    let result = run_video_task_inner(&app, &handle, &client).await;
    let snapshot = {
        let mut status = handle.status.lock();
        status.finished = true;
        match &result {
            Ok(reason) => match reason {
                FinishReason::Completed => {
                    status.cancelled = false;
                }
                FinishReason::Cancelled => {
                    status.cancelled = true;
                }
            },
            Err(e) => {
                status.error = Some(format!("{e}"));
                // 错误终止：不是用户取消，但也不算完成，前端应当**不**把 leaf 标完成
                status.cancelled = true;
                log::error!("视频任务 {} 失败: {e:?}", handle.task_id);
            }
        }
        status.clone()
    };
    let _ = app.emit("video://done", &snapshot);

    // 显式 drop permit，把槽位归还给 Semaphore；
    // 紧接着尝试唤起队列中的下一个视频任务（drop 必须发生在 try_dequeue 之前，
    // 否则下一个任务 try_acquire 拿不到 permit 又会被丢回队列）。
    drop(permit);
    try_dequeue_and_start(&app);
}

enum FinishReason {
    Completed,
    Cancelled,
}

async fn run_video_task_inner(
    app: &AppHandle<Wry>,
    handle: &VideoTaskHandle,
    client: &XtClient,
) -> Result<FinishReason> {
    let p = handle.params.clone();
    if p.duration <= 0.0 {
        return Err(anyhow!("视频时长为 0，拒绝启动（可能是非视频节点或未发布）"));
    }
    if p.video_ccid.trim().is_empty() {
        return Err(anyhow!("视频 ccid 为空，拒绝启动"));
    }
    let pg = rand_pg(p.leaf_id);
    let interval = std::time::Duration::from_millis(p.interval_ms.unwrap_or(5000));
    let speed = p.speed.unwrap_or(1.0).clamp(0.5, 2.0);
    let start = p.start_position.unwrap_or(0.0).clamp(0.0, p.duration);

    let mut sq: u64 = 0;
    // 心跳位置以 Instant 墙钟为基线：cp = start + (now - task_start) * speed。
    // 这样即使 tokio::sleep 因系统挂起 / 进程被冻结而拉长，cp 也不会累积漂移，
    // 长视频末段也能可靠地达到 duration → 发 `ended` → 上报完成。
    let task_start = Instant::now();
    let mut cp = start;
    let fp = start;
    let tp = fp;

    // 心跳事件序号使用专用闭包以避免在宏参数里写副作用，且求值顺序更明显。
    let next_sq = |sq: &mut u64| -> u64 {
        *sq += 1;
        *sq
    };

    // 1) 启动事件序列
    let bootstrap_events = vec![
        build_event("loadstart", 0.0, 0.0, 0.0, speed, next_sq(&mut sq), &pg, &p),
        build_event("loadeddata", cp, fp, tp, speed, next_sq(&mut sq), &pg, &p),
        build_event("play", cp, fp, tp, speed, next_sq(&mut sq), &pg, &p),
        build_event("playing", cp, fp, tp, speed, next_sq(&mut sq), &pg, &p),
    ];
    send_heartbeat(client, bootstrap_events).await?;

    // 2) 心跳循环。连续失败超过阈值视为致命错误，终止任务而不是假装播完。
    const MAX_CONSECUTIVE_FAILURES: u32 = 3;
    let mut consecutive_failures: u32 = 0;
    loop {
        tokio::select! {
            _ = handle.cancel.notified() => {
                let pause_evt = build_event("pause", cp, fp, tp, speed, next_sq(&mut sq), &pg, &p);
                let _ = send_heartbeat(client, vec![pause_evt]).await;
                return Ok(FinishReason::Cancelled);
            }
            _ = tokio::time::sleep(interval) => {
                // 用 Instant 墙钟重新计算位置，sleep 只控制 cadence。
                let elapsed = task_start.elapsed().as_secs_f64();
                cp = (start + elapsed * (speed as f64)).min(p.duration);
                sq += 1;
                let evt = build_event("heartbeat", cp, fp, tp, speed, sq, &pg, &p);
                match send_heartbeat(client, vec![evt]).await {
                    Ok(()) => {
                        consecutive_failures = 0;
                        handle.status.lock().error = None;
                    }
                    Err(e) => {
                        consecutive_failures += 1;
                        let msg = format!("{e}");
                        handle.status.lock().error = Some(msg.clone());
                        let _ = app.emit("video://error", json!({
                            "task_id": handle.task_id,
                            "message": msg,
                            "consecutive_failures": consecutive_failures,
                        }));
                        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                            return Err(anyhow!(
                                "心跳连续失败 {consecutive_failures} 次，终止任务（最后一次错误：{e}）"
                            ));
                        }
                        // 当前轮失败：不更新 current_pos，进入下一轮重试
                        continue;
                    }
                }
                let progress_snapshot = {
                    let mut s = handle.status.lock();
                    s.current_pos = cp;
                    s.clone()
                };
                let _ = app.emit("video://progress", &progress_snapshot);
                if cp >= p.duration {
                    let end_evt = build_event("ended", cp, fp, tp, speed, next_sq(&mut sq), &pg, &p);
                    let _ = send_heartbeat(client, vec![end_evt]).await;
                    return Ok(FinishReason::Completed);
                }
            }
        }
    }
}

/// 从 leaf_info 提取视频元数据，构造可直接用于刷课的参数。
/// 学堂在线的链路：
///   leaf_info.content.media.ccid → 视频 CC 标识
///   GET /api/v1/lms/service/playurl/{ccid}/?appid=10000 → 真实 duration
/// leaf_info 自带的 duration 几乎总是 0，必须再请求 playurl 才能拿到真实时长。
pub async fn build_task_params(
    client: &XtClient,
    user_id: i64,
    classroom_id: i64,
    sku_id: i64,
    sign: &str,
    leaf_id: i64,
) -> Result<VideoTaskParams> {
    let info = courses::leaf_info(client, classroom_id, leaf_id, sign).await?;
    let course_id = info
        .get("course_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("leaf_info 缺 course_id（该节点可能不是视频）"))?;

    // 1) 在 leaf_info 中递归找 ccid（leaf 可能挂多个内容，视频对象不一定在 content.media）
    let ccid = extract_ccid(&info)
        .ok_or_else(|| anyhow!("该节点未找到视频 ccid，可能是纯文档/作业节点"))?;
    if ccid.is_empty() {
        return Err(anyhow!("视频 ccid 为空"));
    }

    // 2) 调 playurl 拿真实 duration
    let play = client
        .get_json(&format!("/api/v1/lms/service/playurl/{ccid}/?appid=10000"))
        .await?;
    let duration = play
        .get("data")
        .and_then(|d| d.get("duration"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    if duration <= 0.0 {
        return Err(anyhow!("playurl 未返回有效 duration（ccid={ccid}）"));
    }

    Ok(VideoTaskParams {
        user_id,
        classroom_id,
        sku_id,
        course_id,
        sign: sign.to_string(),
        leaf_id,
        video_ccid: ccid,
        duration,
        start_position: None,
        speed: None,
        interval_ms: None,
        leaf_name: None,
    })
}

/// 在 leaf_info 子树中递归找第一个非空 ccid（兼容 `content.media.ccid`、
/// 数组形式 content、嵌套的视频对象等）。
fn extract_ccid(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(c) = map.get("ccid").and_then(|x| x.as_str()) {
                if !c.is_empty() {
                    return Some(c.to_string());
                }
            }
            for (_, child) in map.iter() {
                if let Some(found) = extract_ccid(child) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for child in arr {
                if let Some(found) = extract_ccid(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// 从等待队列中取出一个任务并启动（前提：能从 `task_semaphore` 抢到一个 permit）。
/// 该函数是非阻塞的——抢不到 permit 就直接返回，等下一次 release 时再被调用。
///
/// `task_semaphore` 受 `task_concurrency` 配置控制：例如设为 3 时，最多同时跑 3 个
/// 视频，剩余按入队顺序 FIFO 排队。前面的视频 drop permit 后立刻触发出队下一个。
fn try_dequeue_and_start(app: &AppHandle<Wry>) {
    let state = app.state::<crate::state::AppState>();

    // 1) 先尝试抢一个 permit。抢不到说明并发已满，留着队列里的任务等下次机会。
    let Ok(permit) = state.task_semaphore.clone().try_acquire_owned() else {
        return;
    };

    // 2) 从队头取一个任务。空队列 → 把 permit 还回去（drop）。
    let Some(next_handle) = state.pending_video_tasks.write().pop_front() else {
        return; // permit 在此 drop，自动归还
    };

    // 3) 获取该任务对应账号的 client。失败的话 emit done 并归还 permit。
    let uid = next_handle.params.user_id;
    let client = match state.client_for(uid) {
        Ok(c) => c,
        Err(e) => {
            let snapshot = {
                let mut s = next_handle.status.lock();
                s.finished = true;
                s.queued = false;
                s.cancelled = true;
                s.error = Some(format!("排队任务无法获取客户端：{e}"));
                s.clone()
            };
            let _ = app.emit("video://done", &snapshot);
            // permit 在函数返回时 drop，自动归还
            return;
        }
    };

    // 4) 标记为已出队并 spawn。permit 转移给 run_video_task，它结束时再 drop。
    next_handle.status.lock().queued = false;
    let app_clone = app.clone();
    tokio::spawn(async move {
        run_video_task(app_clone, next_handle, client, permit).await;
    });
}

/// 终止所有 user_id ≠ `keep_user_id` 的视频任务：
/// - 等待队列中的旧账号项：直接移除，标记 cancelled，emit `video://done`
/// - 已 spawn 的运行中旧账号项：发 cancel notify（run_video_task 自然走 pause→done 流程），
///   同时从 `video_tasks` 表里立刻移除，避免之后 `list_video_tasks` 还能看见
///
/// 由 `switch_account` 命令在切换账号成功后调用。这样切账号后旧账号不再继续
/// 心跳、不再占着 `task_semaphore` 槽位，前端 `useVideoState` 重建后 `listVideoTasks`
/// 也只会看到属于新账号的任务。
pub fn terminate_tasks_except_user(app: &AppHandle<Wry>, keep_user_id: i64) {
    terminate_tasks_matching(app, |h| h.params.user_id != keep_user_id);
}

/// 终止所有 user_id == `target_user_id` 的视频任务。
/// `remove_account` 命令在移除账号时调用，保证该账号一旦被删，对应的运行中视频任务
/// 也立即终止，避免「账号不在了但还在心跳」的诡异状态。
pub fn terminate_tasks_for_user(app: &AppHandle<Wry>, target_user_id: i64) {
    terminate_tasks_matching(app, |h| h.params.user_id == target_user_id);
}

/// 按谓词终止视频任务的通用实现。`predicate(&handle)` 返回 true 的任务会被终止：
/// - 在等待队列里的，从队列移除并 emit done
/// - 在跑的，notify cancel 后从 video_tasks 表移除（run_video_task 收尾时自然 emit done）
///
/// 分三步是为了避免在持有 video_tasks 写锁时再发 notify_waiters —— 任务唤醒后的 done
/// 回调可能也想读 video_tasks，产生不必要的锁竞争 / 死锁风险。
fn terminate_tasks_matching<F>(app: &AppHandle<Wry>, mut predicate: F)
where
    F: FnMut(&VideoTaskHandle) -> bool,
{
    let state = app.state::<crate::state::AppState>();

    // 1) 清等待队列里命中谓词的任务
    let dropped_from_queue: Vec<Arc<VideoTaskHandle>> = {
        let mut queue = state.pending_video_tasks.write();
        let mut keep: std::collections::VecDeque<Arc<VideoTaskHandle>> =
            std::collections::VecDeque::with_capacity(queue.len());
        let mut dropped = Vec::new();
        while let Some(h) = queue.pop_front() {
            if predicate(&h) {
                dropped.push(h);
            } else {
                keep.push_back(h);
            }
        }
        *queue = keep;
        dropped
    };
    for h in dropped_from_queue {
        let snapshot = {
            let mut s = h.status.lock();
            s.finished = true;
            s.cancelled = true;
            s.queued = false;
            s.clone()
        };
        state.video_tasks.write().remove(&h.task_id);
        let _ = app.emit("video://done", &snapshot);
    }

    // 2) 终止运行中命中谓词的任务
    let to_cancel: Vec<Arc<VideoTaskHandle>> = state
        .video_tasks
        .read()
        .values()
        .filter(|h| predicate(h))
        .cloned()
        .collect();
    {
        let mut tasks = state.video_tasks.write();
        for h in &to_cancel {
            tasks.remove(&h.task_id);
        }
    }
    for h in to_cancel {
        // 同 stop_video_task 一致：用 notify_waiters 触发 select! 分支走 pause→done。
        // run_video_task 结束时仍会 emit `video://done`，前端 onDone 找不到对应卡片
        // 就忽略，无副作用。
        h.cancel.notify_waiters();
    }
}
