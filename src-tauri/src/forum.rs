use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::client::XtClient;

/// 节点（leaf）的讨论话题信息。
/// `topic_owner_id` 是话题的发起者（通常是教师/课程管理员），
/// 用作发评论时的 `to_user` 字段。它与"当前登录用户"无关。
#[derive(Clone, Debug, Serialize)]
pub struct DiscussionTopic {
    pub topic_id: i64,
    /// 话题发起者用户 ID（教师/管理员），评论时作为 to_user。
    pub topic_owner_id: i64,
    pub commented: i64,
    /// 题目标题（用于 UI 显示）。
    ///
    /// 实测 `forum/unit/discussion?topic_type=4` 的响应里**没有** `title` 字段，
    /// 真正的"案例标题/案例描述"藏在 `content.text`（HTML 字符串）。所有"讨论
    /// （带分加）"节点在章节树里 `leaf.name` 都叫"案例分析"，如果 UI 只用
    /// `leaf.name` 来标识不同节点，就会出现"张冠李戴"——三个案例节点看上去
    /// 完全一样。这里把 `content.text` 转纯文本截 40 字作为预览，作为前端
    /// 区分节点的依据。
    pub title: String,
}

/// 当前账号在指定节点讨论区的状态。
///
/// 把"是否已评论"和"题目标题"合并返回，避免前端为同一个 leaf 发两次
/// `unit/discussion` 请求。批量检测/拉取标题共用这个结构。
#[derive(Clone, Debug, Serialize)]
pub struct MyTopicStatus {
    pub commented: bool,
    pub title: String,
}

/// 把 HTML 富文本压成"人话"用于 UI 显示。
///
/// 流程：
/// 1. 把块级标签结束符（`</p>` `</div>` `<br>` 等）替换成 `\n`，避免相邻
///    段落首尾直接粘连成"案例一张奶奶,19岁..."。
/// 2. 删掉所有 `<...>` 标签。
/// 3. 解码常见 HTML 实体（`&nbsp;` `&amp;` `&quot;` 等）。
/// 4. 按行去空白、丢弃空行，再用 `·` 拼接前几行。
/// 5. 按 Unicode 字符数（不是字节）截断到 `max_chars`，超出加省略号。
fn html_to_preview(html: &str, max_chars: usize) -> String {
    // 1. 块级结束标签 → 换行。包含大小写两种，避免漏掉教师贴的 UEditor 输出。
    let normalized = html
        .replace("</p>", "\n")
        .replace("</P>", "\n")
        .replace("</div>", "\n")
        .replace("</DIV>", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<BR>", "\n")
        .replace("<BR/>", "\n")
        .replace("</li>", "\n")
        .replace("</LI>", "\n");

    // 2. 剥所有 `<...>` 标签
    let mut stripped = String::with_capacity(normalized.len());
    let mut in_tag = false;
    for ch in normalized.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
            }
            _ if !in_tag => stripped.push(ch),
            _ => {}
        }
    }

    // 3. 解码常见 HTML 实体。学堂在线富文本里 `&quot;` `&nbsp;` 很常见。
    let decoded = stripped
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // 4. 按行整理：去前后空白、丢空行、用 `·` 拼接（人眼读案例一/案例描述更清楚）
    let lines: Vec<String> = decoded
        .split('\n')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let joined = lines.join(" · ");

    // 5. 按 Unicode 字符数截断（中文一字符 ≠ 一字节，必须 chars().count()）
    if joined.chars().count() <= max_chars {
        joined
    } else {
        let truncated: String = joined.chars().take(max_chars).collect();
        format!("{}…", truncated.trim_end())
    }
}

