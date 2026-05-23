use anyhow::{anyhow, Context, Result};
use html_escape::decode_html_entities;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::state::AiSettings;

#[derive(Clone, Debug, Serialize)]
pub struct ProblemForAi {
    pub type_text: String,
    pub body_html: String,
    pub options: Vec<(String, String)>,
}

pub enum AnswerSpec {
    Choice(Vec<String>),
    Text(String),
}

fn strip_html(s: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    let stripped = re.replace_all(s, "");
    decode_html_entities(&stripped).trim().to_string()
}

fn build_prompt(p: &ProblemForAi) -> String {
    let label = p.type_text.as_str();
    let is_judgement = label.contains("判断");
    let is_multi = label.contains("多选") || label.contains("不定项");
    let is_single = label.contains("单选");
    let is_completion = label.contains("填空");
    let body = strip_html(&p.body_html);
    let options = p
        .options
        .iter()
        .map(|(k, v)| format!("{k}. {}", strip_html(v)))
        .collect::<Vec<_>>()
        .join("\n");
    let option_keys = p
        .options
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let has_boolean_keys = p
        .options
        .iter()
        .any(|(k, _)| matches!(k.as_str(), "true" | "false"));

    if is_judgement {
        if has_boolean_keys {
            return format!(
                "你正在解一道判断题。\n\n题干：\n{body}\n\n选项：\n{options}\n\n输出规则：\n1. 只能输出 true 或 false。\n2. true 表示正确/对，false 表示错误/错。\n3. 不要输出 A/B、中文、解释、标点或多余内容。\n\n最终答案："
            );
        }
        return format!(
            "你正在解一道判断题。\n\n题干：\n{body}\n\n选项：\n{options}\n\n输出规则：\n1. 只能输出一个选项 key，合法 key 只有：{option_keys}。\n2. 如果 A=正确、B=错误，请按该映射输出 A 或 B。\n3. 不要解释，不要输出题干，不要输出标点或多余内容。\n\n最终答案："
        );
    }

    if is_single {
        return format!(
            "你正在解一道单选题。\n\n题干：\n{body}\n\n选项：\n{options}\n\n输出规则：\n1. 只能输出一个选项 key。\n2. 合法 key 只有：{option_keys}。\n3. 不要解释，不要输出题干，不要输出标点或多余内容。\n\n最终答案："
        );
    }

    if is_multi {
        return format!(
            "你正在解一道多选题。\n\n题干：\n{body}\n\n选项：\n{options}\n\n输出规则：\n1. 输出所有正确选项 key，按字母/选项顺序直接拼接。\n2. 合法 key 只有：{option_keys}。\n3. 示例：如果选 A、B、D，只输出 ABD。\n4. 不要逗号、空格、解释、题干、标点或多余内容。\n\n最终答案："
        );
    }

    if is_completion {
        return format!(
            "你正在解一道填空题。\n\n题干：\n{body}\n\n输出规则：\n1. 只输出填空内容。\n2. 多个空位用 ## 分隔。\n3. 不要解释，不要复述题干，不要加“答案是”。\n\n最终答案："
        );
    }

    if !p.options.is_empty() {
        return format!(
            "你正在解一道选择题。\n\n题干：\n{body}\n\n选项：\n{options}\n\n输出规则：\n1. 只能输出选项 key。\n2. 合法 key 只有：{option_keys}。\n3. 单选输出一个 key；多选按顺序拼接多个 key。\n4. 不要解释，不要输出题干，不要输出标点或多余内容。\n\n最终答案："
        );
    }

    format!(
        "你正在解一道主观/简答题。\n\n题干：\n{body}\n\n输出规则：\n1. 直接给出答案文本，紧扣题意。\n2. 不超过 200 字。\n3. 不要解释答题思路，不要复述题干。\n\n最终答案："
    )
}

pub async fn ask_ai(ai: &AiSettings, p: &ProblemForAi) -> Result<AnswerSpec> {
    ask_ai_once(ai, p).await
}

