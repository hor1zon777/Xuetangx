//! 本地敏感数据加密。
//!
//! 目的：把 cookie value（sessionid/csrftoken 等）和 AI API Key 加密后再写入
//! tauri-plugin-store 的 JSON 文件，避免明文落盘被信息窃取程序直接读取。
//!
//! 设计：
//! 1. 主密钥（32 字节随机）由系统密钥环（Windows Credential Manager / macOS
//!    Keychain / Linux secret-service）保管，应用第一次启动时生成。
//! 2. 字符串敏感字段用 AES-256-GCM 加密：随机 12 字节 nonce + 密文 + 16 字节
//!    认证标签，序列化为 `enc:v1:<base64-no-padding>` 字符串。
//! 3. 解密接受同前缀字符串，遇到旧版本明文（无前缀）则**原样返回**，提供
//!    一次性向后兼容；推荐用户重新登录 / 重保设置触发重新加密落盘。
//!
//! 如果系统密钥环不可用（无头 Linux 服务器、Wine、企业策略禁用等），
//! 模块自动降级为"不加密"——`encrypt_string` 返回原文。这保证应用仍可运行，
//! 但会通过 [`Encryptor::is_active`] 让调用方/上层 UI 知晓，便于在 About
//! 页面提示用户。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use once_cell::sync::OnceCell;
use rand::RngCore;

const KEYRING_SERVICE: &str = "com.captain.xuetanghelper";
const KEYRING_USER: &str = "store-master-key";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const ENC_PREFIX: &str = "enc:v1:";

/// 进程级单例。lazy 初始化，首次调用时尝试从 keyring 读密钥；
/// 失败则置为 `Encryptor::Disabled`，调用方继续以明文方式工作。
static ENCRYPTOR: OnceCell<Encryptor> = OnceCell::new();

pub enum Encryptor {
    Active(Aes256Gcm),
    Disabled,
}

impl Encryptor {
    /// 是否成功启用加密（keyring 可用且密钥已就位）。
    pub fn is_active(&self) -> bool {
        matches!(self, Encryptor::Active(_))
    }
}

/// 获取全局加密器。首次调用会读 / 写 keyring。
pub fn instance() -> &'static Encryptor {
    ENCRYPTOR.get_or_init(|| match load_or_create_key() {
        Ok(key_bytes) => {
            let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
            Encryptor::Active(Aes256Gcm::new(key))
        }
        Err(e) => {
            log::warn!(
                "本地密钥环不可用，敏感字段将以明文持久化（建议手动清理 store.json）：{e:#}"
            );
            Encryptor::Disabled
        }
    })
}

/// 尝试从 keyring 读密钥；不存在则生成新密钥并写入。
fn load_or_create_key() -> Result<[u8; KEY_BYTES]> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .context("创建 keyring entry 失败")?;
    match entry.get_password() {
        Ok(s) => {
            let bytes = STANDARD_NO_PAD
                .decode(s.as_bytes())
                .context("解码 keyring 中的主密钥失败")?;
            if bytes.len() != KEY_BYTES {
                anyhow::bail!("keyring 中密钥长度异常: {} bytes", bytes.len());
            }
            let mut out = [0u8; KEY_BYTES];
            out.copy_from_slice(&bytes);
            Ok(out)
        }
        Err(keyring::Error::NoEntry) => {
            let mut new_key = [0u8; KEY_BYTES];
            rand::thread_rng().fill_bytes(&mut new_key);
            entry
                .set_password(&STANDARD_NO_PAD.encode(new_key))
                .context("写入 keyring 失败")?;
            Ok(new_key)
        }
        Err(e) => Err(anyhow!("读 keyring 失败: {e}")),
    }
}

/// 加密一段字符串。返回 `enc:v1:<base64>`；keyring 不可用时返回原文。
/// 空串原样返回（避免污染未填写的 api_key 字段）。
pub fn encrypt_string(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    if plain.starts_with(ENC_PREFIX) {
        // 已经是密文，直接透传（防止双重加密）
        return plain.to_string();
    }
    let cipher = match instance() {
        Encryptor::Active(c) => c,
        Encryptor::Disabled => return plain.to_string(),
    };
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    match cipher.encrypt(nonce, plain.as_bytes()) {
        Ok(ct) => {
            let mut buf = Vec::with_capacity(NONCE_BYTES + ct.len());
            buf.extend_from_slice(&nonce_bytes);
            buf.extend_from_slice(&ct);
            format!("{ENC_PREFIX}{}", STANDARD_NO_PAD.encode(buf))
        }
        Err(e) => {
            log::warn!("加密失败，回退明文存储: {e}");
            plain.to_string()
        }
    }
}

/// 解密 `enc:v1:` 前缀字符串。
/// - 已加密：尝试解密，失败则返回错误（不要 silently 用明文，可能是损坏）。
/// - 未加密（无前缀）：视为明文向后兼容，原样返回。
/// - 空串：返回空串。
pub fn decrypt_string(value: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let Some(b64) = value.strip_prefix(ENC_PREFIX) else {
        // 旧版本明文，向后兼容
        return Ok(value.to_string());
    };
    let cipher = match instance() {
        Encryptor::Active(c) => c,
        Encryptor::Disabled => {
            anyhow::bail!("数据已加密但本地密钥环不可用，请重新登录 / 重新填写敏感设置");
        }
    };
    let raw = STANDARD_NO_PAD
        .decode(b64.as_bytes())
        .context("base64 解码失败")?;
    if raw.len() <= NONCE_BYTES {
        anyhow::bail!("密文格式异常（长度过短）");
    }
    let (nonce_bytes, ct) = raw.split_at(NONCE_BYTES);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|e| anyhow!("AES-GCM 解密失败: {e}"))?;
    String::from_utf8(plain).context("密文解出非 UTF-8")
}

/// 判断字符串是否带加密前缀。
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENC_PREFIX)
}