/// 拉取节点的讨论话题信息。
///
/// `topic_type` 决定话题种类，学堂在线前端按 leaf_type 选择：
/// - leaf_type=0（视频）+ leaf_type=3（图文）等附带讨论的节点 → `topic_type=0`
/// - leaf_type=4（独立的"讨论"节点，带分加） → `topic_type=4`
///
/// 旧调用方约定的 `topic_type=0` 仍然兼容，仅在需要新讨论时显式传 4。
pub async fn fetch_unit_discussion(
    client: &XtClient,
    sign: &str,
    classroom_id: i64,
    leaf_id: i64,
    topic_type: i64,
) -> Result<DiscussionTopic> {
    let path = format!(
        "/api/v1/lms/forum/unit/discussion/?product_sign={sign}&leaf_id={leaf_id}&classroom_id={classroom_id}&topic_type={topic_type}&channel=xt"
    );
    let v = client.get_json(&path).await?;
    let d = v.get("data").ok_or_else(|| anyhow!("forum 缺 data"))?;
    let topic_id = d
        .get("id")
        .or_else(|| d.get("topic_id"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("forum 缺 topic_id"))?;
    // data.user_id 实际是话题发起者 ID（教师/管理员），不是当前登录用户。
    // 评论 POST 时作为 to_user 提交。
    let topic_owner_id = d
        .get("user_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("forum 缺 topic owner user_id"))?;
    // 优先级：data.title → content.text 抽预览 → 空串。
    // 真实抓包里 topic_type=4 响应**没有** title 字段（所以以前 title 一直是空，
    // UI 上多个"案例分析"节点完全无法区分）；这里 fallback 到 content.text 抽
    // 40 字以内的预览，作为节点的实质性标题。
    let title_from_field = d
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let title_from_content = d
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|t| t.as_str())
        .map(|html| html_to_preview(html, 40))
        .filter(|s| !s.is_empty());
    let title = title_from_field
        .or(title_from_content)
        .unwrap_or_default();
    let commented = d.get("commented").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok(DiscussionTopic {
        topic_id,
        topic_owner_id,
        commented,
        title,
    })
}

pub async fn list_comments(
    client: &XtClient,
    topic_id: i64,
    classroom_id: i64,
    leaf_id: i64,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    let path = format!(
        "/api/v1/lms/forum/comment/list/{topic_id}/?offset={offset}&limit={limit}&cid={classroom_id}&lid={leaf_id}"
    );
    let v = client.get_json(&path).await?;
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

pub async fn post_comment(
    client: &XtClient,
    classroom_id: i64,
    leaf_id: i64,
    topic_id: i64,
    to_user: i64,
    text: &str,
) -> Result<Value> {
    let path = format!("/api/v1/lms/forum/comment/?classroom_id={classroom_id}&leaf_id={leaf_id}");
    let body = json!({
        "to_user": to_user,
        "topic_id": topic_id,
        "content": { "text": text, "upload_images": [] }
    });
    client.post_json(&path, &body).await
}

/// 同 [`post_comment`]，但返回 `(status, body)`，由调用方处理 429 等情况。
/// 批量评论需要识别 429 并按服务端给出的 `Expected available in N seconds.` 自适应等待。
pub async fn post_comment_raw(
    client: &XtClient,
    classroom_id: i64,
    leaf_id: i64,
    topic_id: i64,
    to_user: i64,
    text: &str,
) -> Result<(u16, String)> {
    let path = format!("/api/v1/lms/forum/comment/?classroom_id={classroom_id}&leaf_id={leaf_id}");
    let body = json!({
        "to_user": to_user,
        "topic_id": topic_id,
        "content": { "text": text, "upload_images": [] }
    });
    client.post_json_raw(&path, &body).await
}

/// 从学堂在线 429 响应体里解析 `Expected available in N(.N) seconds.`，返回秒数。
///
/// 实测响应体形如：
/// ```json
/// {"detail":"请求超过了限速。 Expected available in 42.0 seconds."}
/// ```
/// 解析失败时返回 None，调用方应回退到一个保守的默认等待（如 45s）。
/// 即使解析成功，也会 clamp 到 [0, 300] 秒，避免畸形服务端响应让程序挂上几小时。
pub fn parse_retry_after_seconds(body: &str) -> Option<f64> {
    const TAG: &str = "Expected available in";
    let start = body.find(TAG)?;
    let rest = body[start + TAG.len()..].trim_start();
    let mut num = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num.push(ch);
        } else {
            break;
        }
    }
    if num.is_empty() {
        None
    } else {
        num.parse::<f64>().ok().map(|n| n.clamp(0.0, 300.0))
    }
}

