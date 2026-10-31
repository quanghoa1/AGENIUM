//! Checks a designer's answer before the user sees it (ADR-151): a skill the library lacks becomes a proposal, a model
//! the user does not have is dropped, ids are made valid and unique, counts are bounded, and the template must pass
//! the same validation as one written by hand. What it changed is said in the draft's notes.

use std::collections::BTreeSet;

use agenium_core::RoleKind;
use serde::Deserialize;
use serde_json::Value;

use super::{NewSkill, RoleWhy, TeamDraft, REPORTS};
use crate::team_templates::{RoleSpec, TeamTemplate};

/// Above this many agents a draft is flagged as large (advice).
const LARGE_TEAM: u32 = 8;

/// What a designed team may use.
#[derive(Debug, Clone, Default)]
pub struct DesignCtx {
    /// Library skill names.
    pub skills: BTreeSet<String>,
    /// The user's models, `provider/model`.
    pub models: BTreeSet<String>,
    /// Template ids already used (built-in or the user's).
    pub taken: BTreeSet<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawAnswer {
    reply: String,
    questions: Vec<String>,
    team: Option<RawTeam>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTeam {
    id: String,
    name: String,
    description: String,
    report_template: String,
    pre_merge: String,
    why: String,
    roles: Vec<RawRole>,
    new_skills: Vec<NewSkill>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawRole {
    id: String,
    kind: String,
    count: u32,
    tags: Vec<String>,
    skills: Vec<String>,
    allowed_paths: Vec<String>,
    model: String,
    prompt: String,
    why: String,
}

/// Lowercase letters, digits and single hyphens, at most 40 characters.
pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.truncate(40);
    out.trim_matches('-').to_owned()
}

/// A worktree-relative glob (no absolute path, drive, `..` or backslash).
fn inside(p: &str) -> bool {
    let p = p.trim();
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && !p.split('/').any(|s| s == "..")
}

/// The designer's JSON answer: its reply, its questions, and the checked draft when it proposed a team.
pub(super) fn answer(
    v: Value,
    ctx: &DesignCtx,
) -> std::result::Result<(String, Vec<String>, Option<TeamDraft>), String> {
    let a: RawAnswer =
        serde_json::from_value(v).map_err(|e| format!("its fields are not as asked: {e}"))?;
    let questions: Vec<String> = a
        .questions
        .into_iter()
        .map(|q| q.trim().to_owned())
        .filter(|q| !q.is_empty())
        .collect();
    let draft = match a.team {
        Some(t) if !t.roles.is_empty() => Some(check(t, ctx)?),
        _ => None,
    };
    if draft.is_none() && questions.is_empty() && a.reply.trim().is_empty() {
        return Err("it has no reply, no question and no team".into());
    }
    Ok((a.reply.trim().to_owned(), questions, draft))
}

/// [`answer`]'s team check, for a team given as JSON.
pub fn check_team(v: Value, ctx: &DesignCtx) -> std::result::Result<TeamDraft, String> {
    let t: RawTeam = serde_json::from_value(v)
        .map_err(|e| format!("the team's fields are not as asked: {e}"))?;
    check(t, ctx)
}

fn check(t: RawTeam, ctx: &DesignCtx) -> std::result::Result<TeamDraft, String> {
    let mut notes = Vec::new();
    // The id: valid and not one the user already has (a draft never overwrites a template).
    let base = match slug(if t.id.trim().is_empty() {
        &t.name
    } else {
        &t.id
    }) {
        s if s.is_empty() => "designed-team".to_owned(),
        s => s,
    };
    let mut id = base.clone();
    let mut n = 2;
    while ctx.taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    if id != t.id {
        notes.push(format!("template id set to {id}"));
    }
    let report = if REPORTS.contains(&t.report_template.as_str()) {
        t.report_template.clone()
    } else {
        notes.push(format!(
            "report template {:?} is not one Agenium has: coding@1 is used",
            t.report_template
        ));
        "coding@1".to_owned()
    };
    let mut new_skills: Vec<NewSkill> = Vec::new();
    let mut add_new = |s: NewSkill, notes: &mut Vec<String>| {
        let name = slug(&s.name);
        if name.is_empty() {
            return;
        }
        if ctx.skills.contains(&name) {
            notes.push(format!("{name} is already in the library"));
            return;
        }
        match new_skills.iter_mut().find(|x| x.name == name) {
            Some(x) => {
                // First named in a role's skills, then described: the description fills in.
                if x.does.trim().is_empty() {
                    x.does = s.does;
                }
                if x.when.trim().is_empty() {
                    x.when = s.when;
                }
                for r in s.roles {
                    if !x.roles.contains(&r) {
                        x.roles.push(r);
                    }
                }
            }
            None => new_skills.push(NewSkill { name, ..s }),
        }
    };
    let mut roles = Vec::new();
    let mut whys = Vec::new();
    let mut ids = BTreeSet::new();
    for r in t.roles {
        let mut rid = match slug(&r.id) {
            s if s.is_empty() => "worker".to_owned(),
            s => s,
        };
        let mut k = 2;
        while ids.contains(&rid) {
            rid = format!("{}-{k}", slug(&r.id));
            k += 1;
        }
        ids.insert(rid.clone());
        let kind = r.kind.trim().to_lowercase();
        let (role_kind, planner) = match kind.as_str() {
            "lead" => (RoleKind::Lead, false),
            "planner" => (RoleKind::Worker, true),
            _ => (RoleKind::Worker, false),
        };
        let count = if role_kind == RoleKind::Lead || planner {
            1
        } else {
            r.count.clamp(1, 10)
        };
        let mut skills = Vec::new();
        for s in r.skills {
            let s = slug(&s);
            if s.is_empty() || skills.contains(&s) {
                continue;
            }
            if ctx.skills.contains(&s) {
                skills.push(s);
            } else {
                notes.push(format!(
                    "{s} is not in the library: proposed as a new skill for {rid}"
                ));
                add_new(
                    NewSkill {
                        name: s,
                        roles: vec![rid.clone()],
                        ..Default::default()
                    },
                    &mut notes,
                );
            }
        }
        let model = r.model.trim().to_owned();
        let default_model = if model.is_empty() {
            None
        } else if ctx.models.contains(&model) {
            Some(model)
        } else {
            notes.push(format!(
                "{rid}: model {model:?} is not one of yours; left for you to pick"
            ));
            None
        };
        let allowed_paths: Vec<String> = r
            .allowed_paths
            .into_iter()
            .filter(|p| {
                let ok = inside(p);
                if !ok {
                    notes.push(format!(
                        "{rid}: write scope {p:?} is outside the worktree; dropped"
                    ));
                }
                ok
            })
            .map(|p| p.trim().to_owned())
            .collect();
        let prompt = if r.prompt.trim().is_empty() {
            crate::role_library::role_skeleton()
        } else {
            r.prompt.trim().to_owned()
        };
        whys.push(RoleWhy {
            id: rid.clone(),
            why: r.why.trim().to_owned(),
        });
        roles.push(RoleSpec {
            id: rid,
            kind: role_kind,
            tags: r
                .tags
                .iter()
                .map(|x| slug(x))
                .filter(|x| !x.is_empty())
                .collect(),
            prompt,
            skills,
            allowed_paths,
            default_model,
            count,
            planner,
        });
    }
    for s in t.new_skills {
        add_new(s, &mut notes);
    }
    let agents: u32 = roles.iter().map(|r| r.count).sum();
    if agents > LARGE_TEAM {
        notes.push(format!(
            "{agents} agents is a large team: each one adds coordination and cost"
        ));
    }
    let template = TeamTemplate {
        id,
        version: "1.0.0".into(),
        name: if t.name.trim().is_empty() {
            "Designed team".into()
        } else {
            t.name.trim().to_owned()
        },
        description: t.description.trim().to_owned(),
        report_template: report,
        pre_merge: t.pre_merge.trim().to_owned(),
        roles,
    };
    template.validate().map_err(|e| e.to_string())?;
    Ok(TeamDraft {
        template,
        why: t.why.trim().to_owned(),
        roles: whys,
        new_skills,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> DesignCtx {
        DesignCtx {
            skills: ["writing-tests", "code-review", "a11y-audit"]
                .map(String::from)
                .into(),
            models: [
                "claude/claude-sonnet-5-5",
                "opencode-go/deepseek-v4.1-flash",
            ]
            .map(String::from)
            .into(),
            taken: ["coding", "web-team"].map(String::from).into(),
        }
    }

    #[test]
    fn a_draft_keeps_what_the_user_has_and_proposes_the_rest() {
        let v = json!({
            "reply": "Here is a team.",
            "team": {
                "id": "web-team", "name": "Web team", "report_template": "coding@1",
                "roles": [
                    {"id": "lead", "kind": "lead", "count": 3, "model": "claude/claude-sonnet-5-5", "prompt": "## Mission\nLead."},
                    {"id": "Front End", "kind": "worker", "count": 14, "skills": ["a11y-audit", "seo-basics", "a11y-audit"],
                     "allowed_paths": ["web/**", "../etc", "C:/x"], "model": "openai/gpt-9"},
                    {"id": "qa", "kind": "worker", "skills": ["writing-tests"], "model": "opencode-go/deepseek-v4.1-flash"}
                ],
                "new_skills": [{"name": "seo-basics", "does": "SEO checks", "when": "before release", "roles": ["front-end"]},
                               {"name": "code-review", "does": "x", "when": "y"}]
            }
        });
        let (reply, questions, draft) = answer(v, &ctx()).unwrap();
        assert_eq!(reply, "Here is a team.");
        assert!(questions.is_empty());
        let d = draft.unwrap();
        let t = &d.template;
        // A taken id is never reused: the draft cannot overwrite a template.
        assert_eq!(t.id, "web-team-2");
        assert_eq!(t.roles[0].count, 1, "a Lead is one agent");
        let fe = &t.roles[1];
        assert_eq!(fe.id, "front-end");
        assert_eq!(fe.count, 10, "at most 10 per role");
        assert_eq!(fe.skills, vec!["a11y-audit"], "only library skills, once");
        assert_eq!(fe.allowed_paths, vec!["web/**"]);
        assert_eq!(
            fe.default_model, None,
            "a model the user does not have is dropped"
        );
        assert_eq!(
            t.roles[2].default_model.as_deref(),
            Some("opencode-go/deepseek-v4.1-flash")
        );
        // The unknown skill is proposed once, for the role that wanted it; a library skill is not proposed.
        assert_eq!(d.new_skills.len(), 1);
        assert_eq!(d.new_skills[0].name, "seo-basics");
        assert_eq!(d.new_skills[0].roles, vec!["front-end"]);
        assert_eq!(
            d.new_skills[0].does, "SEO checks",
            "named in a role, described later"
        );
        assert!(d.notes.iter().any(|n| n.contains("openai/gpt-9")));
        assert!(d.notes.iter().any(|n| n.contains("../etc")));
        assert!(d.notes.iter().any(|n| n.contains("large team")));
        t.validate().unwrap();
    }

    #[test]
    fn questions_alone_are_an_answer_but_a_team_must_be_valid() {
        let (_, q, d) = answer(
            json!({"reply": "Two things first.", "questions": ["Back end?", " "]}),
            &ctx(),
        )
        .unwrap();
        assert_eq!(q, vec!["Back end?"]);
        assert!(d.is_none());
        // No worker: the same validation as a template written by hand refuses it, so the designer is asked again.
        let e = answer(
            json!({"team": {"name": "x", "roles": [{"id": "lead", "kind": "lead"}]}}),
            &ctx(),
        )
        .unwrap_err();
        assert!(e.contains("worker"), "{e}");
        assert!(answer(json!({}), &ctx()).is_err());
        assert_eq!(slug("  My Web -- Team!! "), "my-web-team");
    }
}
