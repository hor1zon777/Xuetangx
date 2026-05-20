use anyhow::{anyhow, Result};
use cookie_store::CookieStore;
use reqwest::{header::HeaderMap, Client, ClientBuilder};
use reqwest_cookie_store::CookieStoreMutex;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use url::Url;

use crate::accounts::{Account, StoredCookie};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const BASE: &str = "https://www.xuetangx.com";

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
            .build()?;
        Ok(Self { http, cookies })
    }

    pub fn from_account(account: &Account) -> Result<Self> {
        let client = Self::empty()?;
        let url = Url::parse(BASE)?;
        {
            let mut store = client.cookies.lock().unwrap();
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
        let store = self.cookies.lock().unwrap();
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

    pub fn csrf_token(&self) -> Option<String> {
        let store = self.cookies.lock().unwrap();
        let val = store
            .iter_any()
            .find(|c| c.name() == "csrftoken")
            .map(|c| c.value().to_string());
        val
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
        let mut h = HeaderMap::new();
        h.insert("x-client", "web".parse().unwrap());
        h.insert("xtbz", "xt".parse().unwrap());
        h.insert("app-name", "xtzx".parse().unwrap());
        h.insert("terminal-type", "web".parse().unwrap());
        h.insert("django-language", "zh".parse().unwrap());
        h.insert("x-requested-with", "XMLHttpRequest".parse().unwrap());
        h.insert(
            "accept",
            "application/json, text/plain, */*".parse().unwrap(),
        );
        h.insert(
            "Referer",
            referer.unwrap_or(BASE).parse().unwrap(),
        );
        h.insert("Origin", BASE.parse().unwrap());
        if let Some(tok) = self.csrf_token() {
            if let Ok(v) = tok.parse() {
                h.insert("x-csrftoken", v);
            }
        }
        h
    }

    /// 手动写入 cookie。用于登录成功后写 `k=<user_id>`、`mode_type=normal` 等
    /// 浏览器端 JS 自己 setCookie 而服务器不会 Set-Cookie 的字段。
    pub fn set_cookie(&self, name: &str, value: &str) {
        let url = Url::parse(BASE).unwrap();
        let header = format!("{name}={value}; Path=/; Domain=.xuetangx.com");
        let mut store = self.cookies.lock().unwrap();
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
            return Err(anyhow!("GET {} 失败: {} - {}", url, status, txt));
        }
        let v: Value = serde_json::from_str(&txt)
            .map_err(|e| anyhow!("解析 JSON 失败: {} - body: {}", e, &txt[..txt.len().min(400)]))?;
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
            return Err(anyhow!("POST {} 失败: {} - {}", url, status, txt));
        }
        if txt.trim().is_empty() {
            return Ok(Value::Null);
        }
        let v: Value = serde_json::from_str(&txt)
            .map_err(|e| anyhow!("解析 JSON 失败: {} - body: {}", e, &txt[..txt.len().min(400)]))?;
        Ok(v)
    }

    pub async fn warm_up(&self) -> Result<()> {
        let _ = self.http.get(BASE).send().await?.text().await.ok();
        Ok(())
    }
}

