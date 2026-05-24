use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::accounts::Account;
use crate::client::XtClient;
use crate::state::AppState;

const WSS_URL: &str = "wss://www.xuetangx.com/wsapp/";
/// 登录流程整体超时：用户长时间不扫码或服务端不响应时强制收尾。
const LOGIN_TOTAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// 单条消息等待超时：用于触发 ping 维持连接活跃。
const LOGIN_READ_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Serialize)]
pub struct QrPayload {
    pub ticket: String,
    pub loginid: i64,
    pub expire_seconds: i64,
}

pub struct LoginSession {
    pub cancel: Arc<Notify>,
}

#[derive(Deserialize, Debug)]
struct WsRequestLoginResp {
    op: String,
    #[serde(default)]
    loginid: Option<i64>,
    #[serde(default)]
    ticket: Option<String>,
    #[serde(default)]
    expire_seconds: Option<i64>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    is_new_user: Option<bool>,
    #[serde(default)]
    #[serde(rename = "UserID")]
    user_id: Option<i64>,
}

pub async fn start_login_flow(app: AppHandle<Wry>) -> Result<()> {
    let state: tauri::State<AppState> = app.state();
    // 若已有进行中登录，取消之
    let prev_session = state.login_session.read().clone();
    if let Some(prev) = prev_session {
        prev.cancel.notify_waiters();
    }
    let cancel = Arc::new(Notify::new());
    let session = Arc::new(LoginSession {
        cancel: cancel.clone(),
    });
    *state.login_session.write() = Some(session.clone());

    let app_clone = app.clone();
    tokio::spawn(async move {
        if let Err(e) = run_session(app_clone.clone(), session).await {
            log::error!("登录流程错误: {e:?}");
            let _ = app_clone.emit(
                "login://error",
                json!({"message": format!("登录流程错误: {e}")}),
            );
        }
    });
    Ok(())
}

pub fn cancel_login(app: &AppHandle<Wry>) {
    let state: tauri::State<AppState> = app.state();
    let session = state.login_session.read().clone();
    if let Some(s) = session {
        s.cancel.notify_waiters();
    }
}

