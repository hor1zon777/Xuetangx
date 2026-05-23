//! 本地题库（local answer bank）。
//!
//! 数据来源：仅接受学堂在线 `/get_exercise_list` 在小题"已批改"时下发的 `answer` 字段。
//! AI 给出的答案**不写入**——AI 偶尔出错，写入会持续污染后续命中。
//!
//! 索引策略：
//! 1. 优先按 `problem_id` 精确匹配（同一道题在学堂的稳定 ID）。
//! 2. 退化按 `body_hash` 匹配，覆盖"题面相同但 problem_id 因换班次/重开课而变化"的情况。
//!    选项题命中时还要校验 `option_keys` 全等，避免选项顺序变化导致答案错位。
//!
//! 持久化：通过 `tauri-plugin-store` 写到独立文件 `xuetang-helper.bank.json`，
//! 与账号/设置分离，便于备份、导入导出和清空。

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use html_escape::decode_html_entities;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Wry};
use tauri_plugin_store::StoreExt;

use crate::exercise::{Problem, ProblemKind, ProblemOption};

pub const BANK_STORE_FILE: &str = "xuetang-helper.bank.json";
const KEY_ENTRIES: &str = "entries";
/// 题面预览长度：用于 UI 浏览，不参与命中匹配。
const BODY_PREVIEW_CHARS: usize = 80;

/// 题库条目来源。当前只有 Xuetang（学堂确认答案）和 Manual（用户手动导入）。
/// 未来若开放"AI 候选答案"，再扩 AiUnverified。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BankSource {
    /// 学堂在线服务端在批改后下发的正确答案（最高可信）
    Xuetang,
    /// 用户手动添加/导入的答案
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BankEntry {
    pub problem_id: i64,
    pub kind: ProblemKind,
    /// 题面前 80 字纯文本预览，UI 浏览用，不参与命中
    pub body_preview: String,
    /// sha256(strip_html(body) + "|" + 选项 key+value 拼接)。命中兜底用
    pub body_hash: String,
    /// 选项题的选项 key 序列（按学堂返回顺序）。命中 body_hash 时要全等
    pub option_keys: Vec<String>,
    /// 选项题答案：选中的 key 数组（如 ["A","C"]、["true"]）
    pub answer: Option<Vec<String>>,
    /// 文本题答案（填空/主观）
    pub answer_text: Option<String>,
    pub source: BankSource,
    pub updated_at: i64,
    /// 该条目被命中的累计次数（用于诊断 / Top 排行）
    pub hit_count: u32,
}

/// 题库主结构。
///
/// 内存中维护两套索引：
/// - `by_problem_id`：拥有完整条目
/// - `by_body_hash`：仅存 problem_id 引用，避免双份内存占用
///
/// 持久化时只序列化 `entries: Vec<BankEntry>`，加载时重建索引。
#[derive(Default)]
pub struct Bank {
    entries: HashMap<i64, BankEntry>, // problem_id -> entry
    by_body_hash: HashMap<String, i64>, // body_hash -> problem_id
}

