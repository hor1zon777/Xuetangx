use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::OwnedSemaphorePermit;
use uuid::Uuid;

use crate::accounts::Account;
use crate::ai::test_settings;
use crate::bank::{BankEntry, BankStats};
use crate::courses::{self, CourseSummary, EvaluationDetail, LeafNode};
use crate::exercise::{
    self, auto_run_exercise_with_captcha, CaptchaChallenge, ExerciseList, ExerciseProbe,
    SubmitDelay,
};
use crate::forum;
use crate::login;
use crate::state::{AiSettings, AppSettings, AppState};
use crate::video::{self, VideoTaskHandle, VideoTaskParams, VideoTaskStatus};

fn err_str<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}

/// 获取一个任务槽位。返回的 permit drop 时自动归还，
/// 即便 await 后续 panic 也不会泄漏。
///
/// 对于"槽满"的情况，await 会挂起在 Semaphore 上，由 tokio 公平唤醒。
async fn acquire_task_slot(state: &AppState) -> OwnedSemaphorePermit {
    state
        .task_semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("task_semaphore 不应被 close")
}

/// 非阻塞地尝试获取一个槽位。槽满时返回 None，用于视频任务排队判定
/// （不能阻塞 startVideo 这个 Tauri command，否则前端 await 会卡住）。
///
/// 视频任务和其它任务（作业/评论/图文）共享同一个 `task_semaphore`，
/// 受 `task_concurrency` 配置控制：`task_concurrency=3` 时最多同时跑 3 个，
/// 超出的视频按到达顺序入 `pending_video_tasks` 队列等待，前面的跑完再出队。
fn try_acquire_task_slot(state: &AppState) -> Option<OwnedSemaphorePermit> {
    state.task_semaphore.clone().try_acquire_owned().ok()
}

#[tauri::command]
pub fn list_accounts(state: tauri::State<'_, AppState>) -> Vec<Account> {
    state.accounts.read().values().cloned().collect()
}

#[tauri::command]
pub fn switch_account(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    user_id: i64,
) -> Result<(), String> {
    state.switch(&app, user_id).map_err(err_str)?;
    // 切账号后，旧账号已经 spawn 的视频任务还在心跳、占着 task_semaphore 槽位。
    // 终止它们，让前端 listVideoTasks 不再看到旧账号任务，新账号的并发也能立刻可用。
    video::terminate_tasks_except_user(&app, user_id);
    Ok(())
}

#[tauri::command]
pub fn remove_account(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    user_id: i64,
) -> Result<(), String> {
    // 顺手把该账号正在跑的视频任务也终止——账号都没了任务还心跳就太怪了。
    // 必须在 state.remove_account 之前调用：等账号被移除后，client_for 会失败，
    // 队列中等待出队的任务会因取不到 client 直接报错。这里提前把它们干净地停掉。
    video::terminate_tasks_for_user(&app, user_id);
    state.remove_account(&app, user_id).map_err(err_str)
}

#[tauri::command]
pub fn current_account(state: tauri::State<'_, AppState>) -> Option<Account> {
    state.current_account()
}

#[tauri::command]
pub async fn check_login(app: AppHandle<Wry>) -> Result<bool, String> {
    let state: tauri::State<AppState> = app.state();
    let Ok(client) = state.current_client() else {
        return Ok(false);
    };
    login::check_is_login(&client).await.map_err(err_str)
}

#[tauri::command]
pub async fn start_login(app: AppHandle<Wry>) -> Result<(), String> {
    login::start_login_flow(app).await.map_err(err_str)
}

#[tauri::command]
pub fn cancel_login(app: AppHandle<Wry>) {
    login::cancel_login(&app);
}

#[tauri::command]
pub async fn list_courses(app: AppHandle<Wry>) -> Result<Vec<CourseSummary>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let res = courses::list_my_courses(&client).await.map_err(err_str);
    if let Some(uid) = *state.current_user_id.read() {
        let _ = state.save_account_cookies(&app, uid);
    }
    res
}

#[tauri::command]
pub async fn list_chapters(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
) -> Result<Vec<LeafNode>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::list_chapters(&client, classroom_id, &sign)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn leaf_info(
    app: AppHandle<Wry>,
    classroom_id: i64,
    leaf_id: i64,
    sign: String,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::leaf_info(&client, classroom_id, leaf_id, &sign)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn course_schedule(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
) -> Result<std::collections::HashMap<i64, f64>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::course_schedule(&client, classroom_id, &sign)
        .await
        .map_err(err_str)
}

/// "总成绩"页一次性拉真实数据：总分 + 等级 + 5 个分项明细 + 每个 leaf 的实得分。
/// 一次接口调用拿全，不需要批量拉 leaf_info。
#[tauri::command]
pub async fn course_evaluation_detail(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
) -> Result<EvaluationDetail, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::course_evaluation_detail(&client, classroom_id, &sign)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn batch_exercise_ids(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
) -> Result<std::collections::HashMap<i64, i64>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::batch_exercise_ids(client, classroom_id, sign, leaf_ids)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn batch_exercise_kinds(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
    sku_id: i64,
    items: Vec<(i64, i64)>,
) -> Result<std::collections::HashMap<i64, std::collections::HashMap<String, i64>>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::batch_exercise_kinds(client, classroom_id, sign, sku_id, items)
        .await
        .map_err(err_str)
}