async fn run_session(app: AppHandle<Wry>, session: Arc<LoginSession>) -> Result<()> {
    // 先建一个全新的 XtClient 用于本次登录，登录成功后转为账号
    let temp_client = Arc::new(XtClient::empty()?);
    temp_client.warm_up().await.ok();

    let (ws_stream, _) = connect_async(WSS_URL).await?;
    let (mut writer, mut reader) = ws_stream.split();
    let req = json!({
        "op":"requestlogin",
        "role":"web",
        "version":"1.4",
        "purpose":"login",
        "xtbz":"xt",
        "x-client":"web"
    });
    writer.send(Message::Text(req.to_string())).await?;

    // 登录主循环：总超时 + 单条等待超时 + cancel 三路 select。
    // - 总超时：避免用户离开后任务永久占资源（默认 5 min）。
    // - 读超时：每 LOGIN_READ_TIMEOUT 秒触发一次 ping，维持服务端连接活性。
    // - cancel：用户主动取消登录。
    let deadline = Instant::now() + LOGIN_TOTAL_TIMEOUT;
    let token: String;
    loop {
        if Instant::now() >= deadline {
            let _ = writer.send(Message::Close(None)).await;
            let _ = app.emit("login://timeout", json!({}));
            return Err(anyhow!("登录超时（{}s 未完成扫码）", LOGIN_TOTAL_TIMEOUT.as_secs()));
        }
        let next_msg = tokio::time::timeout(LOGIN_READ_TIMEOUT, reader.next());
        tokio::select! {
            _ = session.cancel.notified() => {
                let _ = writer.send(Message::Close(None)).await;
                let _ = app.emit("login://cancelled", json!({}));
                return Ok(());
            }
            res = next_msg => {
                let Ok(opt_msg) = res else {
                    // 读超时：发一个 ping，让服务端 / 中间盒子知道连接还活着
                    if let Err(e) = writer.send(Message::Ping(Vec::new())).await {
                        return Err(anyhow!("WebSocket ping 失败: {e}"));
                    }
                    continue;
                };
                let Some(msg) = opt_msg else { return Err(anyhow!("WebSocket 已关闭")) };
                let msg = msg?;
                match msg {
                    Message::Text(t) => {
                        let v: WsRequestLoginResp = match serde_json::from_str(&t) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        match v.op.as_str() {
                            "requestlogin" => {
                                if let (Some(loginid), Some(ticket), Some(exp)) = (v.loginid, v.ticket.clone(), v.expire_seconds) {
                                    let payload = QrPayload { ticket: ticket.clone(), loginid, expire_seconds: exp };
                                    let _ = app.emit("login://qr", &payload);
                                }
                            }
                            "loginsuccess" => {
                                token = v.token.ok_or_else(|| anyhow!("loginsuccess 未携带 token"))?;
                                let _ = app.emit("login://scanned", json!({"user_id": v.user_id}));
                                break;
                            }
                            _ => {}
                        }
                    }
                    Message::Ping(payload) => {
                        let _ = writer.send(Message::Pong(payload)).await;
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => return Err(anyhow!("WebSocket 已关闭")),
                    _ => {}
                }
            }
        }
    }
    let _ = writer.send(Message::Close(None)).await;

    // 用 token 调用 login/wx 写 sessionid
    let body = json!({
        "s_s": token,
        "preset_properties": {
            "$timezone_offset": -480,
            "$screen_height": 943,
            "$screen_width": 1676,
            "$lib": "js",
            "$lib_version": "1.19.14",
            "$latest_traffic_source_type": "直接流量",
            "$latest_search_keyword": "未取到值_直接打开",
            "$latest_referrer": "",
            "$is_first_day": true,
            "$referrer": "",
            "$referrer_host": "",
            "$url": "https://www.xuetangx.com/",
            "$url_path": "/",
            "$title": "学堂在线 - 精品在线课程学习平台"
        },
        "page_name": "首页"
    });
    let resp = temp_client.post_json("/api/v1/u/login/wx/", &body).await?;
    if !resp.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(anyhow!("login/wx 返回失败: {resp}"));
    }

    // 拿 profile
    let profile = temp_client.get_json("/api/v1/u/user/basic_profile/").await?;
    let d = profile
        .get("data")
        .cloned()
        .ok_or_else(|| anyhow!("profile 缺 data"))?;
    let uid = d
        .get("user_id")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            d.get("id").and_then(|v| v.as_i64())
        })
        .ok_or_else(|| anyhow!("profile 缺 user_id"))?;
    let nickname = d
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| d.get("nickname").and_then(|v| v.as_str()))
        .unwrap_or("学堂用户")
        .to_string();
    let avatar = d
        .get("avatar")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // 写入浏览器 JS 在登录后会手动设置的 cookie：k=<user_id>、mode_type=normal
    // 不带这些 cookie，部分 LMS 接口会过滤掉数据（例如 user-courses 返回空）。
    temp_client.set_cookie("k", &uid.to_string());
    temp_client.set_cookie("mode_type", "normal");
    temp_client.set_cookie("login_type", "WX");
    temp_client.set_cookie("provider", "xuetang");
    temp_client.set_cookie("django_language", "zh-cn");

    let account = Account {
        user_id: uid,
        nickname,
        avatar,
        login_type: Some("WX".into()),
        login_time: chrono::Utc::now().timestamp(),
        cookies: temp_client.export_cookies(),
    };

    // 注入到 state、缓存客户端
    let state: tauri::State<AppState> = app.state();
    state.upsert_account(&app, account.clone())?;
    state
        .clients
        .write()
        .insert(uid, temp_client);

    let _ = app.emit("login://success", json!({"user_id": uid, "nickname": account.nickname}));
    *state.login_session.write() = None;
    Ok(())
}

pub async fn check_is_login(client: &XtClient) -> Result<bool> {
    let v = client.get_json("/api/v1/u/login/check_is_l/").await?;
    Ok(v.get("data")
        .and_then(|d| d.get("is_login"))
        .and_then(|x| x.as_bool())
        .unwrap_or(false))
}
