//! The helpers that draft with the user's chosen model (ADR-151): the team designer and the skill writer. Each call is
//! one turn of their chat; the draft opens in the template or skill editor, where the user saves it.

use agenium_orchestrator::skill_write::SkillDoc;
use agenium_orchestrator::team_design::{ChatTurn, NewSkill};

use super::*;

#[tauri::command]
pub async fn design_team(d: State<'_, Desktop>, model: ModelArg, turns: Vec<ChatTurn>) -> R<Value> {
    let o = d.opened()?;
    let (backend, binding) = model.binding()?;
    let design =
        o.rt.design_team(backend, &binding, &turns)
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
) -> R<Value> {
    let o = d.opened()?;
    let (backend, binding) = model.binding()?;
    let draft =
        o.rt.write_skill(backend, &binding, &turns, &documents, seed.as_ref())
            .await
            .map_err(err)?;
    serde_json::to_value(draft).map_err(|e| e.to_string())
}
