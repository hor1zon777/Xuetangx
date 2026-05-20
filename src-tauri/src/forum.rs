use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::client::XtClient;

#[derive(Clone, Debug, Serialize)]
pub struct DiscussionTopic {
    pub topic_id: i64,
    pub to_user: i64,
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
    let to_user = d
        .get("user_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("forum 缺 user_id"))?;
    let title = d
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let commented = d.get("commented").and_then(|v| v.as_i64()).unwrap_or(0);
    Ok(DiscussionTopic {
        topic_id,
        to_user,
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
