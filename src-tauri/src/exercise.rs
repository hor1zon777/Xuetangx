use anyhow::{anyhow, bail, Result};
use parking_lot::RwLock;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

use crate::ai::{ask_ai_with_retry, AnswerSpec, ProblemForAi};
use crate::bank::{entry_to_submit_payload, Bank};
use crate::client::XtClient;
use crate::state::AiSettings;

/// 自动作业提交节流配置。第一题不延迟，后续每题从 [min_ms, max_ms] 区间均匀采样。
/// 用于规避学堂在线对"瞬时多次提交"的风控。
#[derive(Clone, Copy, Debug)]
pub struct SubmitDelay {
    pub min_ms: u64,
    pub max_ms: u64,
}

impl SubmitDelay {
    /// 默认范围：2500–4000ms。提到 2500 是因为 1500ms 在密集提交时仍会被学堂在线
    /// 偶发 429 限流；2500 是经验上比较稳的下界，且对学习体验影响很小。
    pub const DEFAULT_MIN_MS: u64 = 2500;
    pub const DEFAULT_MAX_MS: u64 = 4000;

    pub const fn defaults() -> Self {
        Self {
            min_ms: Self::DEFAULT_MIN_MS,
            max_ms: Self::DEFAULT_MAX_MS,
        }
    }

    /// 从设置里两个可选字段构造延迟配置；缺省回落到默认。
    /// 若上界小于下界，把上界拉到下界，保证 `pick` 不会 panic。
    pub fn from_settings(min: Option<u64>, max: Option<u64>) -> Self {
        let min_ms = min.unwrap_or(Self::DEFAULT_MIN_MS);
        let max_ms = max.unwrap_or(Self::DEFAULT_MAX_MS).max(min_ms);
        Self { min_ms, max_ms }
    }

    /// 在 [min_ms, max_ms] 上均匀采样一个时长。
    pub fn pick(&self) -> Duration {
        if self.max_ms <= self.min_ms {
            Duration::from_millis(self.min_ms)
        } else {
            let mut rng = rand::thread_rng();
            let ms = rng.gen_range(self.min_ms..=self.max_ms);
            Duration::from_millis(ms)
        }
    }
}

impl Default for SubmitDelay {
    fn default() -> Self {
        Self::defaults()
    }
}

/// 学堂在线题型枚举。基于 HAR 抓包 + 学堂在线公开文档归纳。
/// 数值取自 ProblemType 字段；name 取自 Type；label 取自 TypeText（中文）。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Copy)]
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
    /// 学堂服务端返回的标准答案（选项题：key 数组；判断题：["true"]/["false"]）。
    /// 仅在该小题"已批改"（submitted=true 且响应里给了 answer 字段）时有值。
    /// 这是本地题库的唯一可信来源；AI 答出的内容不会写入此字段。
    pub correct_answer: Option<Vec<String>>,
    /// 文本类（填空/主观）的标准答案文本。多个空位按学堂原样保留（通常以 ## 或换行分隔）。
    pub correct_answer_text: Option<String>,
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

