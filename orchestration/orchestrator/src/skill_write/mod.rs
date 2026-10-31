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
pub type SkillDoc = crate::documents::Attached;

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
    /// The library skill this draft improves (a merge, ADR-152): saving it edits that skill.
    pub extends: Option<String>,
    /// That skill's SKILL.md as it is now, to compare with the draft.
    pub original: Option<String>,
    /// Library skills on the same or a near subject (a new draft): merge into one rather than add a second skill.
    pub similar: Vec<crate::skill_similar::SimilarSkill>,
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
    extends: String,
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
    let extends = crate::team_design::check::slug(&s.extends);
    let extends = if extends.is_empty() {
        None
    } else if c.library.contains(&extends) {
        Some(extends)
    } else {
        return Err(format!(
            "\"extends\" names {:?}, which is not a library skill",
            s.extends
        ));
    };
    let name = match &extends {
        Some(e) => e.clone(),
        None => crate::team_design::check::slug(&s.name),
    };
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
        extends,
        original: None,
        similar: Vec::new(),
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
    if !docs.is_empty() {
        lead.push_str(&crate::documents::material(docs, DOCS_MAX_CHARS));
        lead.push_str("\n\n");
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
    #[allow(clippy::too_many_arguments)]
    pub async fn write_skill(
        &self,
        backend: BackendKind,
        model: &ModelBinding,
        turns: &[ChatTurn],
        docs: &[SkillDoc],
        seed: Option<&NewSkill>,
        merge_into: Option<&str>,
        session: Option<&str>,
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
        let library = self.app.skill_registry().library();
        let mut catalogue = String::from(
            "\n\n# The library's skills (improve one of these rather than adding a second skill on its job)\n",
        );
        for sk in &library.skills {
            catalogue.push_str(&format!(
                "- {}: {}\n",
                sk.meta.name,
                sk.meta.description.trim()
            ));
        }
        // The skills closest to what is asked, in full, so the writer merges knowingly (ADR-152).
        let asked = format!(
            "{} {}",
            turns
                .iter()
                .find(|t| t.role == "user")
                .map(|t| t.text.as_str())
                .unwrap_or(""),
            seed.map(|s| format!("{} {}", s.does, s.when))
                .unwrap_or_default()
        );
        for close in self.app.skills_for_request(&asked, 3) {
            let dir = self
                .app
                .skill_registry()
                .dir_of(&agenium_skills::Owner::Library)
                .join(&close.name);
            if let Ok(text) = std::fs::read_to_string(dir.join(agenium_skills::SKILL_FILE)) {
                catalogue.push_str(&format!(
                    "\n<close_skill name={:?} level={:?}>\n{text}\n</close_skill>\n",
                    close.name, close.level
                ));
            }
        }
        let system = format!("{}{catalogue}", prompt::RULES);
        let mut turns = with_material(turns, docs, seed);
        // The user chose to merge the draft into a library skill: its text goes with the request.
        if let Some(name) = merge_into {
            let dir = self
                .app
                .skill_registry()
                .dir_of(&agenium_skills::Owner::Library)
                .join(name);
            let text = std::fs::read_to_string(dir.join(agenium_skills::SKILL_FILE))
                .map_err(|_| OrchError::NotFound(format!("library skill {name}")))?;
            let mut files: Vec<String> = ["references", "scripts", "assets"]
                .iter()
                .flat_map(|sub| {
                    std::fs::read_dir(dir.join(sub))
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter(|e| e.path().is_file())
                        .map(move |e| format!("{sub}/{}", e.file_name().to_string_lossy()))
                })
                .collect();
            files.sort();
            if let Some(last) = turns.last_mut() {
                last.text.push_str(&prompt::merge(name, &text, &files));
            }
        }
        let mut msgs: Vec<Message> = messages(&turns)?;
        let mut last = String::new();
        for attempt in 0..2 {
            let a = self
                .side_chat(
                    "skill",
                    session,
                    backend,
                    model,
                    &system,
                    msgs.clone(),
                    MAX_TOKENS,
                )
                .await?;
            let why = match agenium_memory::pipeline::json_object(&a.text) {
                None => "it is not a JSON object".to_owned(),
                Some(v) => match check_answer(v, &check) {
                    Ok((reply, questions, mut draft)) => {
                        if let Some(d) = draft.as_mut() {
                            self.relate(d, merge_into);
                        }
                        return Ok(SkillWrite {
                            reply,
                            questions,
                            draft,
                            raw: a.text,
                            model: a.model,
                        });
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

impl Runtime {
    /// What a draft is to the library: the text of the skill it improves (a merge), or the library skills on the same or
    /// a near subject for a new one (local embedding model and subject words: no model call).
    /// `merge_into`: the user asked to merge into that skill, so the draft improves it whatever the model wrote; a draft
    /// that took a library skill's name improves that skill too (seen with DeepSeek: the name, no `extends`).
    fn relate(&self, d: &mut SkillDraft, merge_into: Option<&str>) {
        let into = match merge_into {
            Some(m) => Some(m.to_owned()),
            None if d.extends.is_none() && d.exists => Some(d.name.clone()),
            None => None,
        };
        if let Some(name) = into.filter(|n| d.extends.as_deref() != Some(n.as_str())) {
            d.extends = Some(name.clone());
            d.name = name;
            d.exists = true;
            d.skill_md = skill_md(&d.name, &d.description, &d.tags, &d.body);
        }
        if let Some(e) = &d.extends {
            let dir = self
                .app
                .skill_registry()
                .dir_of(&agenium_skills::Owner::Library)
                .join(e);
            d.original = std::fs::read_to_string(dir.join(agenium_skills::SKILL_FILE)).ok();
            return;
        }
        d.similar = self
            .app
            .similar_skills(&d.description, &d.body, Some(&d.name), 3);
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
    fn a_draft_that_improves_a_library_skill_takes_its_name() {
        let skill = |extends: &str| json!({"skill": {"name": "review-helper", "description": "Reviews code before merge. Use when a change is ready.", "body": BODY.replace("1. Read references/checklist.md.\n", ""), "extends": extends}});
        let (_, _, d) = run(skill("code-review")).unwrap();
        let d = d.unwrap();
        assert_eq!(
            (d.name.as_str(), d.extends.as_deref(), d.exists),
            ("code-review", Some("code-review"), true)
        );
        assert!(d.skill_md.starts_with("---\nname: code-review\n"));
        assert!(run(skill("no-such-skill"))
            .unwrap_err()
            .contains("not a library skill"));
        let (_, _, d) = run(skill("")).unwrap();
        assert_eq!(d.unwrap().extends, None);
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
