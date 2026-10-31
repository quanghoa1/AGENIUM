//! The skill writer (ADR-151): the user says what a skill should do, adds documents if they like, and a model of their
//! choosing (an API provider or a vendor CLI) writes it to Agenium's skill contract — the five sections, a description
//! that says when, long detail in `references/`. Agenium checks the draft (shape, name, files named by the body, no
//! secret) before the user sees it; the draft opens in the skill editor, where the user evaluates and saves it like a
//! skill they wrote. Nothing is saved here.

mod prompt;

use std::collections::BTreeSet;

use agenium_core::{BackendKind, ModelBinding};
use agenium_llm::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runtime::Runtime;
use crate::team_design::{messages, ChatTurn, NewSkill};
use crate::{OrchError, Result};

/// The documents' text sent at most (the rest is cut, and the model is told).
pub const DOCS_MAX_CHARS: usize = 120_000;
/// A file of the skill at most.
const FILE_MAX_CHARS: usize = 60_000;
const MAX_TOKENS: u32 = 16_000;

/// A document the user gave the writer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDoc {
    pub name: String,
    pub text: String,
}

/// A file of a drafted skill (`references/…` or `scripts/…`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillFileDraft {
    pub path: String,
    pub text: String,
}

/// A checked skill draft.
#[derive(Debug, Clone, Serialize)]
pub struct SkillDraft {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub body: String,
    pub files: Vec<SkillFileDraft>,
    /// The whole SKILL.md (frontmatter and body), as the editor opens it.
    pub skill_md: String,
    /// The library already has a skill with this name (saving it means editing that one, or renaming).
    pub exists: bool,
    /// The shape and description advice (ADR-88/141), as Evaluate would show it.
    pub notes: Vec<String>,
}

