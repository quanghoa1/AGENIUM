//! A skill's quality on 12 criteria and its effect on real tickets (ADR-152): local checks, no model call.

use std::collections::BTreeMap;

use agenium_orchestrator::skill_quality::{Effect, Origin, Scorecard};

use super::*;

/// `origin`: `user`, `builtin`, `writer` (from the request), `writer-docs` (from documents) or `proposal:<contexts>`.
/// `skip`: the library skill being scored (left out of the closeness check; its effect is shown).
#[tauri::command]
pub fn skill_scorecard(
    d: State<'_, Desktop>,
    name: String,
    description: String,
    body: String,
    origin: String,
    skip: Option<String>,
) -> R<Scorecard> {
    let o = d.opened()?;
    let origin = match origin.as_str() {
        "builtin" => Origin::Builtin,
        "writer" => Origin::Writer { documents: false },
        "writer-docs" => Origin::Writer { documents: true },
        p if p.starts_with("proposal:") => Origin::Proposal {
            contexts: p["proposal:".len()..].parse().unwrap_or(0),
        },
        _ => Origin::User,
    };
    o.app
        .skill_scorecard(&name, &description, &body, origin, skip.as_deref())
        .map_err(err)
}

/// The effect of every skill loaded in a closed ticket, by name (the library cards).
#[tauri::command]
pub fn skill_effects(d: State<'_, Desktop>) -> R<BTreeMap<String, Effect>> {
    let o = d.opened()?;
    o.app.skill_effects().map_err(err)
}
