use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::client::XtClient;

#[derive(Clone, Debug, Serialize)]
pub struct CourseSummary {
    pub classroom_id: i64,
    pub sku_id: i64,
    pub sign: String,
    pub name: String,
    pub cover: Option<String>,
    pub status: i64,
}

async fn fetch_courses_with_status(
    client: &XtClient,
    status: i64,
) -> Result<Vec<CourseSummary>> {
    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let v = client
            .get_json_with_referer(
                &format!("/api/v1/lms/user/user-courses/?status={status}&page={page}"),
                Some("https://www.xuetangx.com/my-courses/current"),
            )
            .await?;
        let arr = v
            .get("data")
            .and_then(|d| d.get("product_list"))
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        if arr.is_empty() {
            break;
        }
        for p in arr.iter() {
            let classroom_id = p.get("classroom_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let sku_id = p.get("sku_id").and_then(|v| v.as_i64()).unwrap_or(0);
            let sign = p
                .get("course_sign")
                .or_else(|| p.get("sign"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let cover = p
                .get("cover")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if classroom_id != 0 {
                out.push(CourseSummary {
                    classroom_id,
                    sku_id,
                    sign,
                    name,
                    cover,
                    status,
                });
            }
        }
        let count = v
            .get("data")
            .and_then(|d| d.get("count"))
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        if (out.len() as i64) >= count || arr.len() < 10 {
            break;
        }
        page += 1;
        if page > 30 {
            break;
        }
    }
    Ok(out)
}

/// 同时尝试 status=1（进行中）、status=2（已结束）、status=0（全部），
/// 然后按 classroom_id 去重合并。学堂在线不同站点/校园版对 status 的含义不同，
/// 全量尝试一次能避免课程“看不到”的常见问题。
///
/// 当同一 classroom 在多个 status 下都出现时，按优先级保留更"活跃"的状态：
/// 1（进行中）> 2（已结束）> 0（全部/未知），避免 UI 显示错误状态。
pub async fn list_my_courses(client: &XtClient) -> Result<Vec<CourseSummary>> {
    fn status_rank(s: i64) -> i32 {
        match s {
            1 => 3, // 进行中最优
            2 => 2, // 已结束
            0 => 1, // 全部（来源不明）
            _ => 0,
        }
    }
    let mut merged: HashMap<i64, CourseSummary> = HashMap::new();
    for st in [1, 2, 0] {
        match fetch_courses_with_status(client, st).await {
            Ok(list) => {
                for c in list {
                    match merged.get(&c.classroom_id) {
                        Some(existing) if status_rank(existing.status) >= status_rank(c.status) => {
                            // 已有更优 status，跳过
                        }
                        _ => {
                            merged.insert(c.classroom_id, c);
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("拉取课程 status={st} 失败: {e}");
            }
        }
    }
    let mut out: Vec<CourseSummary> = merged.into_values().collect();
    out.sort_by_key(|c| -c.classroom_id);
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct LeafNode {
    pub id: i64,
    pub name: String,
    pub leaf_type: i64,
    pub chapter_path: Vec<String>,
}

pub async fn list_chapters(client: &XtClient, classroom_id: i64, sign: &str) -> Result<Vec<LeafNode>> {
    let v = client
        .get_json(&format!(
            "/api/v1/lms/kg/kg_learn_chapter/?cid={classroom_id}&sign={sign}"
        ))
        .await?;
    let mut out = Vec::new();
    if let Some(root) = v.get("data").and_then(|d| d.get("course_chapter")) {
        walk_chapter(root, &mut Vec::new(), &mut out);
    }
    Ok(out)
}

fn walk_chapter(node: &Value, path: &mut Vec<String>, out: &mut Vec<LeafNode>) {
    let name = node
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    path.push(name);
    if let Some(leafs) = node.get("leaf_list").and_then(|v| v.as_array()) {
        for l in leafs {
            let id = l
                .get("id")
                .or_else(|| l.get("leafinfo_id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if id == 0 {
                continue;
            }
            out.push(LeafNode {
                id,
                name: l
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                leaf_type: l.get("leaf_type").and_then(|v| v.as_i64()).unwrap_or(0),
                chapter_path: path.clone(),
            });
        }
    }
    if let Some(children) = node.get("children").and_then(|v| v.as_array()) {
        for c in children {
            walk_chapter(c, path, out);
        }
    }
    path.pop();
}

pub async fn leaf_info(client: &XtClient, classroom_id: i64, leaf_id: i64, sign: &str) -> Result<Value> {
    let v = client
        .get_json(&format!(
            "/api/v1/lms/learn/leaf_info/{classroom_id}/{leaf_id}/?sign={sign}"
        ))
        .await?;
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

/// 拉取整门课程所有 leaf 的学习进度。返回 `{ leaf_id: rate }`，rate ∈ [0, 1]。
/// rate >= 1 视为已完成。
pub async fn course_schedule(
    client: &XtClient,
    classroom_id: i64,
    sign: &str,
) -> Result<std::collections::HashMap<i64, f64>> {
    let v = client
        .get_json(&format!(
            "/api/v1/lms/learn/course/schedule?cid={classroom_id}&sign={sign}"
        ))
        .await?;
    let mut out = std::collections::HashMap::new();
    if let Some(obj) = v
        .get("data")
        .and_then(|d| d.get("leaf_schedules"))
        .and_then(|x| x.as_object())
    {
        for (k, v) in obj.iter() {
            if let (Ok(id), Some(rate)) = (k.parse::<i64>(), v.as_f64()) {
                out.insert(id, rate);
            }
        }
    }
    Ok(out)
}

/// 从 leaf_info 中递归找 exercise_id。
/// 学堂在线返回结构里 `data.content_info.leaf_type_id` 是 exercise_id，
/// 也兼容 `content.exercise_id` / `exercise_id` 等历史字段名。
fn extract_exercise_id(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::Object(map) => {
            for key in ["exercise_id", "leaf_type_id"] {
                if let Some(id) = map.get(key).and_then(|x| x.as_i64()) {
                    if id > 0 {
                        return Some(id);
                    }
                }
            }
            for (_, child) in map.iter() {
                if let Some(found) = extract_exercise_id(child) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for child in arr {
                if let Some(found) = extract_exercise_id(child) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// 并行预取多个 leaf 的 exercise_id，返回 `{leaf_id: exercise_id}`。
/// 没找到 exercise_id 的 leaf 不会出现在结果里。
pub async fn batch_exercise_ids(
    client: std::sync::Arc<XtClient>,
    classroom_id: i64,
    sign: String,
    leaf_ids: Vec<i64>,
) -> Result<std::collections::HashMap<i64, i64>> {
    let mut handles = Vec::new();
    for id in leaf_ids {
        let c = client.clone();
        let s = sign.clone();
        handles.push(tokio::spawn(async move {
            let info = leaf_info(&c, classroom_id, id, &s).await.ok()?;
            extract_exercise_id(&info).map(|ex| (id, ex))
        }));
    }
    let mut out = std::collections::HashMap::new();
    for h in handles {
        if let Ok(Some((id, ex))) = h.await {
            out.insert(id, ex);
        }
    }
    Ok(out)
}

/// 给定一组 (leaf_id, exercise_id, sku_id)，并行拉取每个习题集的题型计数。
/// 返回 `{ leaf_id: { kind: count } }`，kind 用 ProblemKind 的 snake_case 字符串。
pub async fn batch_exercise_kinds(
    client: std::sync::Arc<XtClient>,
    sku_id: i64,
    items: Vec<(i64, i64)>,
) -> Result<std::collections::HashMap<i64, std::collections::HashMap<String, i64>>> {
    use crate::exercise::{fetch_exercise, ProblemKind};
    let mut handles = Vec::new();
    for (leaf_id, ex_id) in items {
        let c = client.clone();
        handles.push(tokio::spawn(async move {
            let list = fetch_exercise(&c, ex_id, sku_id).await.ok()?;
            let mut counts: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();
            for p in list.problems.iter() {
                let key = match p.kind {
                    ProblemKind::SingleChoice => "single_choice",
                    ProblemKind::MultipleChoice => "multiple_choice",
                    ProblemKind::Judgement => "judgement",
                    ProblemKind::Completion => "completion",
                    ProblemKind::Subjective => "subjective",
                    ProblemKind::Other => "other",
                };
                *counts.entry(key.to_string()).or_insert(0) += 1;
            }
            Some((leaf_id, counts))
        }));
    }
    let mut out = std::collections::HashMap::new();
    for h in handles {
        if let Ok(Some((id, c))) = h.await {
            out.insert(id, c);
        }
    }
    Ok(out)
}