#[derive(Deserialize)]
pub struct StartVideoArgs {
    pub classroom_id: i64,
    pub sku_id: i64,
    pub sign: String,
    pub leaf_id: i64,
    /// 可选：手动覆盖 ccid / duration / start_position / speed / interval_ms
    pub override_params: Option<VideoTaskParams>,
    /// 可选：仅覆盖播放偏好（倍速、心跳间隔、起始位置），ccid/duration 仍由后端探测
    pub speed: Option<f32>,
    pub interval_ms: Option<u64>,
    pub start_position: Option<f64>,
    /// 节点名，用于 UI 显示
    pub leaf_name: Option<String>,
}

#[tauri::command]
pub async fn start_video_task(
    app: AppHandle<Wry>,
    args: StartVideoArgs,
) -> Result<VideoTaskStatus, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let uid = state
        .current_user_id
        .read()
        .clone()
        .ok_or_else(|| "未选择账号".to_string())?;

    // 先解析视频参数（锁外，允许多个调用并行拉取元数据）
    let mut params = if let Some(p) = args.override_params {
        p
    } else {
        video::build_task_params(&client, uid, args.classroom_id, args.sku_id, &args.sign, args.leaf_id)
            .await
            .map_err(err_str)?
    };
    if let Some(sp) = args.speed {
        params.speed = Some(sp);
    }
    if let Some(iv) = args.interval_ms {
        params.interval_ms = Some(iv);
    }
    if let Some(sp) = args.start_position {
        params.start_position = Some(sp);
    } else {
        // 未显式指定起始位置时，从课程进度中获取上次播放位置
        if let Ok(schedule) =
            courses::course_schedule(&client, params.classroom_id, &params.sign).await
        {
            if let Some(&ratio) = schedule.get(&params.leaf_id) {
                if ratio > 0.0 && ratio < 1.0 {
                    let pos = ratio * params.duration;
                    if pos < params.duration {
                        params.start_position = Some(pos);
                    }
                }
            }
        }
    }
    if args.leaf_name.is_some() {
        params.leaf_name = args.leaf_name.clone();
    }
    {
        let s = state.settings.read();
        if params.speed.is_none() {
            params.speed = s.video_speed;
        }
        if params.interval_ms.is_none() {
            params.interval_ms = s.heartbeat_interval_ms;
        }
    }

    let task_id = Uuid::new_v4().to_string();
    let status = VideoTaskStatus {
        task_id: task_id.clone(),
        leaf_id: params.leaf_id,
        leaf_name: params.leaf_name.clone(),
        classroom_id: params.classroom_id,
        current_pos: params.start_position.unwrap_or(0.0),
        duration: params.duration,
        finished: false,
        error: None,
        cancelled: false,
        queued: false,
    };
    let handle = Arc::new(VideoTaskHandle {
        task_id: task_id.clone(),
        params: params.clone(),
        cancel: Arc::new(tokio::sync::Notify::new()),
        status: parking_lot::Mutex::new(status.clone()),
    });

    // 注册 handle 到全局表。无论是否立即执行，前端都能查询/取消该任务。
    state
        .video_tasks
        .write()
        .insert(task_id.clone(), handle.clone());

    // 尝试立即抢一个槽位。抢到 → 立即 spawn；抢不到 → 入队等待。
    // task_concurrency 决定 permits 总数；视频与作业/评论等共享同一组 permit。
    // 队列消费由 video::run_video_task 结束时调用 try_dequeue_and_start 触发。
    let permit = try_acquire_task_slot(&state);
    let result_status;
    match permit {
        Some(p) => {
            result_status = handle.status.lock().clone();
            let app_clone = app.clone();
            let handle_for_spawn = handle.clone();
            tokio::spawn(async move {
                video::run_video_task(app_clone, handle_for_spawn, client, p).await;
            });
        }
        None => {
            handle.status.lock().queued = true;
            state
                .pending_video_tasks
                .write()
                .push_back(handle.clone());
            result_status = handle.status.lock().clone();
        }
    }

    Ok(result_status)
}

#[tauri::command]
pub fn stop_video_task(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    // 检查是否在等待队列中，是则直接从队列移除并标记取消
    {
        let mut queue = state.pending_video_tasks.write();
        if let Some(pos) = queue.iter().position(|h| h.task_id == task_id) {
            queue.remove(pos);
            drop(queue);
            if let Some(h) = state.video_tasks.read().get(&task_id).cloned() {
                let snapshot = {
                    let mut s = h.status.lock();
                    s.finished = true;
                    s.cancelled = true;
                    s.queued = false;
                    s.clone()
                };
                // 前端依赖 video://done 事件来移除卡片/解锁 UI；
                // 队列中被取消的任务从未派出过 done 事件，必须在这里补发。
                let _ = app.emit("video://done", &snapshot);
            }
            return Ok(());
        }
    }
    // 否则通知正在运行的任务取消
    if let Some(t) = state.video_tasks.read().get(&task_id).cloned() {
        t.cancel.notify_waiters();
    }
    Ok(())
}

#[tauri::command]
pub fn list_video_tasks(state: tauri::State<'_, AppState>) -> Vec<VideoTaskStatus> {
    state
        .video_tasks
        .read()
        .values()
        .map(|h| h.status.lock().clone())
        .collect()
}

