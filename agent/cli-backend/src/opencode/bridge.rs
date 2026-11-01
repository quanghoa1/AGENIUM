//! The opencode bridge (ADR-68, ADR-153): `opencode serve` driven through its HTTP API, its event stream turned into
//! the driver's events, permission requests answered through Agenium's hook, and a turn whose answer left no text event
//! read back (or asked again) instead of completing empty.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use super::{model_ref, MODEL_ENV, PASSWORD_ENV, TOKEN_ENV};
use crate::driver::HOOK_URL_ENV;

/// The next line of `r`, its invalid UTF-8 replaced, `None` at the end (`lines()` stops for good at such a byte).
async fn next_line_lossy<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    buf: &mut Vec<u8>,
) -> Option<String> {
    buf.clear();
    match r.read_until(b'\n', buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(
            String::from_utf8_lossy(buf)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        ),
    }
}

/// Runs a turn-by-turn conversation with an `opencode serve` process: reads the server address from its stdout,
/// creates the session in `worktree` (or goes on with `resume`, the ticket's kept one), posts each input line (`{"text": ...}`) as a prompt and sends the server's
/// events to `out` (tool events carry the tool's name and input). Permission requests go to Agenium's hook
/// endpoint and its decision back to opencode. Ends when `input` closes.
pub(crate) fn bridge(
    stdout: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    mut input: mpsc::UnboundedReceiver<String>,
    out: mpsc::UnboundedSender<Value>,
    env: &BTreeMap<String, String>,
    worktree: PathBuf,
    tail: Arc<Mutex<Vec<u8>>>,
    resume: Option<String>,
) {
    let password = env.get(PASSWORD_ENV).cloned().unwrap_or_default();
    let hook = env.get(HOOK_URL_ENV).cloned().unwrap_or_default();
    let token = env.get(TOKEN_ENV).cloned().unwrap_or_default();
    let model = env.get(MODEL_ENV).cloned().unwrap_or_default();
    tokio::spawn(async move {
        let fail = |out: &mpsc::UnboundedSender<Value>, msg: String| {
            let _ = out.send(json!({"type": "session.execution.failed", "data": {"error": msg}}));
        };
        // Read as bytes: one line that is not UTF-8 stopped `lines()` for good, and the server's pipe was no longer
        // drained (audit M12, 2026-10-31).
        let (mut r, mut buf) = (BufReader::new(stdout), Vec::new());
        let base = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            while let Some(l) = next_line_lossy(&mut r, &mut buf).await {
                crate::run::keep_tail(&tail, format!("{l}\n").as_bytes());
                if let Some(u) = l.split("listening on ").nth(1) {
                    return Some(u.trim().trim_end_matches('/').to_owned());
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let Some(base) = base else {
            return fail(&out, "opencode serve did not start".into());
        };
        // Keep draining the server's stdout (its tail explains failures).
        let t = tail.clone();
        tokio::spawn(async move {
            while let Some(l) = next_line_lossy(&mut r, &mut buf).await {
                crate::run::keep_tail(&t, format!("{l}\n").as_bytes());
            }
        });
        let http = reqwest::Client::new();
        let api = |path: &str| format!("{base}/api{path}");
        let auth = |rb: reqwest::RequestBuilder| rb.basic_auth("opencode", Some(&password));
        let events = match auth(http.get(api("/event")))
            .header("accept", "text/event-stream")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => return fail(&out, format!("opencode events: HTTP {}", r.status())),
            Err(e) => return fail(&out, format!("opencode events: {e}")),
        };
        // The ticket's session of an earlier run, its data brought back (ADR-134); a prompt to a session the server
        // does not know fails, and the run starts again with a new one.
        let sid = if let Some(sid) = resume {
            sid
        } else {
            let mut body = json!({"location": {"directory": worktree.display().to_string()}});
            if !model.is_empty() {
                body["model"] = model_ref(&model);
            }
            let session = match auth(http.post(api("/session"))).json(&body).send().await {
                Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
                Ok(r) => {
                    let s = r.status();
                    let text = r.text().await.unwrap_or_default();
                    return fail(&out, format!("opencode session: HTTP {s}: {text}"));
                }
                Err(e) => return fail(&out, format!("opencode session: {e}")),
            };
            let Some(sid) = session
                .as_ref()
                .and_then(|s| s["data"]["id"].as_str().or_else(|| s["id"].as_str()))
                .map(str::to_owned)
            else {
                return fail(&out, "opencode created no session".into());
            };
            sid
        };
        // Events of this session, tool events completed with the tool's name and input.
        {
            let (out, http, sid) = (out.clone(), http.clone(), sid.clone());
            let (api_base, password) = (base.clone(), password.clone());
            tokio::spawn(async move {
                use futures::StreamExt;
                let mut stream = events.bytes_stream();
                let mut sse = agenium_llm::sse::SseDecoder::new();
                let mut names: BTreeMap<String, (String, Value)> = BTreeMap::new();
                let mut watch = TextWatch::default();
                let mut nudged = false;
                while let Some(Ok(chunk)) = stream.next().await {
                    for e in sse.feed(&chunk) {
                        let Ok(mut v) = serde_json::from_str::<Value>(&e.data) else {
                            continue;
                        };
                        let (ask, own) = route(&v, &sid);
                        tracing::debug!(
                            target: "agenium_cli_backend::opencode::events",
                            own,
                            kind = v["type"].as_str().unwrap_or(""),
                            session = v["data"]["sessionID"].as_str().unwrap_or("-"),
                            data = %v["data"].to_string().chars().take(300).collect::<String>(),
                        );
                        if ask {
                            let (http, hook, token) = (http.clone(), hook.clone(), token.clone());
                            let (base, password, data) =
                                (api_base.clone(), password.clone(), v["data"].clone());
                            tokio::spawn(async move {
                                answer(&http, &hook, &token, &base, &password, data).await
                            });
                        }
                        if !own {
                            continue;
                        }
                        let id = v["data"]["id"].as_str().unwrap_or("").to_owned();
                        match v["type"].as_str().unwrap_or("") {
                            "session.tool.input.started" => {
                                let name = v["data"]["name"].as_str().unwrap_or("tool").to_owned();
                                names.insert(id, (name, Value::Null));
                            }
                            "session.tool.called" => {
                                let e = names
                                    .entry(id)
                                    .or_insert_with(|| ("tool".into(), Value::Null));
                                e.1 = v["data"]["input"].clone();
                                v["data"]["name"] = json!(e.0);
                            }
                            "session.tool.success" | "session.tool.error" => {
                                if let Some((name, input)) = names.get(&id) {
                                    v["data"]["name"] = json!(name);
                                    v["data"]["input"] = input.clone();
                                }
                            }
                            _ => {}
                        }
                        // A user's prompt (not the bridge's nudge) starts a turn that may be nudged once.
                        if v["type"] == "session.inbox.enqueued"
                            && v["data"]["item"]["payload"]["text"] != EMPTY_NUDGE
                        {
                            nudged = false;
                        }
                        // A turn whose last step showed no text: its answer is read back from the session; a turn
                        // that has none is asked again once, then fails instead of completing empty.
                        if watch.see(&v) {
                            let text = final_text(&http, &api_base, &password, &sid).await;
                            match after_silent(text, nudged) {
                                Silent::Text(text) => {
                                    let recovered = json!({"type": "session.text.ended",
                                        "data": {"sessionID": sid, "text": text}});
                                    if out.send(recovered).is_err() {
                                        return;
                                    }
                                }
                                Silent::Nudge => {
                                    nudged = true;
                                    let asked = http
                                        .post(format!("{api_base}/api/session/{sid}/prompt"))
                                        .basic_auth("opencode", Some(&password))
                                        .json(&json!({"text": EMPTY_NUDGE}))
                                        .send()
                                        .await;
                                    if asked.is_ok_and(|r| r.status().is_success()) {
                                        continue;
                                    }
                                }
                                Silent::Fail => {
                                    v = json!({"type": "session.execution.failed", "data": {"error": {
                                        "message": "the model's answer was empty twice (opencode recorded no text)"}}});
                                }
                            }
                        }
                        if out.send(v).is_err() {
                            return;
                        }
                    }
                }
                // The server stopped (its event stream ended): the run is told, not left waiting 45 minutes for an
                // end of turn that cannot come.
                let _ = out.send(json!({"type": "session.execution.failed",
                    "data": {"error": "the opencode server stopped (its event stream ended)"}}));
            });
        }
        while let Some(line) = input.recv().await {
            let text = serde_json::from_str::<Value>(line.trim())
                .ok()
                .and_then(|v| v["text"].as_str().map(str::to_owned))
                .unwrap_or_else(|| line.trim().to_owned());
            match auth(http.post(api(&format!("/session/{sid}/prompt"))))
                .json(&json!({"text": text}))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => fail(&out, format!("opencode prompt: HTTP {}", r.status())),
                Err(e) => fail(&out, format!("opencode prompt: {e}")),
            }
        }
    });
}

