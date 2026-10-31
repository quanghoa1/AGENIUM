//! The team designer (ADR-151): the user describes the team they want in a short chat; a model of their choosing (an
//! API provider or a vendor CLI) proposes a team template — roles, counts, library skills, write scopes, a model per
//! role, and why — which Agenium checks before the user sees it. Nothing is created here: the draft opens in the
//! template editor, and skills the library lacks come back as proposals for the skill writer.

pub(crate) mod check;
mod prompt;

use std::collections::BTreeSet;

use agenium_core::{BackendKind, ModelBinding};
use agenium_llm::Message;
use serde::{Deserialize, Serialize};

pub use check::{check_team, DesignCtx};

use crate::app::App;
use crate::runtime::Runtime;
use crate::team_templates::TeamTemplate;
use crate::{OrchError, Result};

/// Report templates a designed team may use.
pub const REPORTS: [&str; 4] = ["coding@1", "research@1", "pentest@1", "content@1"];
/// Models of one source named to the designer at most.
const MODELS_PER_SOURCE: usize = 25;
/// The answer's token budget (a whole team with its ROLE.md texts).
const MAX_TOKENS: u32 = 16_000;

/// One turn of a helper chat (the team designer's or the skill writer's): the user's text, or the helper's whole
/// answer as it gave it (the next turn sends it back so a change request edits that draft).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurn {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

/// A skill the designer would give a role but the library does not have (the skill writer can write it).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewSkill {
    pub name: String,
    pub does: String,
    pub when: String,
    pub roles: Vec<String>,
}

/// Why a role is in the draft.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RoleWhy {
    pub id: String,
    pub why: String,
}

/// A checked team proposal.
#[derive(Debug, Clone, Serialize)]
pub struct TeamDraft {
    pub template: TeamTemplate,
    pub why: String,
    pub roles: Vec<RoleWhy>,
    pub new_skills: Vec<NewSkill>,
    /// What Agenium changed or wants the user to look at (an unknown model dropped, a large team...).
    pub notes: Vec<String>,
}

/// The designer's answer to one turn.
#[derive(Debug, Clone, Serialize)]
pub struct TeamDesign {
    /// Its message to the user.
    pub reply: String,
    pub questions: Vec<String>,
    pub draft: Option<TeamDraft>,
    /// The answer as given, for the next turn's history.
    pub raw: String,
    /// The model that answered.
    pub model: String,
}

/// The conversation as model messages (the last turn must be the user's).
pub(crate) fn messages(turns: &[ChatTurn]) -> Result<Vec<Message>> {
    if turns
        .last()
        .is_none_or(|t| t.role != "user" || t.text.trim().is_empty())
    {
        return Err(OrchError::Invalid(
            "the last turn must be the user's message".into(),
        ));
    }
    Ok(turns
        .iter()
        .map(|t| {
            if t.role == "assistant" {
                Message::assistant_text(t.text.clone())
            } else {
                Message::user_text(t.text.clone())
            }
        })
        .collect())
}

impl App {
    /// What a designed team may use: the library's skills, the user's models (`provider/model`), the template ids
    /// taken, and the catalogue text shown to the designer.
    pub fn design_ctx(&self) -> Result<(DesignCtx, String)> {
        let skills = self.skill_registry().library();
        let roles = crate::role_library::library();
        let cache = self.model_cache();
        let mut models = BTreeSet::new();
        let mut text = String::from("## Library roles (start a role from one when it fits)\n");
        for r in &roles {
            text.push_str(&format!(
                "- {} [{}]: {}\n",
                r.id,
                r.tags.join(", "),
                crate::role_library::mission(&r.prompt)
            ));
        }
        text.push_str("\n## Library skills (give roles these by name)\n");
        for s in &skills.skills {
            text.push_str(&format!(
                "- {}: {}\n",
                s.meta.name,
                s.meta.description.trim()
            ));
        }
        text.push_str(&format!("\n## Report templates\n{}\n", REPORTS.join(", ")));
        text.push_str("\n## The user's models (provider/model; CLI programs run the user's own subscription)\n");
        for (key, list) in &cache.models {
            let (kind, source) = key.split_once(':').unwrap_or(("api", key));
            if list.value.is_empty() || (kind == "api" && self.config().provider(source).is_none())
            {
                continue;
            }
            let ids: Vec<String> = list
                .value
                .iter()
                .take(MODELS_PER_SOURCE)
                .map(|m| format!("{source}/{}", m.id))
                .collect();
            models.extend(ids.iter().cloned());
            text.push_str(&format!(
                "- {} {source}: {}\n",
                if kind == "cli" { "CLI" } else { "API" },
                ids.join(", ")
            ));
        }
        if models.is_empty() {
            text.push_str("- (none listed yet: leave every model empty)\n");
        }
        let taken = self.team_templates()?.into_iter().map(|t| t.id).collect();
        Ok((
            DesignCtx {
                skills: skills.skills.iter().map(|s| s.meta.name.clone()).collect(),
                models,
                taken,
            },
            text,
        ))
    }
}

impl Runtime {
    /// One turn of the team designer with the user's chosen model. An answer that is not JSON or does not make a valid
    /// team is sent back once with the reason.
    pub async fn design_team(
        &self,
        backend: BackendKind,
        model: &ModelBinding,
        turns: &[ChatTurn],
    ) -> Result<TeamDesign> {
        let (ctx, catalogue) = self.app.design_ctx()?;
        let system = prompt::system(&catalogue);
        let mut msgs = messages(turns)?;
        let mut last = String::new();
        for attempt in 0..2 {
            let a = self
                .side_chat(backend, model, &system, msgs.clone(), MAX_TOKENS)
                .await?;
            let why = match agenium_memory::pipeline::json_object(&a.text) {
                None => "it is not a JSON object".to_owned(),
                Some(v) => match check::answer(v, &ctx) {
                    Ok((reply, questions, draft)) => {
                        return Ok(TeamDesign {
                            reply,
                            questions,
                            draft,
                            raw: a.text,
                            model: a.model,
                        })
                    }
                    Err(e) => e,
                },
            };
            last = why.clone();
            if attempt == 0 {
                msgs.push(Message::assistant_text(a.text));
                msgs.push(Message::user_text(prompt::repair(&why)));
            }
        }
        Err(OrchError::Invalid(format!(
            "the designer's answer could not be used: {last}"
        )))
    }
}