#[tauri::command]
pub async fn send_comment(
    app: AppHandle<Wry>,
    classroom_id: i64,
    leaf_id: i64,
    sign: String,
    text: String,
    topic_type: Option<i64>,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let tt = topic_type.unwrap_or(0);
    let topic = forum::fetch_unit_discussion(&client, &sign, classroom_id, leaf_id, tt)
        .await
        .map_err(err_str)?;
    let resp = forum::post_comment(&client, classroom_id, leaf_id, topic.topic_id, topic.topic_owner_id, &text)
        .await
        .map_err(err_str)?;
    Ok(resp)
}

#[tauri::command]
pub async fn list_topic_comments(
    app: AppHandle<Wry>,
    topic_id: i64,
    classroom_id: i64,
    leaf_id: i64,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    forum::list_comments(
        &client,
        topic_id,
        classroom_id,
        leaf_id,
        offset.unwrap_or(0),
        limit.unwrap_or(10),
    )
    .await
    .map_err(err_str)
}

/// 批量检测当前账号是否在每个 leaf 的讨论区发过评论，同时把题目标题一起带回。
/// 并行拉每个 leaf 的 topic + 评论列表，返回 `{ leaf_id: MyTopicStatus }`。
/// 单个 leaf 出错时不影响其它，仅该 leaf 不出现在结果里。
///
/// 之所以把 title 也并到这里返回：`unit/discussion` 响应里没有真正的 title 字段，
/// 必须从 `content.text` 提取，而前端无论"区分节点"还是"判断已评论"都用得上同一份
/// 数据。如果分两次请求，对同一批 leaf 要发两遍 `unit/discussion`（每次都受限速影响），
/// 没必要。
///
/// `topic_type` 默认为 0（视频底下的旧讨论）。新增的"讨论（带分加）"节点
/// 需要传 4，否则后端拿不到正确的 topic_id。
#[tauri::command]
pub async fn batch_my_comment_status(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
    topic_type: Option<i64>,
) -> Result<std::collections::HashMap<i64, forum::MyTopicStatus>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let uid = state
        .current_user_id
        .read()
        .clone()
        .ok_or_else(|| "未选择账号".to_string())?;
    let tt = topic_type.unwrap_or(0);
    let mut handles = Vec::new();
    for leaf_id in leaf_ids {
        let c = client.clone();
        let s = sign.clone();
        handles.push(tokio::spawn(async move {
            forum::check_my_comment_status(&c, &s, classroom_id, leaf_id, tt, uid)
                .await
                .ok()
                .map(|info| (leaf_id, info))
        }));
    }
    let mut out = std::collections::HashMap::new();
    for h in handles {
        if let Ok(Some((id, info))) = h.await {
            out.insert(id, info);
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn auto_comment_leaf(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
    text: String,
    delay_ms: Option<u64>,
    topic_type: Option<i64>,
    // 评论成功后是否额外上报 `chapter/schedule`（让"讨论（带分加）"节点真正记分）。
    // 视频底下的讨论无需此步骤，调用方应传 false / None；leaf_type=4 时传 true。
    report_schedule: Option<bool>,
    // 仅在 `report_schedule=true` 时使用，写入 chapter/schedule 的 sku_id。
    sku_id: Option<i64>,
) -> Result<Vec<Value>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    // _permit drop 时自动释放槽位，即便后续 await 中 panic 也安全
    let _permit = acquire_task_slot(&state).await;
    let tt = topic_type.unwrap_or(0);
    let do_schedule = report_schedule.unwrap_or(false);
    let sku = sku_id.unwrap_or(0);

    // 学堂在线评论接口是"滑动 ~60s / 约 10 条"的限速，实测中：
    // - 1.5s 间隔会在第 11 条左右连续命中 429
    // - 服务端用 body 里的 `Expected available in N seconds.` 告知何时可用
    //
    // 因此这里采用 "稳态间隔 + 429 自适应回退" 策略：
    //   - 稳态默认 7s/条（约 8 条/分钟，留有余量）
    //   - 用户传入的 delay_ms 会被钳制到 [MIN_INTERVAL_MS, ..]，避免被前端误传导致打挂
    //   - 任意一次 429 后，把稳态间隔上调到 RATE_LIMIT_BACKOFF_INTERVAL_MS，剩余批次更保守
    //   - 命中 429 时按 server 给出的 retry_after sleep（+1s 兜底）后**重试该条**，最多 3 次
    //   - 进度通过 `forum://progress` 事件 emit 给前端，便于在 UI 显示"限速等待 N 秒"
    const DEFAULT_INTERVAL_MS: u64 = 7000;
    const MIN_INTERVAL_MS: u64 = 5000;
    const RATE_LIMIT_BACKOFF_INTERVAL_MS: u64 = 12000;
    const MAX_RETRY_PER_LEAF: u32 = 3;
    const FALLBACK_RETRY_AFTER_S: f64 = 45.0;

    let mut interval_ms = delay_ms.unwrap_or(DEFAULT_INTERVAL_MS).max(MIN_INTERVAL_MS);
    let total = leaf_ids.len();
    let mut out = Vec::with_capacity(total);
    let mut last_send: Option<std::time::Instant> = None;

    // 进度事件辅助闭包：阶段标识 + 当前进度 + 可选信息字段。
    // 前端可订阅 `forum://progress` 实现"限速等待 X 秒，还剩 Y 条"等提示。
    //
    // 注意：`interval_ms` 通过参数显式传入，而非由闭包借用，
    // 否则后续会因"借用未结束 + 再次赋值"被借用检查器拒绝。
    let emit_progress = {
        let app = app.clone();
        move |phase: &str, index: usize, interval_ms: u64, extra: Value| {
            let payload = json!({
                "phase": phase,
                "index": index,
                "total": total,
                "interval_ms": interval_ms,
                "extra": extra,
            });
            let _ = app.emit("forum://progress", payload);
        }
    };

    for (idx, leaf_id) in leaf_ids.into_iter().enumerate() {
        // 1. 稳态间隔等待（除第一条以外）
        if let Some(last) = last_send {
            let elapsed = last.elapsed().as_millis() as u64;
            if elapsed < interval_ms {
                let wait = interval_ms - elapsed;
                emit_progress(
                    "throttle",
                    idx,
                    interval_ms,
                    json!({ "wait_ms": wait, "leaf_id": leaf_id }),
                );
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
        }

        // 2. 先取 topic 信息（不计入限速：GET /unit/discussion 一般限制宽松）
        let topic = match forum::fetch_unit_discussion(&client, &sign, classroom_id, leaf_id, tt).await
        {
            Ok(t) => t,
            Err(e) => {
                let item = json!({ "leaf_id": leaf_id, "ok": false, "error": err_str(e) });
                // 立刻把这条失败结果回流给前端，避免等整批结束才看到
                emit_progress("item", idx, interval_ms, item.clone());
                out.push(item);
                last_send = Some(std::time::Instant::now());
                continue;
            }
        };

        // 3. 发评论，遇 429 自适应回退重试
        let mut attempt: u32 = 0;
        let item = loop {
            emit_progress(
                "sending",
                idx,
                interval_ms,
                json!({ "leaf_id": leaf_id, "attempt": attempt }),
            );
            match forum::post_comment_raw(
                &client,
                classroom_id,
                leaf_id,
                topic.topic_id,
                topic.topic_owner_id,
                &text,
            )
            .await
            {
                Ok((status, body)) => {
                    if status == 429 {
                        let ra = forum::parse_retry_after_seconds(&body)
                            .unwrap_or(FALLBACK_RETRY_AFTER_S);
                        // 命中过限速后，剩余批次都用更长的稳态间隔，降低再次命中概率
                        if interval_ms < RATE_LIMIT_BACKOFF_INTERVAL_MS {
                            interval_ms = RATE_LIMIT_BACKOFF_INTERVAL_MS;
                        }
                        if attempt < MAX_RETRY_PER_LEAF {
                            attempt += 1;
                            emit_progress(
                                "rate_limited",
                                idx,
                                interval_ms,
                                json!({
                                    "leaf_id": leaf_id,
                                    "retry_after_s": ra,
                                    "attempt": attempt,
                                }),
                            );
                            // sleep retry_after + 1s 缓冲
                            tokio::time::sleep(std::time::Duration::from_secs_f64(ra + 1.0))
                                .await;
                            continue;
                        } else {
                            break json!({
                                "leaf_id": leaf_id,
                                "ok": false,
                                "error": format!(
                                    "限速重试 {} 次仍失败：{}",
                                    MAX_RETRY_PER_LEAF, body
                                ),
                            });
                        }
                    } else if !(200..300).contains(&status) {
                        break json!({
                            "leaf_id": leaf_id,
                            "ok": false,
                            "error": format!("HTTP {}: {}", status, body),
                        });
                    } else {
                        // 成功：尽量把 body 解析为 JSON，失败则原样回传
                        let data: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                        // 仅"讨论（带分加）"节点（leaf_type=4 / topic_type=4）需要
                        // 额外上报 chapter/schedule，告诉服务端"我已完成这次讨论任务"。
                        // 失败不致命：本条仍按"评论成功"计；失败信息附在 schedule_error 里。
                        let mut schedule_error: Option<String> = None;
                        if do_schedule {
                            let referer = format!(
                                "https://www.xuetangx.com/learn/space/{sign}/{sign}/{classroom_id}/discussion/{leaf_id}"
                            );
                            match crate::article::mark_chapter_schedule(
                                &client,
                                classroom_id,
                                leaf_id,
                                sku,
                                &referer,
                            )
                            .await
                            {
                                Ok((s, b)) => {
                                    if !(200..300).contains(&s) {
                                        schedule_error =
                                            Some(format!("chapter/schedule HTTP {}: {}", s, b));
                                    }
                                }
                                Err(e) => schedule_error = Some(err_str(e)),
                            }
                        }
                        break json!({
                            "leaf_id": leaf_id,
                            "ok": true,
                            "data": data,
                            "schedule_error": schedule_error,
                        });
                    }
                }
                Err(e) => {
                    break json!({
                        "leaf_id": leaf_id,
                        "ok": false,
                        "error": err_str(e),
                    });
                }
            }
        };
        out.push(item.clone());
        // 把这条结果即时推给前端订阅，避免整批结束后才能看到结果列表。
        // item 自身就是 { leaf_id, ok, ... }，作为 extra 直接转发即可。
        emit_progress("item", idx, interval_ms, item);
        last_send = Some(std::time::Instant::now());
    }

    emit_progress("done", total, interval_ms, json!({}));
    Ok(out)
    // _permit 在此 drop，自动释放
}

/// 批量标记"图文"（leaf_type=3）节点为已学完。
///
/// 实现：对每个 leaf 调用 user_article_finish（预热同源上下文，与浏览器一致）
/// 然后 POST `/api/v1/lms/learn/chapter/schedule`。这正是新 HAR 中观察到的"完成图文"
/// 触发路径——调用之后 `course/schedule` 中该 leaf 的进度由 None 升到 1。
///
/// 与 `auto_comment_leaf` 一样通过 `task_semaphore` 占一个并发槽，并把进度通过
/// `article://progress` 事件上报给前端，方便实时显示。学堂在线对该接口没有评论接口
/// 那种严苛的滑动限速，但仍稳态间隔 1.5s/条，避免后续被加风控。
#[tauri::command]
pub async fn auto_article_leaf(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sku_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
    delay_ms: Option<u64>,
) -> Result<Vec<Value>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let _permit = acquire_task_slot(&state).await;

    const DEFAULT_INTERVAL_MS: u64 = 1500;
    const MIN_INTERVAL_MS: u64 = 500;
    let interval_ms = delay_ms.unwrap_or(DEFAULT_INTERVAL_MS).max(MIN_INTERVAL_MS);
    let total = leaf_ids.len();
    let mut out = Vec::with_capacity(total);
    let mut last_send: Option<std::time::Instant> = None;

    let emit_progress = {
        let app = app.clone();
        move |phase: &str, index: usize, extra: Value| {
            let payload = json!({
                "phase": phase,
                "index": index,
                "total": total,
                "interval_ms": interval_ms,
                "extra": extra,
            });
            let _ = app.emit("article://progress", payload);
        }
    };

    for (idx, leaf_id) in leaf_ids.into_iter().enumerate() {
        if let Some(last) = last_send {
            let elapsed = last.elapsed().as_millis() as u64;
            if elapsed < interval_ms {
                let wait = interval_ms - elapsed;
                emit_progress(
                    "throttle",
                    idx,
                    json!({ "wait_ms": wait, "leaf_id": leaf_id }),
                );
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
        }

        emit_progress("sending", idx, json!({ "leaf_id": leaf_id }));
        let item = match crate::article::mark_article_finish(
            &client,
            classroom_id,
            sku_id,
            &sign,
            leaf_id,
        )
        .await
        {
            Ok((status, body)) => {
                if (200..300).contains(&status) {
                    let data: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                    json!({ "leaf_id": leaf_id, "ok": true, "data": data })
                } else {
                    json!({
                        "leaf_id": leaf_id,
                        "ok": false,
                        "error": format!("HTTP {}: {}", status, body),
                    })
                }
            }
            Err(e) => json!({
                "leaf_id": leaf_id,
                "ok": false,
                "error": err_str(e),
            }),
        };

        out.push(item.clone());
        emit_progress("item", idx, item);
        last_send = Some(std::time::Instant::now());
    }

    emit_progress("done", total, json!({}));
    Ok(out)
}

#[tauri::command]
pub async fn list_exercise(
    app: AppHandle<Wry>,
    exercise_id: i64,
    sku_id: i64,
) -> Result<ExerciseList, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    exercise::fetch_exercise(&client, exercise_id, sku_id)
        .await
        .map_err(err_str)
}

#[tauri::command]
pub async fn list_exercise_with_captcha(
    app: AppHandle<Wry>,
    exercise_id: i64,
    sku_id: i64,
    referer: Option<String>,
    ticket: Option<String>,
    randstr: Option<String>,
) -> Result<ExerciseList, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    exercise::fetch_exercise_with_captcha(
        &client,
        exercise_id,
        sku_id,
        referer.as_deref(),
        ticket.as_deref(),
        randstr.as_deref(),
    )
    .await
    .map_err(|e| {
        if let Some(ch) = e.downcast_ref::<CaptchaChallenge>() {
            format!(
                "CAPTCHA_REQUIRED:{}:{}:{}",
                ch.captcha_appid, ch.exercise_id, ch.sku_id
            )
        } else {
            err_str(e)
        }
    })
}