/// Whether a turn ended without its answer in the stream. opencode sometimes ends a step with output tokens and
/// `finish: stop` but no text event (OpenCode Go's `iceman-contributor-free`, about one turn in ten, 2026-11-01): the
/// run then completed with an empty answer. Its last assistant message is read back; when that is empty too (the
/// model returned no text: `content: []`), the model is asked once more, then the turn fails.
#[derive(Default)]
struct TextWatch {
    /// The current step showed text.
    step_text: bool,
    /// The last step ended as the final answer (`stop`) without showing any.
    silent_stop: bool,
}

impl TextWatch {
    /// Follows one event of the run's session; true on the turn's end when its answer must be read back.
    fn see(&mut self, v: &Value) -> bool {
        match v["type"].as_str().unwrap_or("") {
            "session.execution.started" => *self = Self::default(),
            "session.step.started" => self.step_text = false,
            "session.text.ended" | "session.text.delta" => self.step_text = true,
            "session.step.ended" => {
                self.silent_stop = !self.step_text && v["data"]["finish"] == "stop";
            }
            "session.execution.succeeded" => return std::mem::take(&mut self.silent_stop),
            _ => {}
        }
        false
    }
}

/// Sent when the model's answer was empty: it answers the user's message again, in the same turn.
const EMPTY_NUDGE: &str = "Your previous reply came back empty. Reply to my last message now.";

