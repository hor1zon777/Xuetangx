use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub user_id: i64,
    pub nickname: String,
    pub avatar: Option<String>,
    pub login_type: Option<String>,
    pub login_time: i64,
    /// Netscape 风格 cookie 列表，便于序列化与重建
    pub cookies: Vec<StoredCookie>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub expires_unix: Option<i64>,
}