#[tauri::command]
pub async fn probe_exercise_captcha(
    app: AppHandle<Wry>,
    exercise_id: i64,
    sku_id: i64,
    referer: Option<String>,
) -> Result<ExerciseProbe, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    exercise::probe_exercise_with_captcha(&client, exercise_id, sku_id, referer.as_deref())
        .await
        .map_err(err_str)
}

#[derive(Deserialize)]
pub struct SubmitProblemArgs {
    pub leaf_id: i64,
    pub classroom_id: i64,
    pub exercise_id: i64,
    pub problem_id: i64,
    pub sign: String,
    pub answer: Vec<String>,
    pub answers: Option<Value>,
}

#[tauri::command]
pub async fn submit_problem(
    app: AppHandle<Wry>,
    args: SubmitProblemArgs,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    exercise::submit_problem(
        &client,
        args.leaf_id,
        args.classroom_id,
        args.exercise_id,
        args.problem_id,
        &args.sign,
        args.answer,
        args.answers.unwrap_or(json!({})),
    )
    .await
    .map_err(err_str)
}

#[derive(Deserialize)]
pub struct AutoHomeworkArgs {
    pub leaf_id: i64,
    pub classroom_id: i64,
    pub sku_id: i64,
    pub exercise_id: i64,
    pub sign: String,
    pub ticket: Option<String>,
    pub randstr: Option<String>,
}

