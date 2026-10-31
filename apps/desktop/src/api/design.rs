//! The helpers that draft with the user's chosen model (ADR-151): the team designer and the skill writer. Each call is
//! one turn of their chat; the draft opens in the template or skill editor, where the user saves it.

use agenium_orchestrator::skill_write::SkillDoc;
use agenium_orchestrator::team_design::{ChatTurn, NewSkill};

use super::*;

#[tauri::command]
pub async fn design_team(
    d: State<'_, Desktop>,
    model: ModelArg,
    turns: Vec<ChatTurn>,
    documents: Option<Vec<SkillDoc>>,
    session: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let (backend, binding) = model.binding()?;
    let design =
        o.rt.design_team(
            backend,
            &binding,
            &turns,
            &documents.unwrap_or_default(),
            session.as_deref(),
        )
        .await
        .map_err(err)?;
    serde_json::to_value(design).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_skill(
    d: State<'_, Desktop>,
    model: ModelArg,
    turns: Vec<ChatTurn>,
    documents: Vec<SkillDoc>,
    seed: Option<NewSkill>,
    merge_into: Option<String>,
    session: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let (backend, binding) = model.binding()?;
    let draft =
        o.rt.write_skill(
            backend,
            &binding,
            &turns,
            &documents,
            seed.as_ref(),
            merge_into.as_deref(),
            session.as_deref(),
        )
        .await
        .map_err(err)?;
    serde_json::to_value(draft).map_err(|e| e.to_string())
}

/// A document the user attached (its bytes, base64), read as Markdown (ADR-152).
#[tauri::command]
pub async fn read_document(d: State<'_, Desktop>, name: String, base64: String) -> R<Value> {
    use base64::Engine as _;
    let o = d.opened()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64.as_bytes())
        .map_err(|e| e.to_string())?;
    let c = o.app.read_document(&name, &bytes).await.map_err(err)?;
    Ok(json!({ "name": name, "text": c.text, "format": c.format, "notes": c.notes }))
}

/// A document dropped on the window or pasted as a path, read from disk as Markdown (ADR-152).
#[tauri::command]
pub async fn read_document_path(d: State<'_, Desktop>, path: String) -> R<Value> {
    let o = d.opened()?;
    let p = std::path::PathBuf::from(path.trim().trim_matches('"'));
    let meta = std::fs::metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", p.display()));
    }
    if meta.len() > agenium_documents::MAX_BYTES as u64 {
        return Err(format!(
            "{} is larger than {} MB",
            p.display(),
            agenium_documents::MAX_BYTES / 1024 / 1024
        ));
    }
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let c = o.app.read_document(&name, &bytes).await.map_err(err)?;
    Ok(json!({ "name": name, "text": c.text, "format": c.format, "notes": c.notes }))
}

/// What reads documents here: markitdown when it is installed, and the formats Agenium reads itself.
#[tauri::command]
pub fn document_readers(d: State<'_, Desktop>) -> R<Value> {
    let o = d.opened()?;
    serde_json::to_value(o.app.document_readers()).map_err(|e| e.to_string())
}

/// An agent's skill proposal (ADR-152): its SKILL.md and the evidence behind it, to open in the skill editor.
#[tauri::command]
pub fn skill_proposal(d: State<'_, Desktop>, id: String) -> R<Value> {
    let o = d.opened()?;
    let p = o.app.skill_proposal(&id).map_err(err)?;
    let agent = o.app.agent(p.agent).map(|a| a.name).unwrap_or_default();
    let mut v = serde_json::to_value(&p).map_err(|e| e.to_string())?;
    v["agent_name"] = json!(agent);
    Ok(v)
}

/// The user's decision on a skill proposal: `saved_as` names the library skill saved from it (switched on for the
/// proposing agent), or the proposal is declined with `note`.
#[tauri::command]
pub fn decide_skill_proposal(
    d: State<'_, Desktop>,
    id: String,
    approve: bool,
    saved_as: Option<String>,
    note: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let p = o
        .app
        .decide_skill_proposal(&id, approve, saved_as.as_deref(), note.as_deref())
        .map_err(err)?;
    serde_json::to_value(p).map_err(|e| e.to_string())
}

/// The user's last choice kept under `key` (ADR-152: a model picked in a form or a helper chat), or null.
#[tauri::command]
pub fn preference(d: State<'_, Desktop>, key: String) -> R<Value> {
    let o = d.opened()?;
    Ok(o.app.preference(&key).map_err(err)?.unwrap_or(Value::Null))
}

#[tauri::command]
pub fn set_preference(d: State<'_, Desktop>, key: String, value: Value) -> R<()> {
    let o = d.opened()?;
    o.app.set_preference(&key, &value).map_err(err)
}

/// A new helper session (ADR-152): `kind` is `design` or `skill`; `source` what it starts from.
#[tauri::command]
pub fn new_session(
    d: State<'_, Desktop>,
    kind: String,
    source: Option<Value>,
    model: Option<Value>,
) -> R<Value> {
    let o = d.opened()?;
    let s = o
        .app
        .new_session(&kind, source, model.unwrap_or(Value::Null))
        .map_err(err)?;
    serde_json::to_value(s).map_err(|e| e.to_string())
}

/// Keeps a session after a turn (and `result` once something was saved from it).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn save_session(
    d: State<'_, Desktop>,
    id: String,
    model: Value,
    turns: Value,
    transcript: Value,
    attachments: Value,
    draft: Option<Value>,
    result: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let s = o
        .app
        .save_session(&id, model, turns, transcript, attachments, draft, result)
        .map_err(err)?;
    serde_json::to_value(s).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sessions(d: State<'_, Desktop>, kind: String) -> R<Value> {
    let o = d.opened()?;
    serde_json::to_value(o.app.sessions(&kind).map_err(err)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn session(d: State<'_, Desktop>, id: String) -> R<Value> {
    let o = d.opened()?;
    serde_json::to_value(o.app.session(&id).map_err(err)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_session(d: State<'_, Desktop>, id: String) -> R<()> {
    let o = d.opened()?;
    o.app.remove_session(&id).map_err(err)
}

/// Records what was saved from a session (a skill name, a template id).
#[tauri::command]
pub fn finish_session(d: State<'_, Desktop>, id: String, result: String) -> R<Value> {
    let o = d.opened()?;
    serde_json::to_value(o.app.finish_session(&id, &result).map_err(err)?)
        .map_err(|e| e.to_string())
}

/// The helpers' model calls (ADR-152), newest first, for the usage page: each as a run of the helper (no ticket, no
/// agent), its outcome `Helper`.
#[tauri::command]
pub fn helper_calls(d: State<'_, Desktop>, limit: Option<usize>) -> R<Vec<Value>> {
    let o = d.opened()?;
    let calls = o
        .app
        .store()
        .read(|c| agenium_store::core::helper_calls::recent(c, limit.unwrap_or(1000)))
        .map_err(err)?;
    Ok(calls
        .into_iter()
        .map(|h| {
            json!({
                "id": h.id,
                "agent": format!("helper:{}", h.kind),
                "kind": h.kind,
                "provider": h.provider,
                "model": h.model,
                "api": h.api,
                "session": h.session,
                "started_at": h.started_at,
                "ended_at": h.ended_at,
                "outcome": "Helper",
                "steps": 1,
                "usage": h.usage,
            })
        })
        .collect())
}
