use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::ai::{ask_ai_with_retry, AnswerSpec, ProblemForAi};
use crate::client::XtClient;
use crate::state::AiSettings;

/// 学堂在线题型枚举。基于 HAR 抓包 + 学堂在线公开文档归纳。
/// 数值取自 ProblemType 字段；name 取自 Type；label 取自 TypeText（中文）。
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// 1 / SingleChoice
    SingleChoice,
    /// 2 / MultipleChoice
    MultipleChoice,
    /// 3 / Completion / 填空题
    Completion,
    /// 4 / Subjective / 主观题/简答题
    Subjective,
    /// 6 / Judgement / 判断题
    Judgement,
    /// 其它未知题型（投票、排序、连线等）
    Other,
}

impl ProblemKind {
    pub fn from_meta(type_str: Option<&str>, problem_type: Option<i64>) -> Self {
        if let Some(s) = type_str {
            match s {
                "SingleChoice" => return Self::SingleChoice,
                "MultipleChoice" => return Self::MultipleChoice,
                "Judgement" => return Self::Judgement,
                "Completion" | "FillBlank" => return Self::Completion,
                "Subjective" | "ShortAnswer" | "Essay" => return Self::Subjective,
                _ => {}
            }
        }
        match problem_type.unwrap_or(-1) {
            1 => Self::SingleChoice,
            2 => Self::MultipleChoice,
            3 => Self::Completion,
            4 => Self::Subjective,
            6 => Self::Judgement,
            _ => Self::Other,
        }
    }

    /// 该题型是否为"选项类"（单选/多选/判断）—— 答案是选项 key 数组。
    pub fn is_choice(&self) -> bool {
        matches!(self, Self::SingleChoice | Self::MultipleChoice | Self::Judgement)
    }

