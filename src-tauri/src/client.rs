use anyhow::{anyhow, Result};
use cookie_store::CookieStore;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, ClientBuilder,
};
use reqwest_cookie_store::CookieStoreMutex;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, MutexGuard};
use url::Url;

use crate::accounts::{Account, StoredCookie};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
const BASE: &str = "https://www.xuetangx.com";

/// 按 Unicode 字符截短一段响应文本，避免裸字节切片在 UTF-8 边界 panic。
/// 用于把上游错误体安全地塞进 anyhow! 错误消息。
fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
    }
    out
}

pub struct XtClient {
    pub http: Client,
    pub cookies: Arc<CookieStoreMutex>,
}

impl XtClient {
    pub fn empty() -> Result<Self> {
        let cookies = Arc::new(CookieStoreMutex::new(CookieStore::default()));
        let http = ClientBuilder::new()
            .user_agent(UA)
            .cookie_provider(cookies.clone())
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(std::time::Duration::from_secs(30))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .build()?;
        Ok(Self { http, cookies })
    }

    /// 安全获取 cookie store 读锁。`std::sync::Mutex` 在持锁线程 panic 后会中毒，
    /// 此处通过 `into_inner` 恢复，避免整个 client 因 unwrap 而连锁 panic。
    fn cookies_guard(
        &self,
    ) -> MutexGuard<'_, cookie_store::CookieStore> {
        match self.cookies.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn from_account(account: &Account) -> Result<Self> {
        let client = Self::empty()?;
        let url = Url::parse(BASE)?;
        {
            let mut store = client.cookies_guard();
            for c in &account.cookies {
                let mut header = format!(
                    "{}={}; Path={}; Domain={}",
                    c.name,
                    c.value,
                    if c.path.is_empty() { "/".into() } else { c.path.clone() },
                    if c.domain.is_empty() {
                        "www.xuetangx.com".into()
                    } else {
                        c.domain.clone()
                    }
                );
                if c.secure {
                    header.push_str("; Secure");
                }
                if c.http_only {
                    header.push_str("; HttpOnly");
                }
                let _ = store.parse(&header, &url);
            }
        }
        Ok(client)
    }