/// 从已批改小题的响应里提取"标准答案"。
///
/// 学堂 `/get_exercise_list` 对已批改小题会在以下位置之一给出正确答案：
///   - 顶层 `answer`（最常见，数组形式）
///   - `user.answer`（兼容形态）
///
/// 这里**不**取 `my_answer` / `user.my_answer` —— 那是"我自己的答案"，
/// 我答错时它会和正确答案不一致。本地题库只接受确定正确的答案，
/// 所以仅当 `answer` 字段存在且与 `my_answer` 同源/独立时才作数。
///
/// 返回 (choice_keys, text_answer)：
///   - 选项题：answer 是数组 → choice_keys = Some(["A","C"])
///   - 文本题：answer 是字符串 / 多空数组 → text_answer = Some("xxx")
fn extract_correct_answer(
    p: &Value,
    kind: ProblemKind,
) -> (Option<Vec<String>>, Option<String>) {
    let user = p.get("user");
    let candidate = p
        .get("answer")
        .filter(|v| !matches!(v, Value::Null))
        .or_else(|| user.and_then(|u| u.get("answer")))
        .filter(|v| !matches!(v, Value::Null));
    let Some(ans) = candidate else {
        return (None, None);
    };

    if kind.is_choice() {
        // 选项题：期望是数组 ["A","C"]；兼容字符串 "AC" / "A,C"
        if let Some(arr) = ans.as_array() {
            let keys: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect();
            if keys.is_empty() {
                (None, None)
            } else {
                (Some(keys), None)
            }
        } else if let Some(s) = ans.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                (None, None)
            } else if trimmed == "true" || trimmed == "false" {
                // 判断题字符串形态
                (Some(vec![trimmed.to_string()]), None)
            } else {
                let keys: Vec<String> = trimmed
                    .split(|c: char| c == ',' || c == '|' || c.is_whitespace())
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                if keys.is_empty() {
                    (None, None)
                } else {
                    (Some(keys), None)
                }
            }
        } else {
            (None, None)
        }
    } else {
        // 文本题：期望是字符串；兼容数组 ["空1","空2"] → 用 ## 拼回
        if let Some(s) = ans.as_str() {
            let t = s.trim().to_string();
            if t.is_empty() { (None, None) } else { (None, Some(t)) }
        } else if let Some(arr) = ans.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .collect();
            if parts.is_empty() {
                (None, None)
            } else {
                (None, Some(parts.join("##")))
            }
        } else {
            (None, None)
        }
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

        // 只有"已批改"的小题才提取标准答案——未提交时学堂不会下发 answer，
        // 提取出来的是脏数据。后续题库写入也只读这两个字段。
        let (correct_answer, correct_answer_text) = if submitted {
            extract_correct_answer(&p, kind)
        } else {
            (None, None)
        };

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
            correct_answer,
            correct_answer_text,
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
    // 旧外部签名：成功返回 data，失败返回 Err。
    // 内部走 submit_problem_detailed 以复用"是否被接受"的判定逻辑。
    let outcome = submit_problem_detailed(
        client,
        leaf_id,
        classroom_id,
        exercise_id,
        problem_id,
        sign,
        answer,
        answers,
    )
    .await?;
    if outcome.is_accepted {
        Ok(outcome.data.unwrap_or(Value::Null))
    } else {
        Err(anyhow!(
            "提交未被学堂接受 (status={}, reason={}, body={})",
            outcome.status,
            outcome.reason,
            outcome
                .raw_body
                .chars()
                .take(300)
                .collect::<String>()
        ))
    }
}

/// `submit_problem_detailed` 的结构化返回。
/// is_accepted=true 才视为成功；为 false 时 reason 给出失败原因（用于事件/日志/重试决策）。
#[derive(Clone, Debug)]
pub struct SubmitOutcome {
    pub status: u16,
    pub raw_body: String,
    /// 完整 JSON（若 body 能解析为 JSON）。学堂在线一般返回 { success, msg, data }。
    pub full: Option<Value>,
    /// 学堂在线惯例：成功时 full.data 是一个含 is_right / my_score 的对象。
    pub data: Option<Value>,
    /// "被学堂接受"——见 [`SubmitOutcome::judge`] 的判定规则。
    pub is_accepted: bool,
    /// 失败原因短代码：rate_limited / http_error / parse_error / server_rejected / missing_grade / network_error。
    pub reason: &'static str,
    /// 是否限流（status==429 或 body 内出现典型限流提示）。专门拎出来供上层决定额外等多久。
    pub rate_limited: bool,
}

