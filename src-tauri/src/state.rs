use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Wry};
use tauri_plugin_store::StoreExt;

use crate::accounts::Account;

const STORE_FILE: &str = "xuetang-helper.store.json";
const KEY_ACCOUNTS: &str = "accounts";
const KEY_CURRENT: &str = "current_user_id";
const KEY_SETTINGS: &str = "settings";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AiSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub system_prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub ai: AiSettings,
    pub heartbeat_interval_ms: Option<u64>,
    pub video_speed: Option<f32>,
    pub auto_comment_default: Option<String>,
}

pub struct AppState {
    pub accounts: RwLock<HashMap<i64, Account>>,
    pub current_user_id: RwLock<Option<i64>>,
    pub settings: RwLock<AppSettings>,
    pub video_tasks: RwLock<HashMap<String, Arc<crate::video::VideoTaskHandle>>>,
    pub login_session: RwLock<Option<Arc<crate::login::LoginSession>>>,
    pub clients: RwLock<HashMap<i64, Arc<crate::client::XtClient>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            accounts: RwLock::new(HashMap::new()),
            current_user_id: RwLock::new(None),
            settings: RwLock::new(AppSettings::default()),
            video_tasks: RwLock::new(HashMap::new()),
            login_session: RwLock::new(None),
            clients: RwLock::new(HashMap::new()),
        }
    }

    pub fn load_persisted(&self, app: &AppHandle<Wry>) {
        let Ok(store) = app.store(STORE_FILE) else {
            return;
        };
        if let Some(v) = store.get(KEY_ACCOUNTS) {
            if let Ok(list) = serde_json::from_value::<Vec<Account>>(v) {
                let mut map = self.accounts.write();
                for a in list {
                    map.insert(a.user_id, a);
                }
            }
        }
        if let Some(v) = store.get(KEY_CURRENT) {
            if let Ok(id) = serde_json::from_value::<i64>(v) {
                *self.current_user_id.write() = Some(id);
            }
        }
        if let Some(v) = store.get(KEY_SETTINGS) {
            if let Ok(s) = serde_json::from_value::<AppSettings>(v) {
                *self.settings.write() = s;
            }
        }
    }

    pub fn persist(&self, app: &AppHandle<Wry>) -> anyhow::Result<()> {
        let store = app.store(STORE_FILE)?;
        let list: Vec<Account> = self.accounts.read().values().cloned().collect();
        store.set(KEY_ACCOUNTS, serde_json::to_value(&list)?);
        store.set(
            KEY_CURRENT,
            serde_json::to_value(*self.current_user_id.read())?,
        );
        store.set(KEY_SETTINGS, serde_json::to_value(&*self.settings.read())?);
        store.save()?;
        Ok(())
    }

    pub fn upsert_account(&self, app: &AppHandle<Wry>, account: Account) -> anyhow::Result<()> {
        let uid = account.user_id;
        {
            let mut map = self.accounts.write();
            map.insert(uid, account);
        }
        *self.current_user_id.write() = Some(uid);
        // 切账号时丢弃旧的客户端
        self.clients.write().remove(&uid);
        self.persist(app)
    }

    pub fn remove_account(&self, app: &AppHandle<Wry>, uid: i64) -> anyhow::Result<()> {
        self.accounts.write().remove(&uid);
        self.clients.write().remove(&uid);
        let mut cur = self.current_user_id.write();
        if *cur == Some(uid) {
            *cur = self.accounts.read().keys().next().copied();
        }
        drop(cur);
        self.persist(app)
    }

    pub fn current_account(&self) -> Option<Account> {
        let cur = *self.current_user_id.read();
        cur.and_then(|id| self.accounts.read().get(&id).cloned())
    }

    pub fn switch(&self, app: &AppHandle<Wry>, uid: i64) -> anyhow::Result<()> {
        if !self.accounts.read().contains_key(&uid) {
            anyhow::bail!("账号不存在");
        }
        *self.current_user_id.write() = Some(uid);
        self.persist(app)
    }

    pub fn client_for(&self, uid: i64) -> anyhow::Result<Arc<crate::client::XtClient>> {
        {
            let cache = self.clients.read();
            if let Some(c) = cache.get(&uid) {
                return Ok(c.clone());
            }
        }
        let account = self
            .accounts
            .read()
            .get(&uid)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("账号不存在"))?;
        let client = Arc::new(crate::client::XtClient::from_account(&account)?);
        // 补齐 JS 端 setCookie 写入的 cookie（k=用户id、mode_type=normal）
        // 兼容旧版本登录后未持久化这些字段的账号。
        let has_k = account.cookies.iter().any(|c| c.name == "k");
        if !has_k {
            client.set_cookie("k", &uid.to_string());
        }
        let has_mode = account.cookies.iter().any(|c| c.name == "mode_type");
        if !has_mode {
            client.set_cookie("mode_type", "normal");
        }
        self.clients.write().insert(uid, client.clone());
        Ok(client)
    }

    pub fn current_client(&self) -> anyhow::Result<Arc<crate::client::XtClient>> {
        let uid = self
            .current_user_id
            .read()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("当前未选择账号"))?;
        self.client_for(uid)
    }

    pub fn save_account_cookies(&self, app: &AppHandle<Wry>, uid: i64) -> anyhow::Result<()> {
        let client = self.client_for(uid)?;
        let cookies = client.export_cookies();
        let mut accounts = self.accounts.write();
        if let Some(acc) = accounts.get_mut(&uid) {
            acc.cookies = cookies;
        }
        drop(accounts);
        self.persist(app)
    }
}