/// What becomes of a turn whose last step showed no text.
#[derive(Debug, PartialEq)]
enum Silent {
    /// Its text, read back from the session.
    Text(String),
    /// None: the model is asked once more.
    Nudge,
    /// None again: the turn fails rather than completing with no answer.
    Fail,
}

fn after_silent(text: Option<String>, nudged: bool) -> Silent {
    match (text, nudged) {
        (Some(t), _) => Silent::Text(t),
        (None, false) => Silent::Nudge,
        (None, true) => Silent::Fail,
    }
}

/// The text of the newest assistant message of a session (`GET /api/session/<sid>/message`, newest first).
fn assistant_text(messages: &Value) -> Option<String> {
    let list = messages["data"]
        .as_array()
        .or_else(|| messages.as_array())?;
    let m = list.iter().find(|m| m["type"] == "assistant")?;
    let text: Vec<&str> = m["content"]
        .as_array()?
        .iter()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect();
    let text = text.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

async fn final_text(
    http: &reqwest::Client,
    base: &str,
    password: &str,
    sid: &str,
) -> Option<String> {
    let v: Value = http
        .get(format!("{base}/api/session/{sid}/message"))
        .basic_auth("opencode", Some(password))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    tracing::debug!(
        target: "agenium_cli_backend::opencode::events",
        sid,
        messages = %v.to_string().chars().take(3000).collect::<String>(),
        "a turn without text: its messages"
    );
    assistant_text(&v)
}

/// What the bridge does with one server event of a run whose session is `sid`: `(ask, own)`. A permission request
/// of any session is asked of Agenium and answered: the server is the run's own (its own home and password), so its
/// sessions are the run's — its session, and the child sessions in which opencode's `task` tool runs a subagent,
/// whose requests went unanswered and hung the run. Only the run's own session's events go on to the driver.
fn route(v: &Value, sid: &str) -> (bool, bool) {
    (
        v["type"] == "permission.asked",
        v["data"]["sessionID"].as_str().is_none_or(|s| s == sid),
    )
}

/// Asks Agenium's hook endpoint about one permission request and replies to opencode (`once` or `reject` with the
/// reason). A hook that cannot be reached rejects (fail-closed).
async fn answer(
    http: &reqwest::Client,
    hook: &str,
    token: &str,
    base: &str,
    password: &str,
    req: Value,
) {
    let decision = match http.post(hook).bearer_auth(token).json(&req).send().await {
        Ok(r) => r.json::<Value>().await.unwrap_or(Value::Null),
        Err(e) => json!({"decision": "deny", "reason": format!("Agenium could not be asked: {e}")}),
    };
    let allow = decision["decision"] == "allow";
    let mut reply = json!({"decision": if allow { "once" } else { "reject" }});
    if !allow {
        reply["message"] = json!(decision["reason"].as_str().unwrap_or("refused by Agenium"));
    }
    let (sid, id) = (
        req["sessionID"].as_str().unwrap_or(""),
        req["id"].as_str().unwrap_or(""),
    );
    let _ = http
        .post(format!("{base}/api/session/{sid}/permission/{id}/reply"))
        .basic_auth("opencode", Some(password))
        .json(&reply)
        .send()
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The event orders captured from opencode 2.1.2 on `iceman-contributor-free` (2026-11-01): a turn with its text,
    /// a turn with tool steps then text, and the turn that ended with output tokens but no text event.
    #[test]
    fn a_turn_that_ended_without_its_text_is_read_back() {
        let ev = |t: &str, data: Value| json!({"type": t, "data": data});
        let mut w = TextWatch::default();
        // Every event is seen (no short cut), as the bridge sees them.
        let seen =
            |w: &mut TextWatch, seq: &[Value]| seq.iter().fold(false, |any, e| w.see(e) | any);
        let shown = [
            ev("session.execution.started", json!({})),
            ev("session.step.started", json!({})),
            ev("session.text.started", json!({})),
            ev("session.text.ended", json!({"text": "OK-1"})),
            ev("session.step.ended", json!({"finish": "stop"})),
            ev("session.execution.succeeded", json!({})),
        ];
        assert!(!seen(&mut w, &shown));
        let tools_then_text = [
            ev("session.execution.started", json!({})),
            ev("session.step.started", json!({})),
            ev("session.tool.called", json!({})),
            ev("session.step.ended", json!({"finish": "tool-calls"})),
            ev("session.step.started", json!({})),
            ev("session.text.ended", json!({"text": "done"})),
            ev("session.step.ended", json!({"finish": "stop"})),
            ev("session.execution.succeeded", json!({})),
        ];
        assert!(!seen(&mut w, &tools_then_text));
        let silent = [
            ev("session.execution.started", json!({})),
            ev("session.step.started", json!({})),
            ev("session.step.streamed", json!({})),
            ev(
                "session.step.ended",
                json!({"finish": "stop", "tokens": {"output": 6}}),
            ),
        ];
        assert!(!seen(&mut w, &silent));
        assert!(w.see(&ev("session.execution.succeeded", json!({}))));
        // The newest assistant message (the list is newest first, after the turn's `idle` marker).
        let messages = json!({"data": [
            {"type": "idle", "outcome": "succeeded"},
            {"type": "assistant", "content": [{"type": "reasoning", "text": "think"}, {"type": "text", "text": "OK-3"}]},
            {"type": "user", "text": "Reply with OK-3"},
            {"type": "assistant", "content": [{"type": "text", "text": "OK-2"}]},
        ], "cursor": null});
        assert_eq!(assistant_text(&messages).as_deref(), Some("OK-3"));
        // The newest answer recorded empty (`content: []`, seen on `iceman`): not an older one.
        let empty = json!({"data": [
            {"type": "assistant", "content": []},
            {"type": "assistant", "content": [{"type": "text", "text": "OK-1"}]},
        ]});
        assert_eq!(assistant_text(&empty), None);
        // No text: asked again once, then the turn fails.
        assert_eq!(
            after_silent(Some("OK".into()), true),
            Silent::Text("OK".into())
        );
        assert_eq!(after_silent(None, false), Silent::Nudge);
        assert_eq!(after_silent(None, true), Silent::Fail);
    }

    /// Review of 2026-10-27: events of other sessions were dropped before the permission branch, so the requests of
    /// a subagent (opencode's `task` tool runs it in a child session) were never answered and the run hung.
    #[test]
    fn permission_requests_of_child_sessions_are_answered() {
        let ev = |t: &str, s: &str| json!({"type": t, "data": {"sessionID": s, "id": "x"}});
        assert_eq!(
            route(&ev("permission.asked", "ses_child"), "ses_1"),
            (true, false)
        );
        assert_eq!(
            route(&ev("permission.asked", "ses_1"), "ses_1"),
            (true, true)
        );
        assert_eq!(
            route(&ev("session.tool.called", "ses_child"), "ses_1"),
            (false, false)
        );
        assert_eq!(
            route(&ev("session.tool.called", "ses_1"), "ses_1"),
            (false, true)
        );
        assert_eq!(
            route(&json!({"type": "server.connected", "data": {}}), "ses_1"),
            (false, true)
        );
    }
}