impl SubmitOutcome {
    /// 学堂在线 problem_apply 响应被认为"已被接受"的条件：
    /// 1. HTTP 200
    /// 2. body 是合法 JSON
    /// 3. 顶层 success 不能显式是 false
    /// 4. data 是对象，且至少出现以下任一字段：is_right (bool) / my_score (number) /
    ///    right_answer (any) / submit_answer (any) —— 任一字段都意味着学堂确实批改过本次提交。
    fn judge(status: u16, raw_body: &str) -> (bool, Option<Value>, Option<Value>, &'static str, bool) {
        // 1. 429 直接判定限流，不读 body（即便 body 是空也算）。
        if status == 429 {
            return (false, None, None, "rate_limited", true);
        }
        if !(200..300).contains(&status) {
            return (false, None, None, "http_error", false);
        }
        let parsed: Option<Value> = serde_json::from_str::<Value>(raw_body).ok();
        let Some(full) = parsed else {
            return (false, None, None, "parse_error", false);
        };
        // 显式 success=false 视作服务端拒绝。
        if let Some(false) = full.get("success").and_then(|v| v.as_bool()) {
            // body 里偶尔会出现"频繁/限流/请稍后"字样，提取出来给上层做更长等待。
            let rl = full
                .get("msg")
                .and_then(|v| v.as_str())
                .map(|s| s.contains("频繁") || s.contains("限流") || s.contains("请稍后"))
                .unwrap_or(false);
            return (
                false,
                Some(full.clone()),
                None,
                "server_rejected",
                rl,
            );
        }
        let data = full.get("data").cloned();
        let accepted = match &data {
            Some(Value::Object(map)) => {
                map.get("is_right").map(|v| v.is_boolean()).unwrap_or(false)
                    || map.get("my_score").map(|v| v.is_number()).unwrap_or(false)
                    || map.contains_key("right_answer")
                    || map.contains_key("submit_answer")
            }
            // data 是 null / 缺失 / 非对象都视作未被批改。
            _ => false,
        };
        if accepted {
            (true, Some(full), data, "", false)
        } else {
            (false, Some(full), data, "missing_grade", false)
        }
    }
}

/// 学堂在线小题提交，外加"是否被批改/接受"的结构化判定。
/// 调用方应基于 `outcome.is_accepted` 决定是否重试，而不是简单看是否 Err。
/// 网络异常会以 Err 抛出（无 HTTP 响应可分析）。
pub async fn submit_problem_detailed(
    client: &XtClient,
    leaf_id: i64,
    classroom_id: i64,
    exercise_id: i64,
    problem_id: i64,
    sign: &str,
    answer: Vec<String>,
    answers: Value,
) -> Result<SubmitOutcome> {
    let body = json!({
        "leaf_id": leaf_id,
        "classroom_id": classroom_id,
        "exercise_id": exercise_id,
        "problem_id": problem_id,
        "sign": sign,
        "answers": answers,
        "answer": answer
    });
    let (status, raw_body) = client
        .post_json_raw("/api/v1/lms/exercise/problem_apply/", &body)
        .await?;
    let (is_accepted, full, data, reason, rate_limited) =
        SubmitOutcome::judge(status, &raw_body);
    Ok(SubmitOutcome {
        status,
        raw_body,
        full,
        data,
        is_accepted,
        reason,
        rate_limited,
    })
}