    /// 该题型是否需要用文本作答（填空、主观题）。
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Completion | Self::Subjective)
    }

    /// 用于 AI 提示词的中文标签
    pub fn label_zh(&self) -> &'static str {
        match self {
            Self::SingleChoice => "单选题",
            Self::MultipleChoice => "多选题",
            Self::Judgement => "判断题",
            Self::Completion => "填空题",
            Self::Subjective => "主观题",
            Self::Other => "其它题型",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Problem {
    pub problem_id: i64,
    pub problem_type: i64,
    pub problem_type_text: String,
    /// 规范化的题型枚举（前端按它显示徽章 + 决定答题策略）
    pub kind: ProblemKind,
    pub body_html: String,
    pub options: Vec<ProblemOption>,
    /// 该小题是否已提交/已完成（无需再次作答）
    pub submitted: bool,
    /// 已提交时服务端返回的得分（仅 submitted=true 时有意义）
    pub my_score: Option<f64>,
    /// 已提交时服务端返回的正确性（仅 submitted=true 时有意义）
    pub is_right: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProblemOption {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExerciseList {
    pub exercise_id: i64,
    pub problems: Vec<Problem>,
}

fn value_as_f64(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

fn value_has_non_empty_answer(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Array(arr)) => !arr.is_empty(),
        Some(Value::Object(obj)) => !obj.is_empty(),
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptchaChallenge {
    pub captcha_appid: String,
    pub exercise_id: i64,
    pub sku_id: i64,
    pub referer: Option<String>,
    pub msg: String,
    pub error_code: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaptchaInfo {
    pub required: bool,
    pub captcha_appid: String,
    pub exercise_id: i64,
    pub sku_id: i64,
    pub referer: Option<String>,
    pub msg: String,
    pub error_code: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExerciseProbe {
    pub blocked: bool,
    pub captcha: Option<CaptchaInfo>,
    pub list: Option<ExerciseList>,
}

impl std::fmt::Display for CaptchaChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "触发滑块风控，需要完成验证码后重试（captcha_appid={}, exercise_id={}, sku_id={}, msg={}, error_code={}）",
            self.captcha_appid, self.exercise_id, self.sku_id, self.msg, self.error_code
        )
    }
}

impl std::error::Error for CaptchaChallenge {}

pub async fn fetch_exercise(
    client: &XtClient,
    exercise_id: i64,
    sku_id: i64,
) -> Result<ExerciseList> {
    fetch_exercise_with_referer(client, exercise_id, sku_id, None).await
}

pub async fn fetch_exercise_with_referer(
    client: &XtClient,
    exercise_id: i64,
    sku_id: i64,
    referer: Option<&str>,
) -> Result<ExerciseList> {
    fetch_exercise_with_captcha(client, exercise_id, sku_id, referer, None, None).await
}

pub async fn fetch_exercise_with_captcha(
    client: &XtClient,
    exercise_id: i64,
    sku_id: i64,
    referer: Option<&str>,
    ticket: Option<&str>,
    randstr: Option<&str>,
) -> Result<ExerciseList> {
    // 学堂在线 `get_exercise_list` 的 URL 第二段是 **sku_id**，不是 classroom_id。
    // HAR 对比：
    //   GET /api/v1/lms/exercise/get_exercise_list/6758621/14953216/
    //   Referer: /learn/space/.../29605421/exercise/76044797
    // 其中 29605421 是 classroom_id（提交 problem_apply 仍然要用它），14953216 才是
    // 拉题列表接口需要的 sku_id。误传 classroom_id 时服务端会 200 返回 `{"data": {}}`。
    let mut path =
        format!("/api/v1/lms/exercise/get_exercise_list/{exercise_id}/{sku_id}/");
    if let (Some(ticket), Some(randstr)) = (ticket, randstr) {
        path.push('?');
        path.push_str("ticket=");
        path.push_str(&urlencoding::encode(ticket));
        path.push_str("&randstr=");
        path.push_str(&urlencoding::encode(randstr));
    }
    let v = if let Some(r) = referer {
        client.get_json_same_origin(&path, r).await?
    } else {
        client.get_json(&path).await?
    };

    if is_captcha_blocked(&v) {
        let msg = v
            .get("msg")
            .and_then(|x| x.as_str())
            .unwrap_or("request blocked")
            .to_string();
        let error_code = v.get("error_code").and_then(|x| x.as_i64()).unwrap_or(400403);
        bail!(CaptchaChallenge {
            captcha_appid: "197282031".to_string(),
            exercise_id,
            sku_id,
            referer: referer.map(|s| s.to_string()),
            msg,
            error_code,
        });
    }

    let d = v.get("data").ok_or_else(|| anyhow!("exercise 缺 data"))?;
    // 题目数组学堂在线**目前**用 `problems` 字段，但历史/某些课程下也见过其它形态
    // （例如包装在 problem_list、topic、按 section 分组的对象等）。这里先按主路径取，
    // 拿不到再尝试已知的几种备选路径；都失败就把响应预览塞到错误里向上抛，
    // 而不是悄悄返回空 Vec —— 否则前端只会看到"得分 0 · 0/0 正确"这种不知所云的成功态。
    let problems_raw: Vec<Value> = {
        if let Some(arr) = d.get("problems").and_then(|v| v.as_array()) {
            arr.clone()
        } else if let Some(arr) = d.get("problem_list").and_then(|v| v.as_array()) {
            arr.clone()
        } else if let Some(arr) = d.get("topic").and_then(|v| v.as_array()) {
            arr.clone()
        } else {
            // 兜底：如果 data.problems 是按 section 分组的对象（例如 {"sec_1": [...]})，
            // 把它们 flatten 成一个数组。这里仅在主字段缺失时才走这条分支。
            let mut flat: Vec<Value> = Vec::new();
            if let Some(obj) = d.get("problems").and_then(|v| v.as_object()) {
                for (_k, val) in obj {
                    if let Some(arr) = val.as_array() {
                        flat.extend(arr.iter().cloned());
                    }
                }
            }
            flat
        }
    };

    if problems_raw.is_empty() {
        // 把响应顶部 600 字记到日志 + 错误信息，方便定位真实字段路径
        let preview = serde_json::to_string(d).unwrap_or_default();
        let head: String = preview.chars().take(600).collect();
        let cookie_names = client
            .cookies
            .lock()
            .map(|store| {
                let mut names: Vec<String> =
                    store.iter_any().map(|c| c.name().to_string()).collect();
                names.sort();
                names.dedup();
                names.join(",")
            })
            .unwrap_or_else(|_| "<cookie-lock-failed>".to_string());
        log::warn!(
            "fetch_exercise 解析到 0 道题: exercise_id={} sku_id={} referer={} cookies=[{}] preview={}",
            exercise_id,
            sku_id,
            referer.unwrap_or(""),
            cookie_names,
            head
        );
        return Err(anyhow!(
            "习题响应里没有 problems 数据（exercise_id={exercise_id}, sku_id={sku_id}）。响应预览：{head}"
        ));
    }
    let mut problems = Vec::new();
    for p in problems_raw {
        let content = p.get("content").cloned().unwrap_or(Value::Null);
        let problem_id = p
            .get("problem_id")
            .and_then(|v| v.as_i64())
            .or_else(|| content.get("ProblemID").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        let problem_type = content
            .get("ProblemType")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let type_str = content.get("Type").and_then(|v| v.as_str());
        let kind = ProblemKind::from_meta(type_str, Some(problem_type));
        let problem_type_text = content
            .get("TypeText")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| kind.label_zh().to_string());
        let body_html = content
            .get("Body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let options = content
            .get("Options")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|o| {
                Some(ProblemOption {
                    key: o.get("key").and_then(|v| v.as_str())?.to_string(),
                    value: o.get("value").and_then(|v| v.as_str())?.to_string(),
                })
            })
            .collect();

        // 检测该“小题”是否已作答且有分数。
        //
        // HAR 实测已批改小题形态：
        //   user: {
        //     my_score: "1.00",      // 注意：有时是字符串，不是数字
        //     is_right: true/false,
        //     answer: ["A"],
        //     my_answer: ["A"],
        //     my_count: 1
        //   }
        //
        // 需求：已经作答有分数的小题跳过；仅“作答了但没有分数”的小题继续处理。
        // 因此跳过条件不能只看整套题/父题状态，也不能误用 `score`（题目总分）。
        // 现行规则：
        //   - 当前小题 user.my_score / 顶层 my_score 能解析出分数
        //   - 且当前小题有明确作答痕迹（answer/my_answer 非空，或 my_count/count > 0）
        //   => 跳过
        //   - 有作答但没有 my_score => 不跳过，继续问 AI + 提交
        let user = p.get("user");
        let server_score = p
            .get("my_score")
            .and_then(value_as_f64)
            .or_else(|| user.and_then(|u| u.get("my_score")).and_then(value_as_f64));
        let server_is_right = p
            .get("is_right")
            .and_then(|v| v.as_bool())
            .or_else(|| user.and_then(|u| u.get("is_right")).and_then(|v| v.as_bool()));
        let has_answer = value_has_non_empty_answer(p.get("answer"))
            || value_has_non_empty_answer(user.and_then(|u| u.get("answer")))
            || value_has_non_empty_answer(user.and_then(|u| u.get("my_answer")))
            || p.get("my_count").and_then(|v| v.as_i64()).unwrap_or(0) > 0
            || user.and_then(|u| u.get("my_count")).and_then(|v| v.as_i64()).unwrap_or(0) > 0
            || p.get("count").and_then(|v| v.as_i64()).unwrap_or(0) > 0
            || user.and_then(|u| u.get("count")).and_then(|v| v.as_i64()).unwrap_or(0) > 0;
        let submitted = has_answer && server_score.is_some();

        problems.push(Problem {
            problem_id,
            problem_type,
            problem_type_text,
            kind,
            body_html,
            options,
            submitted,
            my_score: server_score,
            is_right: server_is_right,
        });
    }
    Ok(ExerciseList {
        exercise_id,
        problems,
    })
}

fn is_captcha_blocked(v: &Value) -> bool {
    v.get("error_code").and_then(|x| x.as_i64()) == Some(400403)
        || v.get("msg").and_then(|x| x.as_str()) == Some("request blocked")
}

pub async fn probe_exercise_with_captcha(
    client: &XtClient,
    exercise_id: i64,
    sku_id: i64,
    referer: Option<&str>,
) -> Result<ExerciseProbe> {
    match fetch_exercise_with_referer(client, exercise_id, sku_id, referer).await {
        Ok(list) => Ok(ExerciseProbe {
            blocked: false,
            captcha: None,
            list: Some(list),
        }),
        Err(e) => {
            if let Some(ch) = e.downcast_ref::<CaptchaChallenge>() {
                Ok(ExerciseProbe {
                    blocked: true,
                    captcha: Some(CaptchaInfo {
                        required: true,
                        captcha_appid: ch.captcha_appid.clone(),
                        exercise_id: ch.exercise_id,
                        sku_id: ch.sku_id,
                        referer: ch.referer.clone(),
                        msg: ch.msg.clone(),
                        error_code: ch.error_code,
                    }),
                    list: None,
                })
            } else {
                Err(e)
            }
        }
    }
}

pub async fn warm_exercise_context(
    client: &XtClient,
    leaf_id: i64,
    classroom_id: i64,
    sku_id: i64,
    sign: &str,
    referer: &str,
) {
    // Match the browser sequence before get_exercise_list:
    //   1. leaf_info for the current exercise leaf
    //   2. chapter/schedule post
    //   3. get_evaluation_detail
    //
    // These calls are best-effort.  Some sessions return `data: {}` from
    // get_exercise_list unless the current learn-space/evaluation context has been
    // initialized first.
    let leaf_path =
        format!("/api/v1/lms/learn/leaf_info/{classroom_id}/{leaf_id}/?sign={sign}");
    if let Err(e) = client.get_json_same_origin(&leaf_path, referer).await {
        log::warn!("warm exercise leaf_info failed: leaf_id={leaf_id} err={e}");
    }

    let schedule_body = json!({
        "leaf_id": leaf_id,
        "classroom_id": classroom_id,
        "sku_id": sku_id,
    });
    if let Err(e) = client
        .post_json_with_referer("/api/v1/lms/learn/chapter/schedule", &schedule_body, referer)
        .await
    {
        log::warn!("warm exercise chapter/schedule failed: leaf_id={leaf_id} err={e}");
    }

    let eval_path =
        format!("/api/v1/lms/learn/get_evaluation_detail/?sign={sign}&cid={classroom_id}");
    if let Err(e) = client.get_json_same_origin(&eval_path, referer).await {
        log::warn!("warm exercise get_evaluation_detail failed: leaf_id={leaf_id} err={e}");
    }
}

pub async fn submit_problem(
    client: &XtClient,
    leaf_id: i64,
    classroom_id: i64,
    exercise_id: i64,
    problem_id: i64,
    sign: &str,
    answer: Vec<String>,
    answers: Value,
) -> Result<Value> {
    let body = json!({
        "leaf_id": leaf_id,
        "classroom_id": classroom_id,
        "exercise_id": exercise_id,
        "problem_id": problem_id,
        "sign": sign,
        "answers": answers,
        "answer": answer
    });
    let v = client
        .post_json("/api/v1/lms/exercise/problem_apply/", &body)
        .await?;
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

/// 自动跑一整套习题：取题目 → 询 AI → 提交。
/// 返回每个题目的结构化结果，前端可按 kind 分组统计。
///
/// `on_progress` 回调用于实时上报进度，调用方（commands 层）负责把它转成 tauri 事件。
/// 阶段约定：
///   - "start"     info = { problem_id, kind, kind_label, index, total }
///   - "skipped"   info = { problem_id, kind, index, my_score, is_right }
///   - "asking_ai" info = { problem_id, kind, index }
///   - "submitting" info = { problem_id, kind, index, answer_text }
///   - "item_done" info = { problem_id, kind, index, result }   // result 即最终回传给前端的整条记录
///   - "done"      info = { total }
pub async fn auto_run_exercise(
    client: &XtClient,
    ai: &AiSettings,
    leaf_id: i64,
    classroom_id: i64,
    sku_id: i64,
    exercise_id: i64,
    sign: &str,
    on_progress: &(dyn Fn(&str, Value) + Send + Sync),
) -> Result<Vec<Value>> {
    auto_run_exercise_with_captcha(
        client,
        ai,
        leaf_id,
        classroom_id,
        sku_id,
        exercise_id,
        sign,
        None,
        None,
        on_progress,
    )
    .await
}

pub async fn auto_run_exercise_with_captcha(
    client: &XtClient,
    ai: &AiSettings,
    leaf_id: i64,
    classroom_id: i64,
    sku_id: i64,
    exercise_id: i64,
    sign: &str,
    ticket: Option<&str>,
    randstr: Option<&str>,
    on_progress: &(dyn Fn(&str, Value) + Send + Sync),
) -> Result<Vec<Value>> {
    let referer =
        format!("https://www.xuetangx.com/learn/space/{sign}/{sign}/{classroom_id}/exercise/{leaf_id}");
    warm_exercise_context(client, leaf_id, classroom_id, sku_id, sign, &referer).await;
    let list = fetch_exercise_with_captcha(
        client,
        exercise_id,
        sku_id,
        Some(&referer),
        ticket,
        randstr,
    )
    .await?;
    let total = list.problems.len();
    let mut results = Vec::with_capacity(total);
    for (index, p) in list.problems.iter().enumerate() {
        let kind_label = p.kind.label_zh();

        on_progress(
            "start",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
                "total": total,
            }),
        );

        // 已提交/已完成的小题直接跳过，记录状态不重复提交
        if p.submitted {
            let result = json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "skipped": true,
                "submitted": true,
                "submit": {
                    "is_right": p.is_right,
                    "my_score": p.my_score
                }
            });
            on_progress(
                "skipped",
                json!({
                    "problem_id": p.problem_id,
                    "kind": p.kind,
                    "index": index,
                    "my_score": p.my_score,
                    "is_right": p.is_right,
                }),
            );
            on_progress(
                "item_done",
                json!({
                    "problem_id": p.problem_id,
                    "kind": p.kind,
                    "index": index,
                    "result": result.clone(),
                }),
            );
            results.push(result);
            continue;
        }

        on_progress(
            "asking_ai",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
            }),
        );

        let pa = ProblemForAi {
            type_text: p.problem_type_text.clone(),
            body_html: p.body_html.clone(),
            options: p
                .options
                .iter()
                .map(|o| (o.key.clone(), o.value.clone()))
                .collect(),
        };
        let spec = match ask_ai_with_retry(ai, &pa).await {
            Ok(s) => s,
            Err(e) => {
                let result = json!({
                    "problem_id": p.problem_id,
                    "kind": p.kind,
                    "kind_label": kind_label,
                    "error": format!("AI 询问失败: {e}")
                });
                on_progress(
                    "item_done",
                    json!({
                        "problem_id": p.problem_id,
                        "kind": p.kind,
                        "index": index,
                        "result": result.clone(),
                    }),
                );
                results.push(result);
                continue;
            }
        };
        // 学堂在线提交规范：
        // - 选项题（单选/多选/判断）：answer = ["A","B"...]，answers = {}
        // - 文本题（填空/主观）：answer = []，answers = { problem_id: text }
        //   注：填空若有多空，AI 用 ## 分隔，这里按需要可继续拆分但通常单字段也接受。
        let (answer_arr, answers_obj, ui_answer): (Vec<String>, Value, String) = match &spec {
            AnswerSpec::Choice(keys) => (keys.clone(), json!({}), keys.join("")),
            AnswerSpec::Text(t) => (
                Vec::new(),
                json!({ p.problem_id.to_string(): t }),
                t.clone(),
            ),
        };

        on_progress(
            "submitting",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
                "answer_text": ui_answer.clone(),
            }),
        );

        let result = match submit_problem(
            client,
            leaf_id,
            classroom_id,
            exercise_id,
            p.problem_id,
            sign,
            answer_arr.clone(),
            answers_obj,
        )
        .await
        {
            Ok(resp) => json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "answer": answer_arr,
                "answer_text": ui_answer,
                "submit": resp
            }),
            Err(e) => json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "answer": answer_arr,
                "answer_text": ui_answer,
                "error": format!("提交失败: {e}")
            }),
        };

        on_progress(
            "item_done",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "index": index,
                "result": result.clone(),
            }),
        );
        results.push(result);
    }
    on_progress("done", json!({ "total": total }));
    Ok(results)
}