    pub fn export_cookies(&self) -> Vec<StoredCookie> {
        let store = self.cookies_guard();
        store
            .iter_any()
            .map(|c| StoredCookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().unwrap_or("").to_string(),
                path: c.path().unwrap_or("/").to_string(),
                secure: c.secure().unwrap_or(false),
                http_only: c.http_only().unwrap_or(false),
                expires_unix: c.expires_datetime().map(|t| t.unix_timestamp()),
            })
            .collect()
    }

    /// 列出当前 cookie 仅含 name（用于诊断输出，不暴露 value）。
    pub fn cookie_names(&self) -> Vec<String> {
        let store = self.cookies_guard();
        store.iter_any().map(|c| c.name().to_string()).collect()
    }

    pub fn csrf_token(&self) -> Option<String> {
        let store = self.cookies_guard();
        let token = store
            .iter_any()
            .find(|c| c.name() == "csrftoken")
            .map(|c| c.value().to_string());
        token
    }

    pub fn build_url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else if path.starts_with('/') {
            format!("{}{}", BASE, path)
        } else {
            format!("{}/{}", BASE, path)
        }
    }

    fn common_headers(&self) -> HeaderMap {
        self.common_headers_with(None)
    }

    fn common_headers_with(&self, referer: Option<&str>) -> HeaderMap {
        let mut h = base_xt_headers();
        h.insert("x-requested-with", HeaderValue::from_static("XMLHttpRequest"));
        h.insert("Origin", HeaderValue::from_static(BASE));
        insert_referer(&mut h, referer.unwrap_or(BASE));
        if let Some(tok) = self.csrf_token() {
            if let Ok(v) = HeaderValue::from_str(&tok) {
                h.insert("x-csrftoken", v);
            }
        }
        h
    }

    /// 手动写入 cookie。用于登录成功后写 `k=<user_id>`、`mode_type=normal` 等
    /// 浏览器端 JS 自己 setCookie 而服务器不会 Set-Cookie 的字段。
    pub fn set_cookie(&self, name: &str, value: &str) {
        let Ok(url) = Url::parse(BASE) else {
            return;
        };
        let header = format!("{name}={value}; Path=/; Domain=.xuetangx.com");
        let mut store = self.cookies_guard();
        let _ = store.parse(&header, &url);
    }

    pub async fn get_json(&self, path: &str) -> Result<Value> {
        self.get_json_with_referer(path, None).await
    }

    pub async fn get_json_with_referer(
        &self,
        path: &str,
        referer: Option<&str>,
    ) -> Result<Value> {
        let url = self.build_url(path);
        let resp = self
            .http
            .get(&url)
            .headers(self.common_headers_with(referer))
            .send()
            .await?;
        let status = resp.status();
        let txt = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "GET {} 失败: {} - {}",
                url,
                status,
                truncate_chars(&txt, 400)
            ));
        }
        let v: Value = serde_json::from_str(&txt).map_err(|e| {
            anyhow!(
                "解析 JSON 失败: {} - body: {}",
                e,
                truncate_chars(&txt, 400)
            )
        })?;
        Ok(v)
    }

    /// Browser-like same-origin GET.
    ///
    /// Some LMS exercise endpoints are sensitive to the navigation context.  HAR shows
    /// `/api/v1/lms/exercise/get_exercise_list/...` is requested with a concrete
    /// exercise-page Referer and without `Origin` / `X-Requested-With`, so this helper
    /// intentionally matches that shape more closely than `common_headers_with`.
    pub async fn get_json_same_origin(&self, path: &str, referer: &str) -> Result<Value> {
        let url = self.build_url(path);
        let h = self.same_origin_headers(referer);

        let resp = self.http.get(&url).headers(h).send().await?;
        let status = resp.status();
        let txt = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "GET {} 失败: {} - {}",
                url,
                status,
                truncate_chars(&txt, 400)
            ));
        }
        let v: Value = serde_json::from_str(&txt).map_err(|e| {
            anyhow!(
                "解析 JSON 失败: {} - body: {}",
                e,
                truncate_chars(&txt, 400)
            )
        })?;
        Ok(v)
    }

    fn same_origin_headers(&self, referer: &str) -> HeaderMap {
        let mut h = base_xt_headers();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h.insert(
            "sec-ch-ua",
            HeaderValue::from_static(
                "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\"",
            ),
        );
        h.insert("sec-ch-ua-mobile", HeaderValue::from_static("?0"));
        h.insert("sec-ch-ua-platform", HeaderValue::from_static("\"Windows\""));
        h.insert("accept-language", HeaderValue::from_static("zh-CN,zh;q=0.9"));
        h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        h.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        h.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        insert_referer(&mut h, referer);
        if let Some(tok) = self.csrf_token() {
            if let Ok(v) = HeaderValue::from_str(&tok) {
                h.insert("x-csrftoken", v);
            }
        }
        h
    }

    pub async fn post_json_with_referer<T: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &T,
        referer: &str,
    ) -> Result<Value> {
        let url = self.build_url(path);
        let resp = self
            .http
            .post(&url)
            .headers(self.common_headers_with(Some(referer)))
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let txt = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "POST {} 失败: {} - {}",
                url,
                status,
                truncate_chars(&txt, 400)
            ));
        }
        if txt.trim().is_empty() {
            return Ok(Value::Null);
        }
        let v: Value = serde_json::from_str(&txt).map_err(|e| {
            anyhow!(
                "解析 JSON 失败: {} - body: {}",
                e,
                truncate_chars(&txt, 400)
            )
        })?;
        Ok(v)
    }

    pub async fn get_raw(&self, path: &str, referer: Option<&str>) -> Result<(u16, String)> {
        let url = self.build_url(path);
        let resp = self
            .http
            .get(&url)
            .headers(self.common_headers_with(referer))
            .send()
            .await?;
        let status = resp.status().as_u16();
        let txt = resp.text().await?;
        Ok((status, txt))
    }

    pub async fn get_raw_same_origin(&self, path: &str, referer: &str) -> Result<(u16, String)> {
        let url = self.build_url(path);
        let h = self.same_origin_headers(referer);
        let resp = self.http.get(&url).headers(h).send().await?;
        let status = resp.status().as_u16();
        let txt = resp.text().await?;
        Ok((status, txt))
    }

    pub async fn post_json<T: Serialize + ?Sized>(&self, path: &str, body: &T) -> Result<Value> {
        let url = self.build_url(path);
        let resp = self
            .http
            .post(&url)
            .headers(self.common_headers())
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let txt = resp.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "POST {} 失败: {} - {}",
                url,
                status,
                truncate_chars(&txt, 400)
            ));
        }
        if txt.trim().is_empty() {
            return Ok(Value::Null);
        }
        let v: Value = serde_json::from_str(&txt).map_err(|e| {
            anyhow!(
                "解析 JSON 失败: {} - body: {}",
                e,
                truncate_chars(&txt, 400)
            )
        })?;
        Ok(v)
    }

    /// 同 `post_json`，但返回原始 `(status_code, body)`，由调用方自行处理非 2xx 状态。
    /// 用于需要识别 429 / Retry-After 等场景，避免把 status 包进字符串错误后再正则解析。
    pub async fn post_json_raw<T: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<(u16, String)> {
        let url = self.build_url(path);
        let resp = self
            .http
            .post(&url)
            .headers(self.common_headers())
            .json(body)
            .send()
            .await?;
        let status = resp.status().as_u16();
        let txt = resp.text().await?;
        Ok((status, txt))
    }

    pub async fn warm_up(&self) -> Result<()> {
        let _ = self.http.get(BASE).send().await?.text().await.ok();
        Ok(())
    }
}

/// 基础 X-T 同源 header 集（不含 Referer / Origin / csrftoken）。
fn base_xt_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("x-client", HeaderValue::from_static("web"));
    h.insert("xtbz", HeaderValue::from_static("xt"));
    h.insert("app-name", HeaderValue::from_static("xtzx"));
    h.insert("terminal-type", HeaderValue::from_static("web"));
    h.insert("django-language", HeaderValue::from_static("zh"));
    h.insert(
        "accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    h
}

/// 把 referer 安全地写进 header。如果 referer 字符串里出现 `\r\n` 等导致
/// HeaderValue 解析失败的字符，回退到 BASE，避免请求被 reqwest 直接拒绝。
fn insert_referer(h: &mut HeaderMap, referer: &str) {
    let name = HeaderName::from_static("referer");
    match HeaderValue::from_str(referer) {
        Ok(v) => {
            h.insert(name, v);
        }
        Err(_) => {
            h.insert(name, HeaderValue::from_static(BASE));
        }
    }
}