/// 用统一接口提交一道题，并在被风控/响应不完整时按 [`SubmitDelay`] 再重试一次。
/// 调用方传入 `submit_count`（用于决定是否需要"前置延迟"，由上层在循环外维护）。
///
/// 行为：
/// 1. 直接发起第一次提交；若成功立即返回。
/// 2. 失败：emit `submit_failed`（含 reason / status / attempt=1），按延迟节流后再提交一次。
///    - 限流（429 / 服务端文案含"频繁/请稍后"）的话用 2 倍延迟，给学堂更长的喘息窗口。
///    - 其他失败用一倍。
/// 3. 第二次结束 emit `submit_retried`（含 attempt=2 + 是否成功）。
/// 4. 网络异常按"失败 outcome"处理，但 reason=network_error 时第二次也直接走原延迟。
async fn submit_with_retry(
    client: &XtClient,
    leaf_id: i64,
    classroom_id: i64,
    exercise_id: i64,
    problem_id: i64,
    kind: ProblemKind,
    kind_label: &str,
    index: usize,
    sign: &str,
    answer: Vec<String>,
    answers: Value,
    submit_delay: SubmitDelay,
    from_bank: bool,
    on_progress: &(dyn Fn(&str, Value) + Send + Sync),
) -> SubmitOutcome {
    // 先做第一次尝试。
    let first = match submit_problem_detailed(
        client,
        leaf_id,
        classroom_id,
        exercise_id,
        problem_id,
        sign,
        answer.clone(),
        answers.clone(),
    )
    .await
    {
        Ok(o) => o,
        Err(e) => SubmitOutcome {
            status: 0,
            raw_body: format!("{e}"),
            full: None,
            data: None,
            is_accepted: false,
            reason: "network_error",
            rate_limited: false,
        },
    };
    if first.is_accepted {
        return first;
    }

    // 走重试路径。先告知前端"将在 X ms 后重试"。
    let base = submit_delay.pick();
    let dur = if first.rate_limited {
        // 限流时延长一倍，避免再撞到同一拨限流窗口。
        std::time::Duration::from_millis(base.as_millis() as u64 * 2)
    } else {
        base
    };
    on_progress(
        "submit_failed",
        json!({
            "problem_id": problem_id,
            "kind": kind,
            "kind_label": kind_label,
            "index": index,
            "attempt": 1u32,
            "status": first.status,
            "reason": first.reason,
            "rate_limited": first.rate_limited,
            "from_bank": from_bank,
            "retry_in_ms": dur.as_millis() as u64,
        }),
    );
    tokio::time::sleep(dur).await;

    let second = match submit_problem_detailed(
        client,
        leaf_id,
        classroom_id,
        exercise_id,
        problem_id,
        sign,
        answer,
        answers,
    )
    .await
    {
        Ok(o) => o,
        Err(e) => SubmitOutcome {
            status: 0,
            raw_body: format!("{e}"),
            full: None,
            data: None,
            is_accepted: false,
            reason: "network_error",
            rate_limited: false,
        },
    };
    on_progress(
        "submit_retried",
        json!({
            "problem_id": problem_id,
            "kind": kind,
            "kind_label": kind_label,
            "index": index,
            "attempt": 2u32,
            "status": second.status,
            "reason": second.reason,
            "accepted": second.is_accepted,
            "rate_limited": second.rate_limited,
            "from_bank": from_bank,
        }),
    );
    second
}