#[tauri::command]
pub async fn auto_homework_leaf(
    app: AppHandle<Wry>,
    args: AutoHomeworkArgs,
) -> Result<Vec<Value>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let ai = state.settings.read().ai.clone();
    let (use_local_bank, auto_harvest, submit_delay, wrong_max) = {
        let s = state.settings.read();
        (
            s.use_local_bank.unwrap_or(true),
            s.auto_harvest_bank.unwrap_or(true),
            // 从设置取每题提交节流的随机延迟范围；缺省回落到 SubmitDelay::defaults()。
            SubmitDelay::from_settings(s.submit_delay_min_ms, s.submit_delay_max_ms),
            // 控分：缺省 0（不答错）。
            s.wrong_answer_max_per_exercise.unwrap_or(0),
        )
    };
    let _permit = acquire_task_slot(&state).await;

    // 进度回调：把 exercise 层上报的阶段转发为 tauri 事件 `homework://progress`，
    // 让前端能在每个 leaf 的卡片下方实时展示"当前正在做的题 + 阶段"。
    // 闭包 move 了一份 `app` 句柄；leaf_id 也带在 payload 里，前端按 leaf_id 路由到对应分组。
    let app_for_cb = app.clone();
    let leaf_id_for_cb = args.leaf_id;
    let on_progress = move |phase: &str, info: Value| {
        let payload = json!({
            "leaf_id": leaf_id_for_cb,
            "phase": phase,
            "info": info,
        });
        let _ = app_for_cb.emit("homework://progress", payload);
    };

    let result = auto_run_exercise_with_captcha(
        &client,
        &ai,
        args.leaf_id,
        args.classroom_id,
        args.sku_id,
        args.exercise_id,
        &args.sign,
        args.ticket.as_deref(),
        args.randstr.as_deref(),
        Some(&state.bank),
        use_local_bank,
        auto_harvest,
        submit_delay,
        wrong_max,
        &on_progress,
    )
    .await
    .map_err(|e| {
        if let Some(ch) = e.downcast_ref::<CaptchaChallenge>() {
            format!(
                "CAPTCHA_REQUIRED:{}:{}:{}",
                ch.captcha_appid, ch.exercise_id, ch.sku_id
            )
        } else {
            err_str(e)
        }
    })?;

    // 自动作业过程中若把答案入库了，立即持久化一次。失败仅记日志。
    if auto_harvest {
        if let Err(e) = state.persist_bank(&app) {
            log::warn!("auto_homework 持久化题库失败: {e}");
        }
    }
    Ok(result)
    // _permit drop 时自动释放槽位（即便 ? 提前返回）
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, AppState>) -> AppSettings {
    state.settings.read().clone()
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    let new_concurrency = settings.task_concurrency;
    *state.settings.write() = settings.clone();
    state.persist(&app).map_err(err_str)?;
    // 调整 Semaphore，让设置实时生效（已经在跑的任务不受影响）
    state.apply_task_concurrency(new_concurrency);
    // 通知前端：设置已变更（视频页可同步倍速等偏好）
    let _ = app.emit("settings://updated", &settings);
    Ok(())
}

