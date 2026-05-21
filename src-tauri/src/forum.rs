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
    pub title: String,
}

pub async fn fetch_unit_discussion(
    client: &XtClient,
    sign: &str,
    classroom_id: i64,
    leaf_id: i64,
) -> Result<DiscussionTopic> {
    let path = format!(
        "/api/v1/lms/forum/unit/discussion/?product_sign={sign}&leaf_id={leaf_id}&classroom_id={classroom_id}&topic_type=0&channel=xt"
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
    let title = d
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
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
        num.parse().ok()
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

/// 判断当前登录账号是否在指定节点的讨论区发过评论。
///
/// 实现思路：
/// 1. 先拉 `unit/discussion` 拿到 topic_id 与 commented 计数；
///    - 若 commented == 0，直接判定为"未评过"，省一次评论列表请求。
/// 2. 否则拉评论列表（取较大的 limit，按时间倒序通常够覆盖个人评论），
///    遍历每条评论的作者 ID，命中 `my_user_id` 即视为已评。
///
/// 即使评论非常多导致漏判，最坏的后果只是误将已评节点再次发送评论一次；
/// 风控间隔由 `auto_comment_leaf` 保证，因此不会造成滥用。
pub async fn check_my_comment_status(
    client: &XtClient,
    sign: &str,
    classroom_id: i64,
    leaf_id: i64,
    my_user_id: i64,
) -> Result<bool> {
    let topic = fetch_unit_discussion(client, sign, classroom_id, leaf_id).await?;
    if topic.commented <= 0 {
        return Ok(false);
    }
    let comments = list_comments(client, topic.topic_id, classroom_id, leaf_id, 0, 100).await?;
    let arr = comments
        .get("list")
        .or_else(|| comments.get("comments"))
        .or_else(|| comments.get("results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for c in arr.iter() {
        if extract_comment_user_id(c) == Some(my_user_id) {
            return Ok(true);
        }
    }
    Ok(false)
}