/// The writer's answer to one turn.
#[derive(Debug, Clone, Serialize)]
pub struct SkillWrite {
    pub reply: String,
    pub questions: Vec<String>,
    pub draft: Option<SkillDraft>,
    pub raw: String,
    pub model: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawAnswer {
    reply: String,
    questions: Vec<String>,
    skill: Option<RawSkill>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawSkill {
    name: String,
    description: String,
    tags: Vec<String>,
    body: String,
    files: Vec<SkillFileDraft>,
}

/// What the check needs from the app: the library's names, the shape advice, and the secret test.
pub struct SkillCheck<'a> {
    pub library: BTreeSet<String>,
    pub lint: &'a (dyn Fn(&str, &str, &str) -> Vec<String> + Send + Sync),
    pub secret: &'a (dyn Fn(&str) -> bool + Send + Sync),
}

/// A file path a skill may hold: `references/` or `scripts/`, then plain segments.
fn file_ok(p: &str) -> bool {
    let mut parts = p.split('/');
    matches!(parts.next(), Some("references" | "scripts"))
        && p.split('/').count() >= 2
        && parts.all(|s| {
            !s.is_empty()
                && !s.starts_with('.')
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
}

/// The SKILL.md the editor opens: the frontmatter fields, then the body.
pub fn skill_md(name: &str, description: &str, tags: &[String], body: &str) -> String {
    let one = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut fm = format!("name: {}\ndescription: {}", one(name), one(description));
    if !tags.is_empty() {
        fm.push_str(&format!("\ntags: {}", tags.join(" ")));
    }
    format!("---\n{fm}\n---\n\n{}\n", body.trim())
}

/// The writer's JSON answer: its reply, questions and the checked draft. `Err` says why it must be asked again.
pub fn check_answer(
    v: Value,
    c: &SkillCheck,
) -> std::result::Result<(String, Vec<String>, Option<SkillDraft>), String> {
    let a: RawAnswer =
        serde_json::from_value(v).map_err(|e| format!("its fields are not as asked: {e}"))?;
    let questions: Vec<String> = a
        .questions
        .into_iter()
        .map(|q| q.trim().to_owned())
        .filter(|q| !q.is_empty())
        .collect();
    let draft = match a.skill {
        Some(s) if !s.body.trim().is_empty() || !s.name.trim().is_empty() => Some(check(s, c)?),
        _ => None,
    };
    if draft.is_none() && questions.is_empty() && a.reply.trim().is_empty() {
        return Err("it has no reply, no question and no skill".into());
    }
    Ok((a.reply.trim().to_owned(), questions, draft))
}

fn check(s: RawSkill, c: &SkillCheck) -> std::result::Result<SkillDraft, String> {
    let name = crate::team_design::check::slug(&s.name);
    if name.is_empty() {
        return Err("the skill has no name".into());
    }
    let body = s.body.trim().to_owned();
    if body.is_empty() {
        return Err("the skill has no body".into());
    }
    let description = s
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut files: Vec<SkillFileDraft> = Vec::new();
    for f in s.files {
        let path = f.path.trim().trim_start_matches("./").to_owned();
        if !file_ok(&path) {
            return Err(format!(
                "file {path:?} is not under references/ or scripts/ with a plain name"
            ));
        }
        if files.iter().any(|x| x.path == path) {
            continue;
        }
        if f.text.chars().count() > FILE_MAX_CHARS {
            return Err(format!("{path} is longer than {FILE_MAX_CHARS} characters"));
        }
        if !body.contains(&path) {
            return Err(format!(
                "the body never names {path}: a step must say when to read it"
            ));
        }
        files.push(SkillFileDraft { path, text: f.text });
    }
    let all = std::iter::once(format!("{description}\n{body}"))
        .chain(files.iter().map(|f| f.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    if (c.secret)(&all) {
        return Err("it holds something that looks like a secret (a key, a token or a password): leave it out".into());
    }
    let notes = (c.lint)(&name, &description, &body);
    if let Some(missing) = notes
        .iter()
        .find(|n| n.starts_with("missing section") || n.contains("is empty"))
    {
        return Err(format!("the body has a {missing}: it needs ## Job, ## When to use, ## Steps, ## Tools and ## Proof"));
    }
    let tags: Vec<String> = s
        .tags
        .iter()
        .map(|t| crate::team_design::check::slug(t))
        .filter(|t| !t.is_empty())
        .take(6)
        .collect();
    Ok(SkillDraft {
        skill_md: skill_md(&name, &description, &tags, &body),
        exists: c.library.contains(&name),
        name,
        description,
        tags,
        body,
        files,
        notes,
    })
}

/// The first user turn with the documents (cut to [`DOCS_MAX_CHARS`]) and the designer's proposal, if any.
fn with_material(turns: &[ChatTurn], docs: &[SkillDoc], seed: Option<&NewSkill>) -> Vec<ChatTurn> {
    let mut lead = String::new();
    if let Some(s) = seed {
        lead.push_str(&format!(
            "The team designer proposed this skill: name {:?}; what it does: {}; when to use it: {}; for the roles: {}.\n\n",
            s.name,
            s.does,
            s.when,
            s.roles.join(", ")
        ));
    }
    let mut left = DOCS_MAX_CHARS;
    for d in docs {
        let n = d.text.chars().count();
        let text: String = d.text.chars().take(left).collect();
        let cut = if n > left {
            "\n[the rest of this document was cut]"
        } else {
            ""
        };
        left = left.saturating_sub(n);
        lead.push_str(&format!(
            "<document name={:?}>\n{text}{cut}\n</document>\n\n",
            d.name
        ));
    }
    let mut out = turns.to_vec();
    if let Some(first) = out.iter_mut().find(|t| t.role == "user") {
        first.text = format!("{lead}{}", first.text);
    }
    out
}

impl Runtime {
    /// One turn of the skill writer with the user's chosen model. An answer that is not JSON, or a skill that breaks
    /// the contract, is sent back once with the reason.
    pub async fn write_skill(
        &self,
        backend: BackendKind,
        model: &ModelBinding,
        turns: &[ChatTurn],
        docs: &[SkillDoc],
        seed: Option<&NewSkill>,
    ) -> Result<SkillWrite> {
        let app = self.app.clone();
        let lint = move |n: &str, d: &str, b: &str| app.lint_skill_md(n, d, b);
        let redactor = self.app.redactor().clone();
        let secret =
            move |t: &str| redactor.redact(t) != t || !agenium_secrets::scan_text(t).is_empty();
        let check = SkillCheck {
            library: self
                .app
                .skill_registry()
                .library()
                .skills
                .iter()
                .map(|s| s.meta.name.clone())
                .collect(),
            lint: &lint,
            secret: &secret,
        };
        let mut msgs: Vec<Message> = messages(&with_material(turns, docs, seed))?;
        let mut last = String::new();
        for attempt in 0..2 {
            let a = self
                .side_chat(backend, model, prompt::RULES, msgs.clone(), MAX_TOKENS)
                .await?;
            let why = match agenium_memory::pipeline::json_object(&a.text) {
                None => "it is not a JSON object".to_owned(),
                Some(v) => match check_answer(v, &check) {
                    Ok((reply, questions, draft)) => {
                        return Ok(SkillWrite {
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
            "the skill writer's answer could not be used: {last}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(v: Value) -> std::result::Result<(String, Vec<String>, Option<SkillDraft>), String> {
        let lint = |n: &str, d: &str, b: &str| {
            let mut x = crate::skill_shape::lint_description(n, d);
            x.extend(crate::skill_shape::lint_skill(n, b, &[]));
            x
        };
        let secret = |t: &str| t.contains("sk-ant-");
        check_answer(
            v,
            &SkillCheck {
                library: ["code-review".to_owned()].into(),
                lint: &lint,
                secret: &secret,
            },
        )
    }

    const BODY: &str = "## Job\nAudit a page for SEO.\n\n## When to use\nBefore a release.\n\n## Steps\n1. Read references/checklist.md.\n2. Check titles.\n\n## Tools\nlighthouse.\n\n## Proof\nThe report.";

    #[test]
    fn a_skill_in_the_contract_comes_back_ready_for_the_editor() {
        let (reply, _, d) = run(json!({"reply": "Done.", "skill": {
            "name": "SEO Audit", "description": "Audits a web page for search engines. Use when a page is about to ship.",
            "tags": ["Web", "seo"], "body": BODY,
            "files": [{"path": "references/checklist.md", "text": "- title"}, {"path": "./references/checklist.md", "text": "dup"}]
        }}))
        .unwrap();
        assert_eq!(reply, "Done.");
        let d = d.unwrap();
        assert_eq!(d.name, "seo-audit");
        assert_eq!(d.tags, vec!["web", "seo"]);
        assert_eq!(d.files.len(), 1, "a path once");
        assert!(!d.exists);
        assert!(d
            .skill_md
            .starts_with("---\nname: seo-audit\ndescription: Audits a web page"));
        assert!(d.skill_md.contains("tags: web seo\n---\n\n## Job"));
        assert!(d.notes.is_empty(), "{:?}", d.notes);
    }

    #[test]
    fn a_skill_that_breaks_the_contract_is_sent_back_with_why() {
        let skill = |body: &str, files: Value| json!({"skill": {"name": "x-skill", "description": "Does x. Use when y happens often.", "body": body, "files": files}});
        let e = run(skill("## Job\nx", json!([]))).unwrap_err();
        assert!(e.contains("missing section"), "{e}");
        let e = run(skill(BODY, json!([{"path": "../evil.md", "text": ""}]))).unwrap_err();
        assert!(e.contains("references/"), "{e}");
        let e = run(skill(
            BODY,
            json!([{"path": "references/other.md", "text": ""}]),
        ))
        .unwrap_err();
        assert!(e.contains("never names"), "{e}");
        let e = run(skill(&format!("{BODY}\nkey sk-ant-abc"), json!([]))).unwrap_err();
        assert!(e.contains("secret"), "{e}");
        // Questions alone are an answer; a name the library has is flagged, not refused.
        let (_, q, d) = run(json!({"questions": ["Which stack?"]})).unwrap();
        assert_eq!(q, vec!["Which stack?"]);
        assert!(d.is_none());
        let (_, _, d) = run(json!({"skill": {"name": "code-review", "description": "Reviews code. Use when a change is ready.", "body": BODY.replace("1. Read references/checklist.md.\n", "")}})).unwrap();
        assert!(d.unwrap().exists);
    }

    #[test]
    fn the_documents_open_the_first_message_as_material() {
        let turns = vec![ChatTurn {
            role: "user".into(),
            text: "Write an SEO skill.".into(),
        }];
        let long = "a".repeat(DOCS_MAX_CHARS + 10);
        let docs = vec![
            SkillDoc {
                name: "guide.md".into(),
                text: "Titles under 60 chars.".into(),
            },
            SkillDoc {
                name: "big.txt".into(),
                text: long,
            },
        ];
        let seed = NewSkill {
            name: "seo-audit".into(),
            does: "SEO checks".into(),
            when: "before release".into(),
            roles: vec!["frontend".into()],
        };
        let t = with_material(&turns, &docs, Some(&seed));
        assert!(t[0]
            .text
            .starts_with("The team designer proposed this skill: name \"seo-audit\""));
        assert!(t[0]
            .text
            .contains("<document name=\"guide.md\">\nTitles under 60 chars."));
        assert!(t[0].text.contains("[the rest of this document was cut]"));
        assert!(t[0].text.ends_with("Write an SEO skill."));
        assert!(t[0].text.chars().count() < DOCS_MAX_CHARS + 2_000);
    }
}
