//! TypeSafe System One ("Jev") client (evaluated 2026-10-25, ADR-75). Jev is not a chat model: it answers typed
//! questions about supplied state (text, application data, images) with calibrated probabilities. Agenium uses it as
//! an optional, cheap second opinion — e.g. a first evidence read before an expensive review — never as an agent's
//! model. One request batches many questions, so several judgments cost one call.
//!
//! API: `POST <base>/v1/systemone`, `Authorization: Bearer <key>`; images need the header
//! `TypeSafe-Danger-Zone-Images: true` (experimental). Verified shapes:
//! - body `{ "model", "state": {<key>: value|image-ref}, "assets": {<ref>: {type:"image", format, data_base64}},
//!   "questions": {<id>: {...}} }`; every asset must be referenced from the state or a question.
//! - `noul`  = `{type:"noul", instructions, criteria?}` -> `{noul: 0..1}` (probability the proposition holds).
//! - `choice`= `{type:"choice", instructions, criteria: {id: desc}}` -> `{choice, confidence, probabilities}`.
//! - `score` = `{type:"score", instructions, criteria: {level: desc}}` -> `{score, confidence}`.
//!
//! A question refers to a state entry by its key in backticks, e.g. `` `report` ``.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::Http;
use crate::LlmError;

/// A typed question for Jev.
#[derive(Debug, Clone)]
pub enum Question {
    /// Probability the proposition is true.
    Noul { instructions: String },
    /// One option among `criteria` (id -> description).
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
}

impl Question {
    fn to_json(&self) -> Value {
        match self {
            Question::Noul { instructions } => {
                json!({"type": "noul", "instructions": instructions})
            }
            Question::Choice {
                instructions,
                criteria,
            } => {
                json!({"type": "choice", "instructions": instructions, "criteria": criteria})
            }
        }
    }
}

/// A PNG/JPEG image for the state (base64, no `data:` prefix, each edge 280..=1568 px).
#[derive(Debug, Clone)]
pub struct Image {
    pub format: String,
    pub data_base64: String,
}

/// One judging request: the state the questions read, optional images, and the questions by id.
#[derive(Debug, Clone, Default)]
pub struct Judging {
    /// State entries by key (strings, numbers, nested JSON); an image is referenced by its asset key here.
    pub state: BTreeMap<String, Value>,
    /// Images by asset key; each must be referenced from `state`.
    pub images: BTreeMap<String, Image>,
    pub questions: BTreeMap<String, Question>,
}

impl Judging {
    /// Puts text under `key` in the state and makes `id` a Noul question over it.
    pub fn noul(mut self, id: &str, instructions: &str) -> Self {
        self.questions.insert(
            id.to_owned(),
            Question::Noul {
                instructions: instructions.to_owned(),
            },
        );
        self
    }

    pub fn state_text(mut self, key: &str, text: &str) -> Self {
        self.state.insert(key.to_owned(), json!(text));
        self
    }
}

/// One answer, kept close to the wire so callers read what they asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    #[serde(rename = "type")]
    pub kind: String,
    /// `noul`: probability 0..1.
    #[serde(default)]
    pub noul: Option<f64>,
    /// `choice`: the chosen option id.
    #[serde(default)]
    pub choice: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub probabilities: Option<BTreeMap<String, f64>>,
}

impl Answer {
    /// True when a Noul lands at or above `threshold` (decision 35 uses 0.7/0.3; between is "unsure").
    pub fn is_true(&self, threshold: f64) -> bool {
        self.noul.is_some_and(|p| p >= threshold)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Response {
    #[serde(default)]
    answers: BTreeMap<String, Answer>,
}

/// A configured Jev endpoint (ADR-75). Cheap, calibrated judgments; never an agent's chat model.
#[derive(Clone)]
pub struct Judge {
    http: Http,
    url: String,
    key: String,
    model: String,
}

impl Judge {
    pub fn new(base_url: &str, key: &str, model: &str) -> Result<Self, LlmError> {
        Ok(Self {
            http: Http::default_client()?,
            url: format!("{}/v1/systemone", base_url.trim_end_matches('/')),
            key: key.to_owned(),
            model: model.to_owned(),
        })
    }

    /// Asks the questions in one request; returns each answer by id.
    pub async fn ask(&self, j: &Judging) -> Result<BTreeMap<String, Answer>, LlmError> {
        if j.questions.is_empty() {
            return Ok(BTreeMap::new());
        }
        let assets: BTreeMap<String, Value> = j
            .images
            .iter()
            .map(|(k, i)| {
                (
                    k.clone(),
                    json!({"type": "image", "format": i.format, "data_base64": i.data_base64}),
                )
            })
            .collect();
        let questions: BTreeMap<String, Value> = j
            .questions
            .iter()
            .map(|(id, q)| (id.clone(), q.to_json()))
            .collect();
        let mut body = json!({"model": self.model, "state": j.state, "questions": questions});
        if !assets.is_empty() {
            body["assets"] = json!(assets);
        }
        let mut headers = vec![("authorization", format!("Bearer {}", self.key))];
        if !assets.is_empty() {
            headers.push(("typesafe-danger-zone-images", "true".to_owned()));
        }
        let v = self
            .http
            .post_json(&self.url, &headers, &body, map_err)
            .await?;
        let r: Response = serde_json::from_value(v).map_err(|e| LlmError::Decode(e.to_string()))?;
        Ok(r.answers)
    }
}

fn map_err(status: u16, _: &reqwest::header::HeaderMap, body: &str) -> LlmError {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["detail"]["message"]
                .as_str()
                .or_else(|| v["detail"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| body.chars().take(200).collect());
    match status {
        401 | 403 => LlmError::Config(format!("TypeSafe key rejected: {detail}")),
        400 | 422 => LlmError::Config(format!("TypeSafe request invalid: {detail}")),
        429 => LlmError::Network(format!("TypeSafe rate limited: {detail}")),
        _ => LlmError::Network(format!("TypeSafe HTTP {status}: {detail}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_shape_and_answer_parsing() {
        let j = Judging::default()
            .state_text("report", "added hello.txt and the test passed")
            .noul("has_file", "Does `report` create hello.txt?")
            .noul("tested", "Does `report` mention a passing test?");
        assert_eq!(j.questions.len(), 2);
        let q = j.questions["has_file"].to_json();
        assert_eq!(q["type"], "noul");

        // Answers as the live API returned them (jev-1.14.0).
        let r: Response = serde_json::from_value(serde_json::json!({
            "model": "jev-1.14.0",
            "answers": {
                "has_file": {"type": "noul", "noul": 0.97},
                "lang": {"type": "choice", "choice": "code", "confidence": 0.98,
                         "probabilities": {"code": 0.99, "docs": 0.0, "other": 0.01}}
            }
        }))
        .unwrap();
        assert!(r.answers["has_file"].is_true(0.7));
        assert!(!r.answers["has_file"].is_true(0.99));
        assert_eq!(r.answers["lang"].choice.as_deref(), Some("code"));
    }
}
