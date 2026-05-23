//! 图文（leaf_type=3）类型节点的"标记完成"实现。
//!
//! 抓包观察到 HAR 中前端进入图文页面后的关键流：
//!   1. GET /api/v1/lms/learn/leaf_info/{cid}/{leaf_id}/?sign=...
//!   2. GET /api/v1/lms/learn/user_article_finish/{leaf_id}/?cid=...&sid=...
//!   3. POST /api/v1/lms/learn/chapter/schedule
//!        body: {"leaf_id":..., "classroom_id":..., "sku_id":...}
//!
//! 第 3 步是触发学习进度上升的关键调用：调用后 `course/schedule` 中该 leaf 的
//! `leaf_schedules[leaf_id]` 由 None 变为 1，表示已完成。讨论节点（leaf_type=4）
//! 在 POST 评论之后也复用同一接口（参见 `forum.rs::auto_comment_leaf` 调用方）。
//!
//! 我们暴露两个 helper：
//! - [`mark_chapter_schedule`]：发出 POST chapter/schedule，返回原始 `(status, body)`，
//!   供 forum / article 复用。
//! - [`mark_article_finish`]：图文节点的"标记完成"组合，先 GET user_article_finish（兼容
//!   服务端对热路径上下文的预检），再 POST chapter/schedule。

use anyhow::Result;
use serde_json::json;

use crate::client::XtClient;

/// 上报 `chapter/schedule`，告诉服务端某个 leaf 已经学完。
///
/// 返回 `(status, body)` 让上层根据 HTTP 状态决定是否成功；网络层错误（连接失败等）
/// 仍会返回 `Err`，由调用者上抛或转换为统一的"失败结果"。
pub async fn mark_chapter_schedule(
    client: &XtClient,
    classroom_id: i64,
    leaf_id: i64,
    sku_id: i64,
    referer: &str,
) -> Result<(u16, String)> {
    let body = json!({
        "leaf_id": leaf_id,
        "classroom_id": classroom_id,
        "sku_id": sku_id,
    });
    // 此接口在浏览器里以同源 Referer 发起；这里通过 post_json_with_referer 设置同样的
    // Referer，避免被风控判定为来源异常。post_json_with_referer 仅支持 2xx，所以这里
    // 走原始 reqwest 路径以拿到状态码/正文用于上报。
    let url = client.build_url("/api/v1/lms/learn/chapter/schedule");
    let resp = client
        .http
        .post(&url)
        .headers(common_post_headers(client, referer))
        .json(&body)
        .send()
        .await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;
    Ok((status, text))
}

/// 图文节点的"标记完成"流程。
/// 先发 user_article_finish GET（与浏览器顺序一致，避免被识别为越过预检），
/// 再 POST chapter/schedule，返回 schedule 调用的 `(status, body)`。
pub async fn mark_article_finish(
    client: &XtClient,
    classroom_id: i64,
    sku_id: i64,
    sign: &str,
    leaf_id: i64,
) -> Result<(u16, String)> {
    let referer = format!(
        "https://www.xuetangx.com/learn/space/{sign}/{sign}/{classroom_id}/article/{leaf_id}"
    );

    // 1. 与浏览器一致，先 GET user_article_finish 预热上下文。失败不致命。
    let probe_path = format!(
        "/api/v1/lms/learn/user_article_finish/{leaf_id}/?cid={classroom_id}&sid={sku_id}"
    );
    if let Err(e) = client.get_json_same_origin(&probe_path, &referer).await {
        log::warn!(
            "article user_article_finish probe failed: leaf_id={leaf_id} err={e}"
        );
    }

    // 2. 真正触发完成的调用
    mark_chapter_schedule(client, classroom_id, leaf_id, sku_id, &referer).await
}

fn common_post_headers(client: &XtClient, referer: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::HeaderMap;
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
    h.insert("content-type", "application/json".parse().unwrap());
    h.insert("Origin", "https://www.xuetangx.com".parse().unwrap());
    h.insert("Referer", referer.parse().unwrap());
    if let Some(tok) = client.csrf_token() {
        if let Ok(v) = tok.parse() {
            h.insert("x-csrftoken", v);
        }
    }
    h
}
