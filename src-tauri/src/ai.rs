use anyhow::{anyhow, Result};
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
    let mut s = String::new();
    s.push_str("题型：");
    s.push_str(&p.type_text);
    s.push_str("\n题干：\n");
    s.push_str(&strip_html(&p.body_html));
    if !p.options.is_empty() {
        s.push_str("\n选项：\n");
        for (k, v) in &p.options {
            s.push_str(&format!("{k}. {}\n", strip_html(v)));
        }
        s.push_str("\n请仅输出最终答案。若是单选题，只输出一个大写字母（如 A）。");
        s.push_str("若是多选题，输出大写字母拼接（如 ABD），不要有逗号、空格或其它字符。");
        s.push_str("若是判断题，使用 A=对 B=错 的映射并只输出 A 或 B。");
        s.push_str("不要解释，不要重复题干。");
    } else {
        s.push_str("\n这是主观/填空/简答题。请直接给出答案文本，简洁、紧扣题意，不要解释。");
    }
    s
}

pub async fn ask_ai(ai: &AiSettings, p: &ProblemForAi) -> Result<AnswerSpec> {
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let resp = client
        .post(&url)
        .bearer_auth(&ai.api_key)
        .json(&body)
        .send()
        .await?;
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
        let keys: Vec<String> = p.options.iter().map(|(k, _)| k.to_uppercase()).collect();
        let upper = content.to_uppercase();
        let mut picked: Vec<String> = upper
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_string())
            .filter(|s| keys.contains(s))
            .collect();
        picked.sort();
        picked.dedup();
        if picked.is_empty() {
            // 兜底：拿第一个出现在响应中的合法选项
            for k in &keys {
                if upper.contains(k.as_str()) {
                    picked.push(k.clone());
                    break;
                }
            }
        }
        if picked.is_empty() {
            return Err(anyhow!("AI 输出无法解析为有效选项：{content}"));
        }
        Ok(AnswerSpec::Choice(picked))
    }
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
