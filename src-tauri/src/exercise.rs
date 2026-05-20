use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::ai::{ask_ai, AnswerSpec, ProblemForAi};
use crate::client::XtClient;
use crate::state::AiSettings;

#[derive(Clone, Debug, Serialize)]
pub struct Problem {
    pub problem_id: i64,
    pub problem_type: i64,
    pub problem_type_text: String,
    pub body_html: String,
    pub options: Vec<ProblemOption>,
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

pub async fn fetch_exercise(
    client: &XtClient,
    exercise_id: i64,
    sku_id: i64,
) -> Result<ExerciseList> {
    let path = format!("/api/v1/lms/exercise/get_exercise_list/{exercise_id}/{sku_id}/");
    let v = client.get_json(&path).await?;
    let d = v.get("data").ok_or_else(|| anyhow!("exercise 缺 data"))?;
    let problems_raw = d
        .get("problems")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
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
        let problem_type_text = content
            .get("TypeText")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
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
        problems.push(Problem {
            problem_id,
            problem_type,
            problem_type_text,
            body_html,
            options,
        });
    }
    Ok(ExerciseList {
        exercise_id,
        problems,
    })
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

/// 自动跑一整套习题：取题目 → 询 AI → 提交
pub async fn auto_run_exercise(
    client: &XtClient,
    ai: &AiSettings,
    leaf_id: i64,
    classroom_id: i64,
    sku_id: i64,
    exercise_id: i64,
    sign: &str,
) -> Result<Vec<Value>> {
    let list = fetch_exercise(client, exercise_id, sku_id).await?;
    let mut results = Vec::new();
    for p in list.problems.iter() {
        let pa = ProblemForAi {
            type_text: p.problem_type_text.clone(),
            body_html: p.body_html.clone(),
            options: p
                .options
                .iter()
                .map(|o| (o.key.clone(), o.value.clone()))
                .collect(),
        };
        let spec = match ask_ai(ai, &pa).await {
            Ok(s) => s,
            Err(e) => {
                results.push(json!({
                    "problem_id": p.problem_id,
                    "error": format!("AI 询问失败: {e}")
                }));
                continue;
            }
        };
        let answer_arr = match &spec {
            AnswerSpec::Choice(keys) => keys.clone(),
            AnswerSpec::Text(t) => vec![t.clone()],
        };
        let answers_obj = match &spec {
            AnswerSpec::Text(t) => json!({ p.problem_id.to_string(): t }),
            AnswerSpec::Choice(_) => json!({}),
        };
        match submit_problem(
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
            Ok(resp) => results.push(json!({
                "problem_id": p.problem_id,
                "answer": answer_arr,
                "submit": resp
            })),
            Err(e) => results.push(json!({
                "problem_id": p.problem_id,
                "answer": answer_arr,
                "error": format!("提交失败: {e}")
            })),
        }
    }
    Ok(results)
}