/// 自动跑一整套习题：取题目 → 询 AI / 题库 → 提交。
/// 返回每个题目的结构化结果（前端可按 kind 分组统计）。
///
/// `on_progress` 回调用于实时上报进度，调用方（commands 层）负责把它转成 tauri 事件。
/// 阶段约定：
///   - "start"          info = { problem_id, kind, kind_label, index, total }
///   - "skipped"        info = { problem_id, kind, index, my_score, is_right }
///   - "bank_hit"       info = { problem_id, kind, index, answer_text, matched_by }
///   - "delaying"       info = { ..., delay_ms, reason, from_bank }
///   - "asking_ai"      info = { problem_id, kind, index, delay_ms? }
///   - "intentional_wrong" info = { ..., original_answer_text, wrong_answer_text }
///   - "submitting"     info = { problem_id, kind, index, answer_text, intentional_wrong? }
///   - "submit_failed"  info = { ..., attempt=1, status, reason, retry_in_ms }
///   - "submit_retried" info = { ..., attempt=2, accepted, reason }
///   - "wrong_plan"     info = { planned_wrong_count, problem_ids, wrong_max }
///   - "item_done"      info = { problem_id, kind, index, result }
///   - "done"           info = { total, bank_harvested, failed_count, failures, intentional_wrong_count }
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
    // bank: 本地题库句柄。None 时退化为旧行为：直接询问 AI、不查不写题库。
    // use_local_bank: 是否优先查本地题库；bank 为 None 时本参数无效。
    // auto_harvest: 整批结束时是否把响应里的已批改答案自动入库；bank 为 None 时无效。
    bank: Option<&RwLock<Bank>>,
    use_local_bank: bool,
    auto_harvest: bool,
    // submit_delay: 每次"真正发出 submit_problem"前的随机延迟配置。
    // - 第一次 submit 不延迟（首题秒答合理，整体看起来更像人）。
    // - 后续每次提交都先 sleep(rand range) 再 submit。
    // - AI 路径会让 sleep 与 AI 询问并发执行，但 submit 必须在两者都完成后才发出。
    submit_delay: SubmitDelay,
    // wrong_max: 本节点最多故意答错的题数。0 = 不控分；命中题目会把答案换成错答提交。
    // 见 [`pick_wrong_targets`] / [`distort_to_wrong`]。
    wrong_max: u32,
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
    let bank_enabled = bank.is_some() && use_local_bank;
    // 已经真正发出过的 submit_problem 次数。第一次不延迟，后续每次都按 submit_delay 节流。
    // 注意：被服务端标记为 submitted=true 的跳过题不计数——它们没有触发实际网络请求。
    let mut submit_count: usize = 0;
    // 收集本次"提交未被学堂接受"的题，最终随 done 事件一起回传给前端汇总告知。
    // 第二次重试还失败的题才进入这里。
    let mut failures: Vec<Value> = Vec::new();
    // 控分：本节点要故意答错哪些 problem_id。0 / 候选题数 <= 1 时 set 为空。
    let wrong_set: HashSet<i64> = pick_wrong_targets(&list.problems, wrong_max);
    if !wrong_set.is_empty() {
        on_progress(
            "wrong_plan",
            json!({
                "planned_wrong_count": wrong_set.len(),
                "problem_ids": wrong_set.iter().collect::<Vec<_>>(),
                "wrong_max": wrong_max,
            }),
        );
    }
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

        // 本地题库查询：在调 AI 之前先看看，命中则直接提交、跳过 AI 询问。
        // 短锁 + clone 出来再 await，避免跨 await 持锁。
        if bank_enabled {
            let hit = bank.unwrap().read().lookup(p);
            if let Some(h) = hit {
                match entry_to_submit_payload(&h.entry, p.problem_id) {
                    Ok((answer_arr, answers_obj, ui_answer)) => {
                        on_progress(
                            "bank_hit",
                            json!({
                                "problem_id": p.problem_id,
                                "kind": p.kind,
                                "kind_label": kind_label,
                                "index": index,
                                "answer_text": ui_answer.clone(),
                                "matched_by": h.matched_by,
                                "source_problem_id": h.entry.problem_id,
                            }),
                        );
                        // 控分：若该题命中"故意答错"集合，把题库给出的答案扰动成错答；
                        // bank_hit 事件还展示真实命中答案（透明可审计），但实际提交用扰动版本。
                        let intentional_wrong = wrong_set.contains(&p.problem_id);
                        let (answer_arr, answers_obj, ui_answer) = if intentional_wrong {
                            let (a, b, c) = distort_to_wrong(p, &answer_arr);
                            on_progress(
                                "intentional_wrong",
                                json!({
                                    "problem_id": p.problem_id,
                                    "kind": p.kind,
                                    "kind_label": kind_label,
                                    "index": index,
                                    "from_bank": true,
                                    "original_answer_text": ui_answer,
                                    "wrong_answer_text": c.clone(),
                                }),
                            );
                            (a, b, c)
                        } else {
                            (answer_arr, answers_obj, ui_answer)
                        };
                        // 题库命中：sleep → submit。第一次提交不延迟。
                        // 这里没有 AI 调用可并发，所以是单纯的串行等待。
                        if submit_count > 0 {
                            let dur = submit_delay.pick();
                            on_progress(
                                "delaying",
                                json!({
                                    "problem_id": p.problem_id,
                                    "kind": p.kind,
                                    "kind_label": kind_label,
                                    "index": index,
                                    "delay_ms": dur.as_millis() as u64,
                                    "reason": "bank_hit",
                                    "from_bank": true,
                                }),
                            );
                            tokio::time::sleep(dur).await;
                        }
                        on_progress(
                            "submitting",
                            json!({
                                "problem_id": p.problem_id,
                                "kind": p.kind,
                                "kind_label": kind_label,
                                "index": index,
                                "answer_text": ui_answer.clone(),
                                "from_bank": true,
                                "intentional_wrong": intentional_wrong,
                            }),
                        );
                        let outcome = submit_with_retry(
                            client,
                            leaf_id,
                            classroom_id,
                            exercise_id,
                            p.problem_id,
                            p.kind,
                            kind_label,
                            index,
                            sign,
                            answer_arr.clone(),
                            answers_obj,
                            submit_delay,
                            true,
                            on_progress,
                        )
                        .await;
                        let result = if outcome.is_accepted {
                            // 故意答错时不计 bank 命中（统计上算"用了题库但没拿到分"会误导）。
                            if !intentional_wrong {
                                bank.unwrap().write().record_hit(h.entry.problem_id);
                            }
                            json!({
                                "problem_id": p.problem_id,
                                "kind": p.kind,
                                "kind_label": kind_label,
                                "answer": answer_arr,
                                "answer_text": ui_answer.clone(),
                                "from_bank": true,
                                "matched_by": h.matched_by,
                                "intentional_wrong": intentional_wrong,
                                "submit": outcome.data.clone().unwrap_or(Value::Null),
                            })
                        } else {
                            // 两次都失败：拼一个人类可读的错误描述。
                            let err_msg = describe_submit_failure(&outcome);
                            let result = json!({
                                "problem_id": p.problem_id,
                                "kind": p.kind,
                                "kind_label": kind_label,
                                "answer": answer_arr,
                                "answer_text": ui_answer.clone(),
                                "from_bank": true,
                                "matched_by": h.matched_by,
                                "intentional_wrong": intentional_wrong,
                                "error": err_msg.clone(),
                                "submit_status": outcome.status,
                                "submit_reason": outcome.reason,
                            });
                            failures.push(json!({
                                "problem_id": p.problem_id,
                                "kind": p.kind,
                                "kind_label": kind_label,
                                "index": index,
                                "error": err_msg,
                                "status": outcome.status,
                                "reason": outcome.reason,
                                "from_bank": true,
                                "intentional_wrong": intentional_wrong,
                            }));
                            result
                        };
                        // 不管成功失败都计数，避免连续失败时下一次又"零延迟"。
                        // 第二次重试本身也已经被 submit_with_retry 内部节流过了。
                        submit_count += 1;
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
                    Err(e) => {
                        log::warn!(
                            "题库条目转提交载荷失败 problem_id={} err={e}",
                            p.problem_id
                        );
                        // 失败则回退到 AI 流程
                    }
                }
            }
        }

        // 题库未命中 / 转换失败：走 AI。
        // 这里把"提交节流的随机延迟"与"询问 AI"做成 tokio::join! 并发，
        // 这样在 AI 还没回答完时延迟就在后台同步消耗，提交时机 = max(sleep, ai)。
        // 注意：只有 submit_count > 0 时才需要延迟（首题秒答更像人工）。
        let delay_dur = if submit_count > 0 {
            Some(submit_delay.pick())
        } else {
            None
        };

        on_progress(
            "asking_ai",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
                // 把延迟时长一并上报，前端如果想做"询问 AI 中（同时节流 X 秒）"提示可以用上。
                "delay_ms": delay_dur.map(|d| d.as_millis() as u64),
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
        let ai_fut = ask_ai_with_retry(ai, &pa);
        let sleep_fut = async {
            if let Some(d) = delay_dur {
                tokio::time::sleep(d).await;
            }
        };
        // join! 让两个 future 并发；任一比另一个早完成的都会在这里等待。
        let (ai_result, _) = tokio::join!(ai_fut, sleep_fut);
        let spec = match ai_result {
            Ok(s) => s,
            Err(e) => {
                // AI 失败时不会真正发出 submit，所以不计 submit_count；
                // 下一题会重新决定是否需要延迟（如果它是本批第一次成功提交，仍按"首题"处理）。
                let result = json!({
                    "problem_id": p.problem_id,
                    "kind": p.kind,
                    "kind_label": kind_label,
                    "error": format!("{e}")
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

        // 控分：本题命中"故意答错"集合时把 AI 给的答案扰动成错答。
        let intentional_wrong = wrong_set.contains(&p.problem_id);
        let (answer_arr, answers_obj, ui_answer) = if intentional_wrong {
            let (a, b, c) = distort_to_wrong(p, &answer_arr);
            on_progress(
                "intentional_wrong",
                json!({
                    "problem_id": p.problem_id,
                    "kind": p.kind,
                    "kind_label": kind_label,
                    "index": index,
                    "from_bank": false,
                    "original_answer_text": ui_answer,
                    "wrong_answer_text": c.clone(),
                }),
            );
            (a, b, c)
        } else {
            (answer_arr, answers_obj, ui_answer)
        };

        on_progress(
            "submitting",
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
                "answer_text": ui_answer.clone(),
                "intentional_wrong": intentional_wrong,
            }),
        );

        let outcome = submit_with_retry(
            client,
            leaf_id,
            classroom_id,
            exercise_id,
            p.problem_id,
            p.kind,
            kind_label,
            index,
            sign,
            answer_arr.clone(),
            answers_obj,
            submit_delay,
            false,
            on_progress,
        )
        .await;
        let result = if outcome.is_accepted {
            json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "answer": answer_arr,
                "answer_text": ui_answer.clone(),
                "intentional_wrong": intentional_wrong,
                "submit": outcome.data.clone().unwrap_or(Value::Null),
            })
        } else {
            let err_msg = describe_submit_failure(&outcome);
            let result = json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "answer": answer_arr,
                "answer_text": ui_answer.clone(),
                "intentional_wrong": intentional_wrong,
                "error": err_msg.clone(),
                "submit_status": outcome.status,
                "submit_reason": outcome.reason,
            });
            failures.push(json!({
                "problem_id": p.problem_id,
                "kind": p.kind,
                "kind_label": kind_label,
                "index": index,
                "error": err_msg,
                "status": outcome.status,
                "reason": outcome.reason,
                "from_bank": false,
                "intentional_wrong": intentional_wrong,
            }));
            result
        };
        // AI 分支：无论学堂是否接受，都视作一次真实提交，给下一题加上节流。
        submit_count += 1;

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

    // 整批结束时若开启了自动收录：再拉一次 list 把刚提交完的题目（此时已被批改）入库。
    // 直接复用本次 list 里 submitted=true 的条目是不够的 —— 本次提交的题在 list 时还是
    // 未提交态。所以重新拉一次。失败仅记日志、不影响主流程。
    let mut harvested: usize = 0;
    if let (Some(bank_lock), true) = (bank, auto_harvest) {
        match fetch_exercise_with_referer(
            client,
            exercise_id,
            sku_id,
            Some(&referer),
        )
        .await
        {
            Ok(fresh) => {
                let mut guard = bank_lock.write();
                for prob in &fresh.problems {
                    if guard.upsert_from_problem(prob) {
                        harvested += 1;
                    }
                }
            }
            Err(e) => {
                log::warn!("auto_harvest 重新拉取 list 失败 exercise_id={exercise_id} err={e}");
            }
        }
    }

    on_progress(
        "done",
        json!({
            "total": total,
            "bank_harvested": harvested,
            "failed_count": failures.len(),
            "failures": failures,
            "intentional_wrong_count": wrong_set.len(),
        }),
    );
    Ok(results)
}