pub async fn ask_ai_with_retry(ai: &AiSettings, p: &ProblemForAi) -> Result<AnswerSpec> {
    let retry_count = ai.retry_count.unwrap_or(2).min(10);
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=retry_count {
        match ask_ai_once(ai, p).await {
            Ok(spec) => return Ok(spec),
            Err(e) => {
                if attempt >= retry_count {
                    let attempts = retry_count + 1;
                    return Err(anyhow!("AI 询问失败，已尝试 {attempts} 次：{e}"));
                }
                last_err = Some(e);
                let delay_ms = 300_u64.saturating_mul(1_u64 << attempt.min(3));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow!("AI 询问失败")))
}

async fn ask_ai_once(ai: &AiSettings, p: &ProblemForAi) -> Result<AnswerSpec> {
    if ai.api_key.trim().is_empty() {
        return Err(anyhow!("AI api_key 为空，无法询问"));
    }
    let base = ai.base_url.trim_end_matches('/').to_string();
    let url = if base.is_empty() {
        "https://api.openai.com/v1/chat/completions".to_string()
    } else if base.ends_with("/chat/completions") {
        base
    } else {
        format!("{base}/chat/completions")
    };
    let prompt = build_prompt(p);
    let system_prompt = ai.system_prompt.clone().unwrap_or_else(|| {
        "你是一位严谨的中文学科助教，只输出最终答案，不要解释。".to_string()
    });
    let body = json!({
        "model": if ai.model.is_empty() { "gpt-4o-mini".to_string() } else { ai.model.clone() },
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": prompt }
        ],
        "temperature": ai.temperature.unwrap_or(0.1)
    });

    let timeout_secs = ai.timeout_secs.unwrap_or(30).clamp(1, 300);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()?;
    let resp = client
        .post(&url)
        .bearer_auth(&ai.api_key)
        .json(&body)
        .send()
        .await
        .with_context(|| "AI 请求超时或网络连接失败")?;
    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(anyhow!("AI 请求失败 {}：{}", status, txt));
    }
    let v: Value = serde_json::from_str(&txt)
        .map_err(|e| anyhow!("AI 响应非 JSON：{e}；body：{}", &txt[..txt.len().min(400)]))?;
    let content = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("AI 响应缺 choices[0].message.content"))?
        .trim()
        .to_string();

    if p.options.is_empty() {
        Ok(AnswerSpec::Text(content))
    } else {
        let keys: Vec<String> = p.options.iter().map(|(k, _)| k.to_string()).collect();
        let picked = parse_choice_answer(&content, &keys);
        if picked.is_empty() {
            return Err(anyhow!("AI 输出无法解析为有效选项：{content}"));
        }
        Ok(AnswerSpec::Choice(picked))
    }
}

/// 从 AI 回复中解析选项答案。优先策略：
/// 1. 提取所有"连续大写字母段"（如 "ABD"、"ABCDE"），取最长的一段；
/// 2. 若失败，回退到提取所有合法单字母（去重排序）；
/// 3. 自动剥离常见噪声词（"答案是""选""我选"等）。
fn parse_choice_answer(content: &str, valid_keys: &[String]) -> Vec<String> {
    // 简单规范化：去掉中文标点、空白
    let cleaned: String = content
        .chars()
        .filter(|c| !"，。、；：（）()【】[]「」“”\"'`\n\r\t ".contains(*c))
        .collect();
    let upper = cleaned.to_uppercase();
    let lower = cleaned.to_lowercase();
    let valid_set: std::collections::HashSet<&str> =
        valid_keys.iter().map(|s| s.as_str()).collect();
    let valid_upper_set: std::collections::HashSet<String> =
        valid_keys.iter().map(|s| s.to_uppercase()).collect();

    let has_true = valid_set.contains("true");
    let has_false = valid_set.contains("false");
    if has_true || has_false {
        if has_true && (lower == "true" || lower.contains("正确") || lower.contains('对')) {
            return vec!["true".to_string()];
        }
        if has_false && (lower == "false" || lower.contains("错误") || lower.contains('错')) {
            return vec!["false".to_string()];
        }
        if has_true && upper == "A" {
            return vec!["true".to_string()];
        }
        if has_false && upper == "B" {
            return vec!["false".to_string()];
        }
    }

    if valid_set.contains(cleaned.as_str()) {
        return vec![cleaned];
    }

    // 1) 找连续大写字母段，按长度降序优先匹配（多选优于单选）
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in upper.chars() {
        if c.is_ascii_uppercase() {
            current.push(c);
        } else {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    // 选最长的且所有字母都在 valid_set 内的段
    segments.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for seg in &segments {
        if seg.chars().all(|c| valid_upper_set.contains(&c.to_string())) {
            let mut keys: Vec<String> = seg.chars().map(|c| c.to_string()).collect();
            keys.sort();
            keys.dedup();
            if !keys.is_empty() {
                return keys;
            }
        }
    }

    // 2) 回退：所有合法单字母（去重）
    let mut fallback: Vec<String> = upper
        .chars()
        .filter(|c| c.is_ascii_uppercase())
        .map(|c| c.to_string())
        .filter(|s| valid_upper_set.contains(s))
        .collect();
    fallback.sort();
    fallback.dedup();
    fallback
}

pub async fn test_settings(ai: &AiSettings) -> Result<String> {
    let demo = ProblemForAi {
        type_text: "单选题".into(),
        body_html: "1+1=?".into(),
        options: vec![("A".into(), "1".into()), ("B".into(), "2".into())],
    };
    match ask_ai(ai, &demo).await? {
        AnswerSpec::Choice(c) => Ok(format!("AI 已连通，示例答案：{}", c.join(""))),
        AnswerSpec::Text(t) => Ok(format!("AI 已连通，示例答案：{t}")),
    }
}
