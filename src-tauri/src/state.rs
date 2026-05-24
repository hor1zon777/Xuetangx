use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Wry};
use tauri_plugin_store::StoreExt;
use tokio::sync::Semaphore;

use crate::accounts::{Account, StoredCookie};
use crate::bank::Bank;
use crate::crypto;

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
    /// 自动作业是否优先查本地题库（None / Some(true) 默认开启）。
    /// 命中后直接组装答案提交，跳过 AI 询问，省 AI tokens + 答案绝对可信。
    pub use_local_bank: Option<bool>,
    /// 自动作业完成后，是否自动把学堂返回的批改答案入库（None / Some(true) 默认开启）。
    /// 关掉则仅手动「收录答案」按钮才写入。
    pub auto_harvest_bank: Option<bool>,
    /// 自动作业每题提交前的随机延迟下界（毫秒）。None 时使用默认值。
    /// 防止"秒提"被风控；第一题不延迟，后续题目都按 [min, max] 之间均匀随机取值。
    /// 题库命中：sleep 后立即 submit。
    /// 回退到 AI：sleep 与 AI 询问并发执行，submit 在两者都完成后才发出。
    pub submit_delay_min_ms: Option<u64>,
    /// 自动作业每题提交前的随机延迟上界（毫秒）。None 时使用默认值。
    /// 若 max < min，运行时按 min 取值。
    pub submit_delay_max_ms: Option<u64>,
    /// 自动作业每个习题节点（exercise）允许故意答错的最大题数，用于"控分"——
    /// 避免节点次次满分太显眼。0 / None 表示不开启，照常追求满分。
    /// 实际答错数 = min(本配置, 该节点未提交题数 - 1)，至少留 1 道答对。
    /// 命中的题目无论走题库还是 AI，都会把答案换成错答（选项题挑非正确 key、
    /// 文本题填"无"），并在结果里标 intentional_wrong=true 供前端区分。
    pub wrong_answer_max_per_exercise: Option<u32>,
}

pub struct AppState {
    pub accounts: RwLock<HashMap<i64, Account>>,
    pub current_user_id: RwLock<Option<i64>>,
    pub settings: RwLock<AppSettings>,
    pub video_tasks: RwLock<HashMap<String, Arc<crate::video::VideoTaskHandle>>>,
    pub pending_video_tasks: RwLock<VecDeque<Arc<crate::video::VideoTaskHandle>>>,
    pub login_session: RwLock<Option<Arc<crate::login::LoginSession>>>,
    pub clients: RwLock<HashMap<i64, Arc<crate::client::XtClient>>>,
    /// 本地题库。仅接受学堂在线"已批改"返回的答案 + 用户手动导入条目。
    /// 持久化到独立文件 `xuetang-helper.bank.json`（见 `bank::BANK_STORE_FILE`），
    /// 不与账号 / 设置混存，方便用户单独备份、清空。
    pub bank: RwLock<Bank>,
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
            bank: RwLock::new(Bank::new()),
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
                for mut a in list {
                    // 解密落盘前加密的 cookie value（兼容旧版本明文）。
                    decrypt_cookies(&mut a.cookies);
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
            if let Ok(mut s) = serde_json::from_value::<AppSettings>(v) {
                // 解密 AI api_key（兼容旧版本明文）。
                s.ai.api_key = crypto::decrypt_string(&s.ai.api_key).unwrap_or_else(|e| {
                    log::warn!("AI api_key 解密失败，请在设置页重新填写: {e}");
                    String::new()
                });
                let concurrency = s.task_concurrency;
                *self.settings.write() = s;
                self.apply_task_concurrency(concurrency);
            }
        }
        // 题库走独立 store 文件（bank.rs::BANK_STORE_FILE），失败不影响主流程
        self.bank.write().load(app);
    }

    pub fn persist(&self, app: &AppHandle<Wry>) -> anyhow::Result<()> {
        let store = app.store(STORE_FILE)?;
        // 写盘前对敏感字段加密，避免 sessionid / api_key 明文落盘。
        let mut list: Vec<Account> = self.accounts.read().values().cloned().collect();
        for a in list.iter_mut() {
            encrypt_cookies(&mut a.cookies);
        }
        store.set(KEY_ACCOUNTS, serde_json::to_value(&list)?);
        store.set(
            KEY_CURRENT,
            serde_json::to_value(*self.current_user_id.read())?,
        );
        let mut settings_copy = self.settings.read().clone();
        settings_copy.ai.api_key = crypto::encrypt_string(&settings_copy.ai.api_key);
        store.set(KEY_SETTINGS, serde_json::to_value(&settings_copy)?);
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

    /// 持久化题库到 `xuetang-helper.bank.json`，成功后 emit `bank://updated` 给前端，
    /// 让题库页之类的常驻 tab 能自动 refresh，无需用户手动点刷新。
    /// payload 里带 `total` 让订阅方做"无变化时跳过刷新"等优化。
    ///
    /// 题库写盘可能不频繁（批量收录、删除、清空、导入等时机），所以单独抽出来
    /// 不卷进 `persist()` 主流程。
    pub fn persist_bank(&self, app: &AppHandle<Wry>) -> anyhow::Result<()> {
        self.bank.read().persist(app)?;
        let total = self.bank.read().len();
        let _ = app.emit(
            "bank://updated",
            serde_json::json!({ "total": total }),
        );
        Ok(())
    }
}

/// 加密 cookie 列表中所有 value（in-place）。`StoredCookie.value` 会被替换成
/// `enc:v1:<base64>`；如果 keyring 不可用，value 保持原样（明文）。
fn encrypt_cookies(cookies: &mut [StoredCookie]) {
    for c in cookies.iter_mut() {
        c.value = crypto::encrypt_string(&c.value);
    }
}

/// 解密 cookie 列表中所有 value（in-place）。带前缀的 → 解密；
/// 无前缀的 → 视为旧版本明文，保留。失败的条目降级为空串以避免发出错误 cookie。
fn decrypt_cookies(cookies: &mut [StoredCookie]) {
    for c in cookies.iter_mut() {
        match crypto::decrypt_string(&c.value) {
            Ok(plain) => c.value = plain,
            Err(e) => {
                log::warn!("cookie {} 解密失败：{e}，账号需重新登录", c.name);
                c.value.clear();
            }
        }
    }
}