/// 在评论项里挖出"作者 user_id"。学堂在线不同接口返回的字段名不一致：
/// 顶层 `user_id`、`user.user_id`、`author.user_id` 都可能出现。这里逐一兜底。
fn extract_comment_user_id(c: &Value) -> Option<i64> {
    c.get("user_id")
        .and_then(|v| v.as_i64())
        .or_else(|| {
            c.get("user")
                .and_then(|u| u.get("user_id"))
                .and_then(|v| v.as_i64())
        })
        .or_else(|| {
            c.get("author")
                .and_then(|a| a.get("user_id"))
                .and_then(|v| v.as_i64())
        })
        .or_else(|| {
            // 旧版返回 sender 字段
            c.get("sender")
                .and_then(|s| s.get("user_id"))
                .and_then(|v| v.as_i64())
        })
}

/// 判断当前登录账号是否在指定节点的讨论区发过评论，并顺便带回 topic 标题。
///
/// 实现思路：
/// 1. 先拉 `unit/discussion` 拿到 topic_id / commented 计数 / 题目标题；
///    - 若 commented == 0，直接判定为"未评过"，省一次评论列表请求。
/// 2. 否则拉评论列表（取较大的 limit，按时间倒序通常够覆盖个人评论），
///    遍历每条评论的作者 ID，命中 `my_user_id` 即视为已评。
///
/// 即使评论非常多导致漏判，最坏的后果只是误将已评节点再次发送评论一次；
/// 风控间隔由 `auto_comment_leaf` 保证，因此不会造成滥用。
///
/// title 由 `fetch_unit_discussion` 从 `content.text` 抽取，保证就算节点
/// 在章节树里都叫"案例分析"，前端也能看到不同的"案例一/案例二/..."等区分。
pub async fn check_my_comment_status(
    client: &XtClient,
    sign: &str,
    classroom_id: i64,
    leaf_id: i64,
    topic_type: i64,
    my_user_id: i64,
) -> Result<MyTopicStatus> {
    let topic = fetch_unit_discussion(client, sign, classroom_id, leaf_id, topic_type).await?;
    if topic.commented <= 0 {
        return Ok(MyTopicStatus {
            commented: false,
            title: topic.title,
        });
    }
    // 分页扫评论列表，直到找到自己的评论或遍历完。
    // 单页 limit 设为 50（默认），最多扫 `MAX_PAGES * 50` 条；评论再多前端也不该
    // 等太久——但至少覆盖 commented 计数报告的范围，避免 limit=100 漏判导致
    // "已评 vs 未评" 误判后重复评论触发风控。
    const PAGE_LIMIT: usize = 50;
    const MAX_PAGES: usize = 40; // 上限 2000 条
    let mut offset = 0usize;
    let mut my_commented = false;
    for _ in 0..MAX_PAGES {
        let page = list_comments(client, topic.topic_id, classroom_id, leaf_id, offset, PAGE_LIMIT)
            .await?;
        let arr = page
            .get("list")
            .or_else(|| page.get("comments"))
            .or_else(|| page.get("results"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if arr.is_empty() {
            break;
        }
        if arr
            .iter()
            .any(|c| extract_comment_user_id(c) == Some(my_user_id))
        {
            my_commented = true;
            break;
        }
        // 如果当前页不足 PAGE_LIMIT，肯定没有下一页
        if arr.len() < PAGE_LIMIT {
            break;
        }
        offset += arr.len();
    }
    Ok(MyTopicStatus {
        commented: my_commented,
        title: topic.title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_extracts_first_lines_and_truncates() {
        let html = "<div class=\"x\"><p style=\"...\">案例一</p><p>张奶奶,19岁，昨天下午跟同学相约爬山，回家路上突然摔倒，自觉头痛伴随...</p><p><br/></p></div>";
        let out = html_to_preview(html, 30);
        assert!(out.starts_with("案例一 · 张奶奶,19岁"), "got: {out}");
        // 应该截断到 30 字以内（含省略号则更短）
        let len = out.chars().count();
        assert!(len <= 31, "preview too long: {len} chars: {out}");
    }

    #[test]
    fn preview_decodes_common_entities() {
        let html = "<p>A &amp; B</p><p>&quot;hi&quot;&nbsp;there</p>";
        assert_eq!(html_to_preview(html, 100), "A & B · \"hi\" there");
    }

    #[test]
    fn preview_handles_empty_or_pure_tags() {
        assert_eq!(html_to_preview("", 10), "");
        assert_eq!(html_to_preview("<p></p><br/>", 10), "");
    }
}
