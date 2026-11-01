//! Helper sessions (ADR-152): each conversation with the team designer or the skill writer is kept in the root, with
//! the model it ran on, the documents attached, the last draft and what was saved from it, so it can be reopened and
//! continued later. A discussion of an agent's skill proposal is a session that starts from the proposal.

use agenium_store::core::helper_sessions::{self, HelperSession};
use serde_json::Value;

use crate::app::App;
use crate::{OrchError, Result};

/// Sessions listed at most.
pub const LIST_MAX: usize = 50;
/// A session's stored size at most (turns, transcript, attachments and draft together).
const SESSION_MAX_BYTES: usize = 2 * 1024 * 1024;

fn kind_ok(kind: &str) -> Result<()> {
    if matches!(kind, "design" | "skill") {
        Ok(())
    } else {
        Err(OrchError::Invalid(format!("{kind:?} is not a helper")))
    }
}

/// A title the user gives a session, at most this long.
const TITLE_MAX_CHARS: usize = 120;

/// The session's title when the user has not named it: its draft's name once it has a draft (the skill's name, the
/// team's name), else the first thing the user wrote, on one line.
fn title_for(turns: &Value, draft: Option<&Value>) -> String {
    let drafted = draft.and_then(|d| {
        d["template"]["name"]
            .as_str()
            .or_else(|| d["extends"].as_str())
            .or_else(|| d["name"].as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
    });
    drafted.unwrap_or_else(|| title_of(turns))
}

/// The first thing the user wrote, on one line.
fn title_of(turns: &Value) -> String {
    let first = turns
        .as_array()
        .and_then(|t| t.iter().find(|x| x["role"] == "user"))
        .and_then(|x| x["text"].as_str())
        .unwrap_or("");
    let one = first.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = one.chars().take(90).collect();
    if short.len() < one.len() {
        format!("{short}…")
    } else {
        short
    }
}

impl App {
    /// A new, empty session of `kind`, from `source` (an agent's proposal, a skill the designer proposed).
    pub fn new_session(
        &self,
        kind: &str,
        source: Option<Value>,
        model: Value,
    ) -> Result<HelperSession> {
        kind_ok(kind)?;
        let now = agenium_core::now_iso();
        let s = HelperSession {
            id: ulid::Ulid::new().to_string(),
            kind: kind.to_owned(),
            title: String::new(),
            named: false,
            model,
            turns: Value::Array(Vec::new()),
            transcript: Value::Array(Vec::new()),
            attachments: Value::Array(Vec::new()),
            draft: None,
            source,
            status: "open".into(),
            result: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.store().commit(None, "user", |w| {
            helper_sessions::put(w.tx, &s).map_err(OrchError::from)
        })?;
        Ok(s)
    }

    pub fn session(&self, id: &str) -> Result<HelperSession> {
        self.store()
            .read(|c| helper_sessions::get(c, id))?
            .ok_or_else(|| OrchError::NotFound(format!("session {id}")))
    }

    pub fn sessions(&self, kind: &str) -> Result<Vec<HelperSession>> {
        kind_ok(kind)?;
        Ok(self
            .store()
            .read(|c| helper_sessions::list(c, kind, LIST_MAX))?)
    }

    /// Keeps the session after a turn: what was said, the documents, the draft and the model. `result` is set when
    /// something was saved from it (the session is then `saved`).
    #[allow(clippy::too_many_arguments)]
    pub fn save_session(
        &self,
        id: &str,
        model: Value,
        turns: Value,
        transcript: Value,
        attachments: Value,
        draft: Option<Value>,
        result: Option<String>,
    ) -> Result<HelperSession> {
        let mut s = self.session(id)?;
        let size = turns.to_string().len()
            + transcript.to_string().len()
            + attachments.to_string().len()
            + draft.as_ref().map_or(0, |d| d.to_string().len());
        if size > SESSION_MAX_BYTES {
            return Err(OrchError::Invalid(
                "the session is too large to keep".into(),
            ));
        }
        // A turn that only asks keeps the draft the session has.
        if draft.is_some() {
            s.draft = draft;
        }
        if !s.named {
            s.title = title_for(&turns, s.draft.as_ref());
        }
        s.model = model;
        s.turns = turns;
        s.transcript = transcript;
        s.attachments = attachments;
        if let Some(r) = result {
            s.result = Some(r);
            s.status = "saved".into();
        }
        s.updated_at = agenium_core::now_iso();
        self.store().commit(None, "user", |w| {
            helper_sessions::put(w.tx, &s).map_err(OrchError::from)
        })?;
        Ok(s)
    }

    /// Records what was saved from the session (a skill, a template id); the session becomes `saved`.
    pub fn finish_session(&self, id: &str, result: &str) -> Result<HelperSession> {
        let mut s = self.session(id)?;
        s.result = Some(result.to_owned());
        s.status = "saved".into();
        s.updated_at = agenium_core::now_iso();
        self.store().commit(None, "user", |w| {
            helper_sessions::put(w.tx, &s).map_err(OrchError::from)
        })?;
        Ok(s)
    }

    /// Names the session (it keeps that name); an empty name gives it back its own title (its draft's name, else its
    /// first message).
    pub fn rename_session(&self, id: &str, title: &str) -> Result<HelperSession> {
        let mut s = self.session(id)?;
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.chars().count() > TITLE_MAX_CHARS {
            return Err(OrchError::Invalid(format!(
                "a session name is at most {TITLE_MAX_CHARS} characters"
            )));
        }
        if title.is_empty() {
            s.named = false;
            s.title = title_for(&s.turns, s.draft.as_ref());
        } else {
            s.named = true;
            s.title = title;
        }
        s.updated_at = agenium_core::now_iso();
        self.store().commit(None, "user", |w| {
            helper_sessions::put(w.tx, &s).map_err(OrchError::from)
        })?;
        Ok(s)
    }

    pub fn remove_session(&self, id: &str) -> Result<()> {
        self.store().commit(None, "user", |w| {
            helper_sessions::remove(w.tx, id).map_err(OrchError::from)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_first_user_line() {
        let t = serde_json::json!([{"role": "user", "text": "  Một team\nlàm web  "}, {"role": "assistant", "text": "{}"}]);
        assert_eq!(title_of(&t), "Một team làm web");
        let long = serde_json::json!([{"role": "user", "text": "x".repeat(200)}]);
        assert!(title_of(&long).ends_with('…') && title_of(&long).chars().count() == 91);
        assert_eq!(title_of(&serde_json::json!([])), "");
    }
}
