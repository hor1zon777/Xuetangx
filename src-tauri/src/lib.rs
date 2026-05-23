mod accounts;
mod ai;
mod article;
mod client;
mod commands;
mod courses;
mod exercise;
mod forum;
mod login;
mod state;
mod video;

use state::AppState;
use tauri::Manager;

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init()
        .ok();

    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::switch_account,
            commands::remove_account,
            commands::current_account,
            commands::check_login,
            commands::start_login,
            commands::cancel_login,
            commands::list_courses,
            commands::list_chapters,
            commands::leaf_info,
            commands::course_schedule,
            commands::batch_exercise_ids,
            commands::batch_exercise_kinds,
            commands::start_video_task,
            commands::stop_video_task,
            commands::list_video_tasks,
            commands::send_comment,
            commands::list_topic_comments,
            commands::auto_comment_leaf,
            commands::auto_article_leaf,
            commands::batch_my_comment_status,
            commands::list_exercise,
            commands::list_exercise_with_captcha,
            commands::probe_exercise_captcha,
            commands::submit_problem,
            commands::auto_homework_leaf,
            commands::get_settings,
            commands::save_settings,
            commands::test_ai,
            commands::debug_user_courses,
            commands::debug_exercise_probe,
        ])
        .setup(|app| {
            let state: tauri::State<AppState> = app.state();
            state.load_persisted(app.handle());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 失败");
}