#[tauri::command]
pub async fn test_ai(settings: AiSettings) -> Result<String, String> {
    test_settings(&settings).await.map_err(err_str)
}

#[tauri::command]
pub async fn debug_user_courses(app: AppHandle<Wry>) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let mut probes = Vec::new();
    // 探测 check_is_l
    let (s1, b1) = client
        .get_raw("/api/v1/u/login/check_is_l/", None)
        .await
        .map_err(err_str)?;
    probes.push(json!({
        "name": "check_is_l",
        "status": s1,
        "body": b1.chars().take(800).collect::<String>(),
    }));
    let (s2, b2) = client
        .get_raw("/api/v1/u/user/basic_profile/", None)
        .await
        .map_err(err_str)?;
    probes.push(json!({
        "name": "basic_profile",
        "status": s2,
        "body": b2.chars().take(800).collect::<String>(),
    }));
    for st in [1, 2, 0] {
        let path = format!("/api/v1/lms/user/user-courses/?status={st}&page=1");
        let (s, b) = client
            .get_raw(&path, Some("https://www.xuetangx.com/my-courses/current"))
            .await
            .map_err(err_str)?;
        probes.push(json!({
            "name": format!("user-courses?status={st}"),
            "status": s,
            "body": b.chars().take(1500).collect::<String>(),
        }));
    }
    // Cookie 信息
    let cookies: Vec<String> = client
        .cookies
        .lock()
        .unwrap()
        .iter_any()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect();
    Ok(json!({ "probes": probes, "cookies": cookies }))
}

#[derive(Deserialize)]
pub struct DebugExerciseProbeArgs {
    pub leaf_id: i64,
    pub classroom_id: i64,
    pub sku_id: i64,
    pub exercise_id: i64,
    pub sign: String,
}

