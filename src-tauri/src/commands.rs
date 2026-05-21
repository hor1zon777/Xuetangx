use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Wry};
use uuid::Uuid;

use crate::accounts::Account;
use crate::ai::test_settings;
use crate::courses::{self, CourseSummary, LeafNode};
use crate::exercise::{self, auto_run_exercise, ExerciseList};
use crate::forum;
use crate::login;
use crate::state::{AiSettings, AppSettings, AppState};
use crate::video::{self, VideoTaskHandle, VideoTaskParams, VideoTaskStatus};

fn err_str<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}

/// 尝试获取一个任务槽位。若已满则等待。
async fn acquire_task_slot(state: &AppState) {
    loop {
        {
            let _lock = state.start_task_lock.lock();
            let limit = state.settings.read().task_concurrency;
            let mut count = state.running_task_count.lock();
            match limit {
                Some(max) if *count >= max => { /* 槽满，等待 */ }
                _ => {
                    *count += 1;
                    return;
                }
            }
        }
        state.task_slot_notify.notified().await;
    }
}

/// 释放一个任务槽位，唤醒等待者。
fn release_task_slot(state: &AppState) {
    *state.running_task_count.lock() = state.running_task_count.lock().saturating_sub(1);
    state.task_slot_notify.notify_waiters();
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
    state.switch(&app, user_id).map_err(err_str)
}

#[tauri::command]
pub fn remove_account(
    app: AppHandle<Wry>,
    state: tauri::State<'_, AppState>,
    user_id: i64,
) -> Result<(), String> {
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
    sku_id: i64,
    items: Vec<(i64, i64)>,
) -> Result<std::collections::HashMap<i64, std::collections::HashMap<String, i64>>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    courses::batch_exercise_kinds(client, sku_id, items)
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

    // 临界区：并发检查 + 计数变更必须原子化
    let should_spawn;
    {
        let _lock = state.start_task_lock.lock();
        state
            .video_tasks
            .write()
            .insert(task_id.clone(), handle.clone());

        let limit = state.settings.read().task_concurrency;
        let mut count = state.running_task_count.lock();
        should_spawn = match limit {
            Some(max) => *count < max,
            None => true,
        };
        if should_spawn {
            *count += 1;
        }
        drop(count);
        if !should_spawn {
            handle.status.lock().queued = true;
            state
                .pending_video_tasks
                .write()
                .push_back(handle.clone());
        }
    } // 释放锁

    let result_status = handle.status.lock().clone();

    if should_spawn {
        let app_clone = app.clone();
        tokio::spawn(async move {
            video::run_video_task(app_clone, handle, client).await;
        });
    }

    Ok(result_status)
}

#[tauri::command]
pub fn stop_video_task(state: tauri::State<'_, AppState>, task_id: String) -> Result<(), String> {
    // 检查是否在等待队列中，是则直接从队列移除并标记取消
    {
        let mut queue = state.pending_video_tasks.write();
        if let Some(pos) = queue.iter().position(|h| h.task_id == task_id) {
            queue.remove(pos);
            // 找到对应的 handle，标记为取消完成
            if let Some(h) = state.video_tasks.read().get(&task_id).cloned() {
                let mut s = h.status.lock();
                s.finished = true;
                s.cancelled = true;
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
) -> Result<Value, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let topic = forum::fetch_unit_discussion(&client, &sign, classroom_id, leaf_id)
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

#[tauri::command]
pub async fn auto_comment_leaf(
    app: AppHandle<Wry>,
    classroom_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
    text: String,
    delay_ms: Option<u64>,
) -> Result<Vec<Value>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    acquire_task_slot(&state).await;

    // 风控保护：批量评论必须有间隔，避免被服务端限流封号。
    const MIN_DELAY_MS: u64 = 1000;
    let effective_delay = delay_ms.unwrap_or(1500).max(MIN_DELAY_MS);
    let mut out = Vec::new();
    let total = leaf_ids.len();
    for (idx, leaf_id) in leaf_ids.into_iter().enumerate() {
        match forum::fetch_unit_discussion(&client, &sign, classroom_id, leaf_id).await {
            Ok(topic) => {
                match forum::post_comment(&client, classroom_id, leaf_id, topic.topic_id, topic.topic_owner_id, &text)
                    .await
                {
                    Ok(v) => out.push(json!({ "leaf_id": leaf_id, "ok": true, "data": v })),
                    Err(e) => out.push(json!({ "leaf_id": leaf_id, "ok": false, "error": err_str(e) })),
                }
            }
            Err(e) => out.push(json!({ "leaf_id": leaf_id, "ok": false, "error": err_str(e) })),
        }
        // 最后一条不再 sleep
        if idx + 1 < total {
            tokio::time::sleep(std::time::Duration::from_millis(effective_delay)).await;
        }
    }

    release_task_slot(&state);
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
}

#[tauri::command]
pub async fn auto_homework_leaf(
    app: AppHandle<Wry>,
    args: AutoHomeworkArgs,
) -> Result<Vec<Value>, String> {
    let state: tauri::State<AppState> = app.state();
    let client = state.current_client().map_err(err_str)?;
    let ai = state.settings.read().ai.clone();
    acquire_task_slot(&state).await;
    let result = auto_run_exercise(
        &client,
        &ai,
        args.leaf_id,
        args.classroom_id,
        args.sku_id,
        args.exercise_id,
        &args.sign,
    )
    .await;
    release_task_slot(&state);
    result.map_err(err_str)
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
    *state.settings.write() = settings.clone();
    state.persist(&app).map_err(err_str)?;
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
