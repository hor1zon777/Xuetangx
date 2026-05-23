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

/// 课程总分概要：`get_evaluation_detail` 接口顶层的 `total_score_and_schedule`，
/// 也是单独 `user-score` 接口返回的同一个对象。字段含义：
/// - `user_score`：当前总分（满分 100）
/// - `pass_line`：及格线（null 表示该课程无明示及格线）
/// - `title`：当前等级（F/D/C/B/A/P 等）
/// - `higher_title`：下一个等级
/// - `lack_score`：距下一个等级还差的分数
#[derive(Clone, Debug, Serialize)]
pub struct EvaluationTotal {
    pub user_score: f64,
    pub pass_line: Option<f64>,
    pub title: String,
    pub higher_title: String,
    pub lack_score: f64,
}

/// 单个 leaf 在成绩中的呈现：所属分类、本 leaf 实际得分 / 满分 / 完成度。
#[derive(Clone, Debug, Serialize)]
pub struct EvaluationLeaf {
    pub leaf_id: i64,
    pub leaf_name: String,
    /// 学堂 leaf_type 数字（0=视频、3=图文、4=讨论、6=习题…）
    pub leaf_type: i64,
    /// 该 leaf 的完成度（0~1）。schedule >= 1 才计满分。
    pub schedule: f64,
    /// 用户在该 leaf 上拿到的分数（已折算到 100 分制中本 leaf 应得的份额）。
    pub user_score: f64,
    /// 该 leaf 的满分（折算到 100 分制中本 leaf 应得的份额）。
    pub leaf_score: f64,
    /// 章节路径，前端展示用。
    pub chapter_path: Vec<String>,
}

/// 一个评分大类（视频/图文/讨论/作业/考试）的明细。
#[derive(Clone, Debug, Serialize)]
pub struct EvaluationCategory {
    /// 接口里返回的 evaluation_id 是字符串（比如 "11"），统一转 i64 方便前端用。
    pub evaluation_id: i64,
    pub evaluation_name: String,
    /// 该分类总满分（折算到 100 分制后的份额，例如 "作业"=20）。
    pub evaluation_score: f64,
    /// 同上但以百分数表示（20 表示 20%）。
    pub proportion: f64,
    /// 该分类已完成度（0~1）。
    pub schedule: f64,
    /// 该分类当前总得分（折算到 100 分制后的份额）。
    pub use_evaluation_score: f64,
    /// 该分类下所有 leaf 的明细（已扁平化、按章节合并）。
    pub leaves: Vec<EvaluationLeaf>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationDetail {
    pub total: EvaluationTotal,
    pub categories: Vec<EvaluationCategory>,
}

/// 解析 `get_evaluation_detail.data` JSON 为结构化的 [`EvaluationDetail`]。
/// 字段缺失/类型异常时退化为 0/空字符串，不抛错——这个接口主要给 UI 用，让前端
/// 自己挑选感兴趣的列展示比抛错更友好。
pub async fn course_evaluation_detail(
    client: &XtClient,
    classroom_id: i64,
    sign: &str,
) -> Result<EvaluationDetail> {
    let v = client
        .get_json(&format!(
            "/api/v1/lms/learn/get_evaluation_detail/?sign={sign}&cid={classroom_id}"
        ))
        .await?;
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    let total = parse_total(&data);
    let categories = parse_categories(&data);
    Ok(EvaluationDetail { total, categories })
}

fn parse_total(data: &Value) -> EvaluationTotal {
    let t = data
        .get("total_score_and_schedule")
        .cloned()
        .unwrap_or(Value::Null);
    EvaluationTotal {
        user_score: t
            .get("user_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        // pass_line 学堂可能返回 null / 数字 / 字符串数字
        pass_line: t
            .get("pass_line")
            .and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))),
        title: t
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        higher_title: t
            .get("higher_title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        lack_score: t
            .get("lack_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    }
}

fn parse_categories(data: &Value) -> Vec<EvaluationCategory> {
    let Some(arr) = data.get("score_detail").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .map(|cat| EvaluationCategory {
            // evaluation_id 在原始 JSON 里是字符串
            evaluation_id: cat
                .get("evaluation_id")
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<i64>().ok())
                        .or_else(|| v.as_i64())
                })
                .unwrap_or(0),
            evaluation_name: cat
                .get("evaluation_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            evaluation_score: cat
                .get("evaluation_score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            proportion: cat
                .get("proportion")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            schedule: cat.get("schedule").and_then(|v| v.as_f64()).unwrap_or(0.0),
            use_evaluation_score: cat
                .get("use_evaluation_score")
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0.0),
            leaves: parse_category_leaves(cat),
        })
        .collect()
}