#[tauri::command]
pub async fn debug_exercise_probe(
    app: AppHandle<Wry>,
    args: DebugExerciseProbeArgs,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let referer = format!(
        "https://www.xuetangx.com/learn/space/{}/{}/{}/exercise/{}",
        args.sign, args.sign, args.classroom_id, args.leaf_id
    );

    let mut probes = Vec::new();

    let leaf_path = format!(
        "/api/v1/lms/learn/leaf_info/{}/{}/?sign={}",
        args.classroom_id, args.leaf_id, args.sign
    );
    let (s_leaf, b_leaf) = client
        .get_raw_same_origin(&leaf_path, &referer)
        .await
        .map_err(err_str)?;
    probes.push(json!({
        "name": "leaf_info",
        "status": s_leaf,
        "body": b_leaf.chars().take(2000).collect::<String>(),
    }));

    let eval_path = format!(
        "/api/v1/lms/learn/get_evaluation_detail/?sign={}&cid={}",
        args.sign, args.classroom_id
    );
    let (s_eval, b_eval) = client
        .get_raw_same_origin(&eval_path, &referer)
        .await
        .map_err(err_str)?;
    probes.push(json!({
        "name": "get_evaluation_detail",
        "status": s_eval,
        "body": b_eval.chars().take(2000).collect::<String>(),
    }));

    let ex_path = format!(
        "/api/v1/lms/exercise/get_exercise_list/{}/{}/",
        args.exercise_id, args.sku_id
    );
    let (s_ex, b_ex) = client
        .get_raw_same_origin(&ex_path, &referer)
        .await
        .map_err(err_str)?;
    let parsed: Value = serde_json::from_str(&b_ex).unwrap_or(Value::Null);
    let data_keys: Vec<String> = parsed
        .get("data")
        .and_then(|d| d.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    let problems_len = parsed
        .get("data")
        .and_then(|d| d.get("problems"))
        .and_then(|p| p.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    probes.push(json!({
        "name": "get_exercise_list",
        "status": s_ex,
        "success": parsed.get("success").cloned(),
        "msg": parsed.get("msg").cloned(),
        "data_keys": data_keys,
        "problems_len": problems_len,
        "body": b_ex.chars().take(2000).collect::<String>(),
    }));

    let cookies: Vec<String> = client
        .cookies
        .lock()
        .unwrap()
        .iter_any()
        .map(|c| format!("{}@{}{}", c.name(), c.domain().unwrap_or(""), c.path().unwrap_or("")))
        .collect();

    Ok(json!({
        "referer": referer,
        "probes": probes,
        "cookies": cookies,
    }))
}

// ============================================================================
// 本地题库（local answer bank）相关命令
// ----------------------------------------------------------------------------
// 数据来源：仅接受学堂在线 `/get_exercise_list` 在小题已批改后下发的 `answer` 字段。
// AI 答案不入库（设计上即避免污染）。
// 所有操作仅读写本地 `xuetang-helper.bank.json`，不发送任何外部数据。
// ============================================================================

#[derive(Deserialize)]
pub struct HarvestArgs {
    pub leaf_id: i64,
    pub classroom_id: i64,
    pub sku_id: i64,
    pub exercise_id: i64,
    pub sign: String,
}

#[derive(serde::Serialize)]
pub struct HarvestOutcome {
    pub leaf_id: i64,
    pub total_problems: usize,
    pub submitted_problems: usize,
    pub harvested: usize,
}

/// 单节点收录：只发起一次 GET `/get_exercise_list`，把响应里已批改且带答案的题入库。
/// 不会触发任何 POST / 提交动作 —— 完全是被动观察。
#[tauri::command]
pub async fn harvest_exercise_answers(
    app: AppHandle<Wry>,
    args: HarvestArgs,
) -> Result<HarvestOutcome, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let referer = format!(
        "https://www.xuetangx.com/learn/space/{}/{}/{}/exercise/{}",
        args.sign, args.sign, args.classroom_id, args.leaf_id
    );
    // 同浏览器流程：先 warm 上下文，避免空 `data: {}` 响应
    exercise::warm_exercise_context(
        &client,
        args.leaf_id,
        args.classroom_id,
        args.sku_id,
        &args.sign,
        &referer,
    )
    .await;
    let list = exercise::fetch_exercise_with_referer(
        &client,
        args.exercise_id,
        args.sku_id,
        Some(&referer),
    )
    .await
    .map_err(err_str)?;
    let total_problems = list.problems.len();
    let mut submitted = 0usize;
    let mut harvested = 0usize;
    {
        let mut guard = state.bank.write();
        for p in &list.problems {
            if p.submitted {
                submitted += 1;
            }
            if guard.upsert_from_problem(p) {
                harvested += 1;
            }
        }
    }
    if harvested > 0 {
        if let Err(e) = state.persist_bank(&app) {
            log::warn!("harvest 持久化题库失败: {e}");
        }
    }
    Ok(HarvestOutcome {
        leaf_id: args.leaf_id,
        total_problems,
        submitted_problems: submitted,
        harvested,
    })
}

#[derive(Deserialize)]
pub struct BatchHarvestArgs {
    pub classroom_id: i64,
    pub sku_id: i64,
    pub sign: String,
    /// 列表里每项是一个待收录节点：leaf_id + exercise_id
    pub leaves: Vec<HarvestLeafSpec>,
}

#[derive(Deserialize)]
pub struct HarvestLeafSpec {
    pub leaf_id: i64,
    pub exercise_id: i64,
}

/// 批量收录：按 1 req/s 节奏顺序处理每个节点。每个节点结束发 `bank://progress` 事件
/// （phase = "item" | "done"），便于前端实时显示。
#[tauri::command]
pub async fn batch_harvest_course_answers(
    app: AppHandle<Wry>,
    args: BatchHarvestArgs,
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let _permit = acquire_task_slot(&state).await;

    // 限速：≤ 1 req/s，符合 CLAUDE.md 全局约束。
    const INTERVAL_MS: u64 = 1100;
    let total = args.leaves.len();
    let mut last_send: Option<std::time::Instant> = None;
    let mut total_harvested = 0usize;
    let mut total_submitted = 0usize;
    let mut items: Vec<Value> = Vec::with_capacity(total);

    let emit_progress = {
        let app = app.clone();
        move |phase: &str, index: usize, extra: Value| {
            let _ = app.emit(
                "bank://progress",
                json!({
                    "phase": phase,
                    "index": index,
                    "total": total,
                    "interval_ms": INTERVAL_MS,
                    "extra": extra,
                }),
            );
        }
    };

    for (idx, spec) in args.leaves.iter().enumerate() {
        if let Some(last) = last_send {
            let elapsed = last.elapsed().as_millis() as u64;
            if elapsed < INTERVAL_MS {
                let wait = INTERVAL_MS - elapsed;
                emit_progress(
                    "throttle",
                    idx,
                    json!({ "wait_ms": wait, "leaf_id": spec.leaf_id }),
                );
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
            }
        }
        emit_progress(
            "fetching",
            idx,
            json!({ "leaf_id": spec.leaf_id, "exercise_id": spec.exercise_id }),
        );
        let referer = format!(
            "https://www.xuetangx.com/learn/space/{}/{}/{}/exercise/{}",
            args.sign, args.sign, args.classroom_id, spec.leaf_id
        );
        exercise::warm_exercise_context(
            &client,
            spec.leaf_id,
            args.classroom_id,
            args.sku_id,
            &args.sign,
            &referer,
        )
        .await;
        let item = match exercise::fetch_exercise_with_referer(
            &client,
            spec.exercise_id,
            args.sku_id,
            Some(&referer),
        )
        .await
        {
            Ok(list) => {
                let mut submitted = 0usize;
                let mut harvested = 0usize;
                {
                    let mut guard = state.bank.write();
                    for p in &list.problems {
                        if p.submitted {
                            submitted += 1;
                        }
                        if guard.upsert_from_problem(p) {
                            harvested += 1;
                        }
                    }
                }
                total_submitted += submitted;
                total_harvested += harvested;
                json!({
                    "leaf_id": spec.leaf_id,
                    "ok": true,
                    "total": list.problems.len(),
                    "submitted": submitted,
                    "harvested": harvested,
                })
            }
            Err(e) => json!({
                "leaf_id": spec.leaf_id,
                "ok": false,
                "error": err_str(e),
            }),
        };
        emit_progress("item", idx, item.clone());
        items.push(item);
        last_send = Some(std::time::Instant::now());
    }

    if total_harvested > 0 {
        if let Err(e) = state.persist_bank(&app) {
            log::warn!("batch_harvest 持久化题库失败: {e}");
        }
    }
    emit_progress(
        "done",
        total,
        json!({
            "total_submitted": total_submitted,
            "total_harvested": total_harvested,
        }),
    );

    Ok(json!({
        "total": total,
        "total_submitted": total_submitted,
        "total_harvested": total_harvested,
        "items": items,
    }))
}

#[derive(Deserialize, Default)]
pub struct BankListArgs {
    pub keyword: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[tauri::command]
pub fn bank_list(
    state: tauri::State<'_, AppState>,
    args: Option<BankListArgs>,
) -> Vec<BankEntry> {
    let a = args.unwrap_or_default();
    let kw = a.keyword.as_deref();
    let offset = a.offset.unwrap_or(0);
    let limit = a.limit.unwrap_or(200).clamp(1, 1000);
    state.bank.read().list(kw, offset, limit)
}

#[tauri::command]
pub fn bank_get(state: tauri::State<'_, AppState>, problem_id: i64) -> Option<BankEntry> {
    state.bank.read().get(problem_id)
}

#[tauri::command]
pub fn bank_delete(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    problem_id: i64,
) -> Result<bool, String> {
    let removed = state.bank.write().delete(problem_id);
    if removed {
        state.persist_bank(&app).map_err(err_str)?;
    }
    Ok(removed)
}

#[tauri::command]
pub fn bank_clear(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let n = state.bank.read().len();
    state.bank.write().clear();
    state.persist_bank(&app).map_err(err_str)?;
    Ok(n)
}

#[tauri::command]
pub fn bank_export(state: tauri::State<'_, AppState>) -> Vec<BankEntry> {
    state.bank.read().export_all()
}

#[derive(Deserialize)]
pub struct BankImportArgs {
    pub entries: Vec<BankEntry>,
}

#[derive(serde::Serialize)]
pub struct BankImportOutcome {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    pub total_after: usize,
}

#[tauri::command]
pub fn bank_import(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    args: BankImportArgs,
) -> Result<BankImportOutcome, String> {
    let (added, updated, skipped) = state.bank.write().import(args.entries);
    let total_after = state.bank.read().len();
    if added + updated > 0 {
        state.persist_bank(&app).map_err(err_str)?;
    }
    Ok(BankImportOutcome {
        added,
        updated,
        skipped,
        total_after,
    })
}

#[tauri::command]
pub fn bank_stats(state: tauri::State<'_, AppState>) -> BankStats {
    state.bank.read().stats()
}