/// 把 SubmitOutcome 翻译成给用户看的"提交失败"文本。简洁、避免堆栈泄漏。
fn describe_submit_failure(o: &SubmitOutcome) -> String {
    match o.reason {
        "rate_limited" => "学堂限流（HTTP 429），两次尝试都被拒绝".to_string(),
        "http_error" => format!("HTTP {} 错误，重试后仍未通过", o.status),
        "parse_error" => format!(
            "学堂返回不是合法 JSON（HTTP {}），可能命中风控页",
            o.status
        ),
        "server_rejected" => {
            let msg = o
                .full
                .as_ref()
                .and_then(|v| v.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("未提供原因");
            format!("学堂拒绝提交：{msg}")
        }
        "missing_grade" => {
            "学堂未返回批改字段（无 is_right/my_score），无法确认是否真正提交".to_string()
        }
        "network_error" => format!("网络异常：{}", o.raw_body),
        _ => format!("提交失败（HTTP {}）", o.status),
    }
}

/// 从未提交题里随机抽 N 个 problem_id 作为"故意答错"目标集合。
///
/// - `wrong_max=0` 直接返回空集合（即关闭"控分"）。
/// - 实际答错数 = min(wrong_max, 未提交题数 - 1)：保留至少 1 道答对，避免 0 分太刻意。
///   如果整个 exercise 总共只有 1 道未提交题，则强制让它答对（返回空集合）。
/// - 仅在未提交题中抽样；已提交题（submitted=true）不会动。
fn pick_wrong_targets(problems: &[Problem], wrong_max: u32) -> HashSet<i64> {
    if wrong_max == 0 {
        return HashSet::new();
    }
    let candidates: Vec<i64> = problems
        .iter()
        .filter(|p| !p.submitted)
        .map(|p| p.problem_id)
        .collect();
    if candidates.len() <= 1 {
        return HashSet::new();
    }
    let take = (wrong_max as usize).min(candidates.len().saturating_sub(1));
    let mut rng = rand::thread_rng();
    candidates
        .choose_multiple(&mut rng, take)
        .cloned()
        .collect()
}

/// 把"正确答案"扰动成"错答"，用于控分。两边（题库 / AI）拿到答案后都走这里。
///
/// 返回值结构与 `entry_to_submit_payload` 一致：`(answer_arr, answers_obj, ui_answer)`。
///
/// 策略：
/// - 选项题（含判断题）：从 `problem.options` 里挑一个不在 `correct_keys` 里的 key 作为
///   错答（恰好 1 个）。这样单选自然就错；多选也大概率错（数量/集合都和正解不同）；
///   判断题"true/false"在 options 里有对应 key，扰动后变成反面。若所有选项都属于正解
///   （理论上多选全选才会发生），就提交空数组——也是错答。
/// - 文本题（填空 / 主观）：填 "无"（学堂阅卷常见 0 分占位）。
fn distort_to_wrong(
    problem: &Problem,
    correct_answer_arr: &[String],
) -> (Vec<String>, Value, String) {
    if problem.kind.is_choice() {
        let correct: HashSet<&str> =
            correct_answer_arr.iter().map(|s| s.as_str()).collect();
        let wrong_candidates: Vec<String> = problem
            .options
            .iter()
            .map(|o| o.key.clone())
            .filter(|k| !correct.contains(k.as_str()))
            .collect();
        if wrong_candidates.is_empty() {
            // 罕见：没有可用的"错"选项（多选全选了所有选项）。空提交也是错。
            return (Vec::new(), json!({}), String::from("（空）"));
        }
        let mut rng = rand::thread_rng();
        let pick = &wrong_candidates[rng.gen_range(0..wrong_candidates.len())];
        (vec![pick.clone()], json!({}), pick.clone())
    } else {
        // 文本题：固定占位答案，简单可读。
        let txt = String::from("无");
        (
            Vec::new(),
            json!({ problem.problem_id.to_string(): txt.clone() }),
            txt,
        )
    }
}