/// 查询命中结果，附带命中方式（便于前端/进度事件展示）。
#[derive(Clone, Debug, Serialize)]
pub struct LookupHit {
    pub entry: BankEntry,
    pub matched_by: MatchedBy,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchedBy {
    ProblemId,
    BodyHash,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct BankStats {
    pub total: usize,
    pub by_kind: HashMap<String, usize>,
    pub by_source: HashMap<String, usize>,
    /// 累计命中次数总和
    pub total_hits: u64,
}

impl Bank {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self, app: &AppHandle<Wry>) {
        let Ok(store) = app.store(BANK_STORE_FILE) else {
            return;
        };
        if let Some(v) = store.get(KEY_ENTRIES) {
            if let Ok(list) = serde_json::from_value::<Vec<BankEntry>>(v) {
                self.entries.clear();
                self.by_body_hash.clear();
                for e in list {
                    self.by_body_hash.insert(e.body_hash.clone(), e.problem_id);
                    self.entries.insert(e.problem_id, e);
                }
            }
        }
    }

    pub fn persist(&self, app: &AppHandle<Wry>) -> Result<()> {
        let store = app.store(BANK_STORE_FILE)?;
        let list: Vec<&BankEntry> = self.entries.values().collect();
        store.set(KEY_ENTRIES, serde_json::to_value(&list)?);
        store.save()?;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 查询：当前题目 `p`（来自 fetch_exercise_with_referer 返回的 Problem）能否命中本地题库。
    ///
    /// 命中规则：
    /// 1. problem_id 严格匹配 → 命中
    /// 2. problem_id 未命中时，按 body_hash 查；选项题还要校验 option_keys 全等
    pub fn lookup(&self, p: &Problem) -> Option<LookupHit> {
        if let Some(entry) = self.entries.get(&p.problem_id) {
            // 同 problem_id 命中时不再额外校验选项顺序：学堂同一 problem_id 的选项是稳定的
            return Some(LookupHit {
                entry: entry.clone(),
                matched_by: MatchedBy::ProblemId,
            });
        }
        let hash = compute_body_hash(&p.body_html, &p.options);
        if let Some(pid) = self.by_body_hash.get(&hash) {
            if let Some(entry) = self.entries.get(pid) {
                // body_hash 命中：选项题要求 option_keys 完全一致，否则
                // 学堂打乱选项顺序时本地的 ["A","C"] 套用到当前题目就会答错。
                if p.kind.is_choice() {
                    let cur_keys: Vec<String> =
                        p.options.iter().map(|o| o.key.clone()).collect();
                    if cur_keys != entry.option_keys {
                        return None;
                    }
                }
                return Some(LookupHit {
                    entry: entry.clone(),
                    matched_by: MatchedBy::BodyHash,
                });
            }
        }
        None
    }

    /// 把"学堂已批改 + 有 correct_answer/correct_answer_text"的 Problem 写入题库。
    /// 返回 true 仅当这是一次**新增**（题库里之前不存在该 problem_id）；
    /// 已有 problem_id 的条目仍然会被 upsert（覆盖答案、更新时间戳、可能变化的 body_hash 索引），
    /// 但返回 false，因为它不应计入"新入库数量"。
    /// 返回 false 还涵盖：未提交 / 缺答案字段，这类不持久化。
    pub fn upsert_from_problem(&mut self, p: &Problem) -> bool {
        if !p.submitted {
            return false;
        }
        if p.correct_answer.is_none() && p.correct_answer_text.is_none() {
            return false;
        }
        let body_hash = compute_body_hash(&p.body_html, &p.options);
        let option_keys: Vec<String> = p.options.iter().map(|o| o.key.clone()).collect();
        let now = chrono::Utc::now().timestamp();
        let existed = self.entries.contains_key(&p.problem_id);
        let entry = BankEntry {
            problem_id: p.problem_id,
            kind: p.kind,
            body_preview: body_preview(&p.body_html),
            body_hash: body_hash.clone(),
            option_keys,
            answer: p.correct_answer.clone(),
            answer_text: p.correct_answer_text.clone(),
            source: BankSource::Xuetang,
            updated_at: now,
            hit_count: self
                .entries
                .get(&p.problem_id)
                .map(|e| e.hit_count)
                .unwrap_or(0),
        };
        // 旧条目可能 body_hash 已变化（学堂改了题面/选项），清掉旧索引
        if let Some(old) = self.entries.get(&p.problem_id) {
            if old.body_hash != body_hash {
                self.by_body_hash.remove(&old.body_hash);
            }
        }
        self.by_body_hash.insert(body_hash, p.problem_id);
        self.entries.insert(p.problem_id, entry);
        // 仅"题库之前不含该 problem_id"才算新入库；覆盖已有条目不计数。
        !existed
    }

    /// 命中后递增 hit_count（异步持久化由调用方决定）。
    pub fn record_hit(&mut self, problem_id: i64) {
        if let Some(e) = self.entries.get_mut(&problem_id) {
            e.hit_count = e.hit_count.saturating_add(1);
        }
    }

    pub fn delete(&mut self, problem_id: i64) -> bool {
        if let Some(e) = self.entries.remove(&problem_id) {
            self.by_body_hash.remove(&e.body_hash);
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_body_hash.clear();
    }

    /// 列表查询：可选 keyword（匹配 body_preview / answer_text / problem_id 字符串）。
    /// 返回按 updated_at 倒序的条目，limit 默认 200。
    pub fn list(&self, keyword: Option<&str>, offset: usize, limit: usize) -> Vec<BankEntry> {
        let kw = keyword.map(|s| s.trim().to_lowercase());
        let mut all: Vec<&BankEntry> = self
            .entries
            .values()
            .filter(|e| {
                let Some(ref k) = kw else { return true };
                if k.is_empty() {
                    return true;
                }
                if e.problem_id.to_string().contains(k) {
                    return true;
                }
                if e.body_preview.to_lowercase().contains(k) {
                    return true;
                }
                if let Some(t) = &e.answer_text {
                    if t.to_lowercase().contains(k) {
                        return true;
                    }
                }
                if let Some(arr) = &e.answer {
                    if arr.iter().any(|s| s.to_lowercase().contains(k)) {
                        return true;
                    }
                }
                false
            })
            .collect();
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        all.into_iter().skip(offset).take(limit).cloned().collect()
    }

    pub fn get(&self, problem_id: i64) -> Option<BankEntry> {
        self.entries.get(&problem_id).cloned()
    }

    pub fn stats(&self) -> BankStats {
        let mut s = BankStats {
            total: self.entries.len(),
            ..Default::default()
        };
        for e in self.entries.values() {
            let kk = match e.kind {
                ProblemKind::SingleChoice => "single_choice",
                ProblemKind::MultipleChoice => "multiple_choice",
                ProblemKind::Judgement => "judgement",
                ProblemKind::Completion => "completion",
                ProblemKind::Subjective => "subjective",
                ProblemKind::Other => "other",
            };
            *s.by_kind.entry(kk.to_string()).or_insert(0) += 1;
            let sk = match e.source {
                BankSource::Xuetang => "xuetang",
                BankSource::Manual => "manual",
            };
            *s.by_source.entry(sk.to_string()).or_insert(0) += 1;
            s.total_hits = s.total_hits.saturating_add(e.hit_count as u64);
        }
        s
    }

    pub fn export_all(&self) -> Vec<BankEntry> {
        self.entries.values().cloned().collect()
    }

    /// 导入：返回 (added, updated, skipped)。冲突时按 updated_at 更新更新者。
    /// skipped 计入数据不完整的条目。
    pub fn import(&mut self, list: Vec<BankEntry>) -> (usize, usize, usize) {
        let mut added = 0usize;
        let mut updated = 0usize;
        let mut skipped = 0usize;
        for mut e in list {
            if e.problem_id <= 0
                || e.body_hash.is_empty()
                || (e.answer.is_none() && e.answer_text.is_none())
            {
                skipped += 1;
                continue;
            }
            // 重建 body_hash 索引（旧条目 hash 不同时要清除旧映射）
            if let Some(old) = self.entries.get(&e.problem_id) {
                if old.updated_at >= e.updated_at {
                    skipped += 1;
                    continue;
                }
                if old.body_hash != e.body_hash {
                    self.by_body_hash.remove(&old.body_hash);
                }
                // 继承累计命中次数
                e.hit_count = e.hit_count.max(old.hit_count);
                self.by_body_hash.insert(e.body_hash.clone(), e.problem_id);
                self.entries.insert(e.problem_id, e);
                updated += 1;
            } else {
                self.by_body_hash.insert(e.body_hash.clone(), e.problem_id);
                self.entries.insert(e.problem_id, e);
                added += 1;
            }
        }
        (added, updated, skipped)
    }
}

/// 把 BankEntry 的 answer 字段转换成可以直接调用 `submit_problem` 的 (answer_arr, ui_text)。
/// 文本题返回的 ui_text 也用于 UI 展示。
pub fn entry_to_submit_payload(
    entry: &BankEntry,
    problem_id: i64,
) -> Result<(Vec<String>, serde_json::Value, String)> {
    if entry.kind.is_choice() {
        let Some(keys) = entry.answer.clone() else {
            return Err(anyhow!("题库条目缺选项答案"));
        };
        let ui = keys.join("");
        Ok((keys, serde_json::json!({}), ui))
    } else {
        let Some(text) = entry.answer_text.clone() else {
            return Err(anyhow!("题库条目缺文本答案"));
        };
        Ok((
            Vec::new(),
            serde_json::json!({ problem_id.to_string(): text }),
            text,
        ))
    }
}

/// 计算题面 + 选项的 sha256 哈希，作为兜底索引 key。
/// 选项参与哈希是为了让"题面相同但选项打乱"的题目得到不同的 hash，避免误命中。
pub fn compute_body_hash(body_html: &str, options: &[ProblemOption]) -> String {
    let body = normalize_body(body_html);
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher.update(b"\x1f"); // unit separator
    for o in options {
        hasher.update(o.key.as_bytes());
        hasher.update(b"=");
        hasher.update(normalize_body(&o.value).as_bytes());
        hasher.update(b"\x1e"); // record separator
    }
    let bytes = hasher.finalize();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// 标准化题面：去 HTML、解 HTML 实体、压缩空白。命中匹配 + 哈希都基于此。
fn normalize_body(s: &str) -> String {
    let re_tag = Regex::new(r"<[^>]+>").unwrap();
    let stripped = re_tag.replace_all(s, " ");
    let decoded = decode_html_entities(&stripped).into_owned();
    let re_ws = Regex::new(r"\s+").unwrap();
    re_ws.replace_all(decoded.trim(), " ").into_owned()
}

fn body_preview(body_html: &str) -> String {
    let normalized = normalize_body(body_html);
    let mut s: String = normalized.chars().take(BODY_PREVIEW_CHARS).collect();
    if normalized.chars().count() > BODY_PREVIEW_CHARS {
        s.push('…');
    }
    s
}
