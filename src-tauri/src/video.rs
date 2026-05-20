use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Wry};
use tokio::sync::Notify;

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
) {
    let result = run_video_task_inner(&app, &handle, &client).await;
    let mut status = handle.status.lock();
    status.finished = true;
    if let Err(e) = &result {
        status.error = Some(format!("{e}"));
        log::error!("视频任务 {} 失败: {e:?}", handle.task_id);
    }
    let snapshot = status.clone();
    drop(status);
    let _ = app.emit("video://done", &snapshot);
}

async fn run_video_task_inner(
    app: &AppHandle<Wry>,
    handle: &VideoTaskHandle,
    client: &XtClient,
) -> Result<()> {
    let p = handle.params.clone();
    if p.duration <= 0.0 {
        return Err(anyhow!("视频时长为 0，拒绝启动（可能是非视频节点或未发布）"));
    }
    if p.video_ccid.trim().is_empty() {
        return Err(anyhow!("视频 ccid 为空，拒绝启动"));
    }
    let pg = rand_pg(p.leaf_id);
    let interval = std::time::Duration::from_millis(p.interval_ms.unwrap_or(5000));
    let speed = p.speed.unwrap_or(1.0).max(0.5).min(2.0);
    let start = p.start_position.unwrap_or(0.0).max(0.0).min(p.duration);

    let mut sq: u64 = 0;
    let mut cp = start;
    let fp = if start > 0.0 { start } else { 0.0 };
    let tp = fp;

    // 1) 启动事件序列
    let bootstrap_events = vec![
        build_event("loadstart", 0.0, 0.0, 0.0, speed, {
            sq += 1;
            sq
        }, &pg, &p),
        build_event("loadeddata", cp, fp, tp, speed, {
            sq += 1;
            sq
        }, &pg, &p),
        build_event("play", cp, fp, tp, speed, {
            sq += 1;
            sq
        }, &pg, &p),
        build_event("playing", cp, fp, tp, speed, {
            sq += 1;
            sq
        }, &pg, &p),
    ];
    send_heartbeat(client, bootstrap_events).await?;

    // 2) 心跳循环
    loop {
        tokio::select! {
            _ = handle.cancel.notified() => {
                let pause_evt = build_event("pause", cp, fp, tp, speed, { sq+=1; sq }, &pg, &p);
                let _ = send_heartbeat(client, vec![pause_evt]).await;
                return Ok(());
            }
            _ = tokio::time::sleep(interval) => {
                cp += (interval.as_secs_f64()) * (speed as f64);
                if cp > p.duration {
                    cp = p.duration;
                }
                sq += 1;
                let evt = build_event("heartbeat", cp, fp, tp, speed, sq, &pg, &p);
                if let Err(e) = send_heartbeat(client, vec![evt]).await {
                    handle.status.lock().error = Some(format!("{e}"));
                    let _ = app.emit("video://error", json!({"task_id": handle.task_id, "message": format!("{e}")}));
                }
                {
                    let mut s = handle.status.lock();
                    s.current_pos = cp;
                }
                let _ = app.emit("video://progress", &*handle.status.lock());
                if cp >= p.duration {
                    let end_evt = build_event("ended", cp, fp, tp, speed, { sq+=1; sq }, &pg, &p);
                    let _ = send_heartbeat(client, vec![end_evt]).await;
                    return Ok(());
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