/// 一个分类下面是 `resource: [章节...]`，每个章节有 `leaf_list` 与可能的 `section_list`
/// （二级章节），section_list 里再嵌套 leaf_list / section_list。这里按 DFS 扁平化，
/// 每个 leaf 把祖先章节名一并保留下来，方便前端按章节排序/聚合。
fn parse_category_leaves(cat: &Value) -> Vec<EvaluationLeaf> {
    let mut out = Vec::new();
    let Some(resources) = cat.get("resource").and_then(|v| v.as_array()) else {
        return out;
    };
    for r in resources {
        walk_eval_resource(r, &mut Vec::new(), &mut out);
    }
    out
}

fn walk_eval_resource(node: &Value, path: &mut Vec<String>, out: &mut Vec<EvaluationLeaf>) {
    let name = node
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let pushed = !name.is_empty();
    if pushed {
        path.push(name);
    }
    if let Some(leafs) = node.get("leaf_list").and_then(|v| v.as_array()) {
        for l in leafs {
            let leaf_id = l
                .get("id")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    l.get("score_info")
                        .and_then(|s| s.get("leaf_id"))
                        .and_then(|v| v.as_i64())
                })
                .unwrap_or(0);
            if leaf_id == 0 {
                continue;
            }
            let score_info = l.get("score_info");
            out.push(EvaluationLeaf {
                leaf_id,
                leaf_name: l
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                leaf_type: l.get("type").and_then(|v| v.as_i64()).unwrap_or(0),
                schedule: l.get("schedule").and_then(|v| v.as_f64()).unwrap_or(0.0),
                user_score: score_info
                    .and_then(|s| s.get("user_score"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                leaf_score: score_info
                    .and_then(|s| s.get("leaf_score"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                chapter_path: path.clone(),
            });
        }
    }
    if let Some(sections) = node.get("section_list").and_then(|v| v.as_array()) {
        for s in sections {
            walk_eval_resource(s, path, out);
        }
    }
    if pushed {
        path.pop();
    }
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

/// 给定一组 (leaf_id, exercise_id)，并行拉取每个习题集的题型计数。
/// 返回 `{ leaf_id: { kind: count } }`，kind 用 ProblemKind 的 snake_case 字符串。
///
/// 注意：参数是 **sku_id** 不是 classroom_id —— 学堂在线
/// `/get_exercise_list/{exercise_id}/{sku_id}/` 的第二段是 sku_id。
pub async fn batch_exercise_kinds(
    client: std::sync::Arc<XtClient>,
    classroom_id: i64,
    sign: String,
    sku_id: i64,
    items: Vec<(i64, i64)>,
) -> Result<std::collections::HashMap<i64, std::collections::HashMap<String, i64>>> {
    use crate::exercise::{fetch_exercise_with_referer, warm_exercise_context, ProblemKind};
    let mut handles = Vec::new();
    for (leaf_id, ex_id) in items {
        let c = client.clone();
        let s = sign.clone();
        handles.push(tokio::spawn(async move {
            let referer =
                format!("https://www.xuetangx.com/learn/space/{s}/{s}/{classroom_id}/exercise/{leaf_id}");
            warm_exercise_context(&c, leaf_id, classroom_id, sku_id, &s, &referer).await;
            let list = fetch_exercise_with_referer(&c, ex_id, sku_id, Some(&referer))
                .await
                .ok()?;
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
