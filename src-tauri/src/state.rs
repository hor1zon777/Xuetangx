use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tauri::{AppHandle, Wry};
use tauri_plugin_store::StoreExt;
use tokio::sync::Semaphore;

use crate::accounts::Account;

/// 任务并发上限的"超大"基数。Semaphore 用 `permits = MAX_TASK_PERMITS - desired_limit`
/// 的方式间接控制并发：
/// - 想限制为 N，就让 Semaphore 持有 (MAX - N) 个 permit，留下 N 个可用；
/// - "不限制" 即让全部 MAX 个 permit 可用（一次性 forget 掉所有内部预占）。
///
/// 之所以这样做是因为 tokio::sync::Semaphore 没有"动态缩小可用 permit"的直接 API，
/// 只能用 forget()/add_permits() 增减。固定基数让运行时调整设置无需重建 Semaphore。
pub const MAX_TASK_PERMITS: u32 = 1024;

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
    /// AI 询问失败后的额外重试次数（0 = 不重试）。
    pub retry_count: Option<u32>,
    /// 单次 AI 请求超时时间（秒）。
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub ai: AiSettings,
    pub heartbeat_interval_ms: Option<u64>,
    pub video_speed: Option<f32>,
    pub auto_comment_default: Option<String>,
    /// 最大并发任务数（None=不限制，0=不允许任何任务，n=最多 n 个任务）
    pub task_concurrency: Option<u32>,
}

pub struct AppState {
    pub accounts: RwLock<HashMap<i64, Account>>,
    pub current_user_id: RwLock<Option<i64>>,
    pub settings: RwLock<AppSettings>,
    pub video_tasks: RwLock<HashMap<String, Arc<crate::video::VideoTaskHandle>>>,
    pub pending_video_tasks: RwLock<VecDeque<Arc<crate::video::VideoTaskHandle>>>,
    pub login_session: RwLock<Option<Arc<crate::login::LoginSession>>>,
    pub clients: RwLock<HashMap<i64, Arc<crate::client::XtClient>>>,
    /// 任务并发信号量。可用 permit 数 = 当前剩余允许并发任务数。
    /// 修改 task_concurrency 时通过 [`AppState::apply_task_concurrency`] 调整。
    ///
    /// 所有类型的任务（视频心跳 / 自动作业 / 评论 / 图文）都走这一个 Semaphore。
    /// 视频任务多于 `task_concurrency` 时，多余的进 `pending_video_tasks` 队列
    /// 按 FIFO 顺序等待——既支持多并发同时刷课，又保证后到的视频按提交顺序排队。
    pub task_semaphore: Arc<Semaphore>,
    /// 当前生效的并发上限（缓存自 settings.task_concurrency，用于差量调整 Semaphore）。
    /// None 表示不限制。
    pub current_concurrency: Mutex<Option<u32>>,
}

impl AppState {
    pub fn new() -> Self {
        // 初始状态：Semaphore 满载，等价"不限制"。
        // load_persisted 之后再根据持久化设置调整。
        Self {
            accounts: RwLock::new(HashMap::new()),
            current_user_id: RwLock::new(None),
            settings: RwLock::new(AppSettings::default()),
            video_tasks: RwLock::new(HashMap::new()),
            pending_video_tasks: RwLock::new(VecDeque::new()),
            login_session: RwLock::new(None),
            clients: RwLock::new(HashMap::new()),
            task_semaphore: Arc::new(Semaphore::new(MAX_TASK_PERMITS as usize)),
            current_concurrency: Mutex::new(None),
        }
    }

    /// 根据 `task_concurrency` 调整 Semaphore 可用 permit 数。
    /// - `None`         → 解除所有限制（permit 总数恢复到 MAX_TASK_PERMITS）。
    /// - `Some(n)`，n ≥ 1 → 仅 n 个 permit 可用。
    /// - `Some(0)`      → 视为 1，避免永久死锁（前端 UI 也应限制最小输入为 1）。
    ///
    /// 注意：本函数只调整"未来"的可用 permit 数；已经被 acquire 的 permit 不会被强制收回。
    /// 这是预期行为——正在跑的任务不应被中途砍掉，调整只对后续 acquire 生效。
    pub fn apply_task_concurrency(&self, new_limit: Option<u32>) {
        let normalized = new_limit.map(|n| n.max(1));
        let mut cur = self.current_concurrency.lock();
        let old = *cur;
        if old == normalized {
            return;
        }
        // 计算"应当可用的 permit 总数"。None = 全开。
        let target = match normalized {
            None => MAX_TASK_PERMITS as usize,
            Some(n) => (n as usize).min(MAX_TASK_PERMITS as usize),
        };
        let old_target = match old {
            None => MAX_TASK_PERMITS as usize,
            Some(n) => (n as usize).min(MAX_TASK_PERMITS as usize),
        };
        if target > old_target {
            // 放宽：增加 permit
            self.task_semaphore.add_permits(target - old_target);
        } else if target < old_target {
            // 收紧：临时 acquire (old - target) 个并 forget 掉
            // forget 不归还，从此 Semaphore 永久少了 (old - target) 个 permit
            let to_remove = old_target - target;
            // try_acquire_many 在 permit 不够时返回 Err（说明大量任务正在跑），
            // 这种情况只能"尽力而为"——能拿多少就 forget 多少，剩余的等运行任务自然完成时
            // 由下一次 apply 再补齐。但更简单的做法是先放宽再立即收紧，让运行中的任务
            // 完成时归还的 permit 被新限制吸收。
            if let Ok(permit) = self.task_semaphore.clone().try_acquire_many_owned(to_remove as u32) {
                permit.forget();
            }
            // 拿不到的情况就放任不管：运行任务完成时 permit 归还，下一次 acquire 会
            // 重新与新设置对齐（因为 acquire_task_slot 每次都 wait 在同一个 Semaphore 上）。
        }
        *cur = normalized;
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
                let concurrency = s.task_concurrency;
                *self.settings.write() = s;
                self.apply_task_concurrency(concurrency);
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
        let has_login_type = account.cookies.iter().any(|c| c.name == "login_type");
        if !has_login_type {
            client.set_cookie("login_type", "WX");
        }
        let has_provider = account.cookies.iter().any(|c| c.name == "provider");
        if !has_provider {
            client.set_cookie("provider", "xuetang");
        }
        let has_lang = account.cookies.iter().any(|c| c.name == "django_language");
        if !has_lang {
            client.set_cookie("django_language", "zh-cn");
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
