//! An agent's copy of a skill that did not rebase cleanly onto its updated base (a plugin's vendor update, a library
//! skill refined): the helper drafts the merge, the user reads or edits it and accepts it (ADR-157, 2026-11-17).

use agenium_orchestrator::skill_overlay::OverlayMergeDraft;

use super::*;

/// The helper's draft of `agent`'s copy of `skill` on the current base, keeping what the agent learned. Nothing is
/// written.
#[tauri::command]
pub async fn draft_overlay_merge(
    d: State<'_, Desktop>,
    agent: String,
    skill: String,
    model: ModelArg,
) -> R<OverlayMergeDraft> {
    let o = d.opened()?;
    let id: AgentId = agent
        .parse()
        .map_err(|_| format!("bad agent id {agent:?}"))?;
    let (backend, binding) = model.binding()?;
    o.rt.draft_overlay_merge(id, &skill, backend, &binding)
        .await
        .map_err(err)
}

/// The user accepts `text` as `agent`'s copy of `skill` (the helper's draft, or their edit of it).
#[tauri::command]
pub fn accept_overlay_merge(
    d: State<'_, Desktop>,
    agent: String,
    skill: String,
    text: String,
) -> R<()> {
    let o = d.opened()?;
    let id: AgentId = agent
        .parse()
        .map_err(|_| format!("bad agent id {agent:?}"))?;
    o.app.accept_overlay_merge(id, &skill, &text).map_err(err)
}
