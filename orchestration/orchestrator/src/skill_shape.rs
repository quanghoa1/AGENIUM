//! The shape of a SKILL.md and the built-in starter library (ADR-88). Mirrors the role shape (ADR-64) for skills, so
//! a skill the user writes and a skill an agent distils or upgrades follow the same contract. It embodies the
//! methodology-as-skill thinking: one job, when to use it, the steps, the tools and how to read their output, and —
//! the "hard stop" — the proof the agent must produce before it may claim the work is done. The checks are advice
//! shown with the skill; they never refuse a save.

use std::collections::BTreeSet;

/// The sections of a SKILL.md body, in order. "Job" and "Proof" are the two that matter most.
pub const SKILL_SECTIONS: [&str; 5] = ["Job", "When to use", "Steps", "Tools", "Proof"];

/// Above this the body crowds a run's context; long detail belongs in `references/` read on demand (tier 3).
pub const MAX_SKILL_BODY_CHARS: usize = 8000;

/// Two skills whose body word sets overlap this much are one skill twice.
const NEAR_DUPLICATE: f64 = 0.8;

/// `## Heading` -> lowercased heading, in file order (frontmatter excluded; pass the body).
fn headings(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            out.push((h.trim().to_lowercase(), String::new()));
        } else if let Some((_, b)) = out.last_mut() {
            b.push_str(line);
            b.push('\n');
        }
    }
    out
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// An empty SKILL.md body with the sections to fill in (the frontmatter is added by the caller).
pub fn skill_skeleton() -> String {
    SKILL_SECTIONS
        .iter()
        .map(|s| format!("## {s}\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Advice about a skill's body; `others` are the other skills (name, body). Never blocks a save. "Job" and "Proof"
/// missing are flagged first because they are the two that make a skill usable and verifiable.
pub fn lint_skill(name: &str, body: &str, others: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let found = headings(body);
    for s in SKILL_SECTIONS {
        match found.iter().find(|(h, _)| *h == s.to_lowercase()) {
            None => out.push(format!("missing section \"## {s}\"")),
            Some((_, b)) if b.trim().is_empty() => out.push(format!("section \"## {s}\" is empty")),
            Some(_) => {}
        }
    }
    let n = body.chars().count();
    if n > MAX_SKILL_BODY_CHARS {
        out.push(format!(
            "{n} characters (more than {MAX_SKILL_BODY_CHARS}): move detail into references/ and read it with load on demand"
        ));
    }
    let mine = words(body);
    for (other, theirs) in others {
        if other == name || mine.is_empty() {
            continue;
        }
        let theirs = words(theirs);
        let union = mine.union(&theirs).count();
        let same = mine.intersection(&theirs).count() as f64 / union.max(1) as f64;
        if same >= NEAR_DUPLICATE {
            out.push(format!(
                "nearly the same as skill {other} ({:.0}% of the words): use that skill or make the difference clear",
                same * 100.0
            ));
        }
    }
    out
}

/// Advice about a skill's name and description, the only text an agent (or the judge's skill hint) reads before it
/// picks a skill among many (Anthropic's skill authoring guide): say what the skill does and when to use it, in the
/// third person, without tags. Never blocks a save.
pub fn lint_description(name: &str, description: &str) -> Vec<String> {
    let mut out = Vec::new();
    let d = description.trim();
    let lower = format!(" {} ", d.to_lowercase());
    let when = [
        " use when",
        " use for",
        " use as",
        " use after",
        " use before",
        " use if",
        " use to ",
        " use it when",
        " use on",
        " when ",
    ];
    if !when.iter().any(|w| lower.contains(w)) {
        out.push(
            "the description does not say when to use the skill (\"… Use when …\"): an agent picks a skill by its \
             description alone, before it reads the body"
                .to_owned(),
        );
    }
    let first = lower.split_whitespace().next().unwrap_or("");
    if ["i", "i'm", "we", "you"].contains(&first)
        || [" i can ", " you can ", " we can ", " i will ", " lets you "]
            .iter()
            .any(|w| lower.contains(w))
    {
        out.push(
            "write the description in the third person, about what the skill does (\"Reviews a change …\"), not \
             \"I/you can …\""
                .to_owned(),
        );
    }
    if d.contains('<') && d.contains('>') {
        out.push("no tags (<…>) in the description: it is read as plain text".to_owned());
    }
    if d.chars().count() < 40 {
        out.push(
            "the description is too short to choose the skill by: say what it does and when"
                .to_owned(),
        );
    }
    let n = name.to_lowercase();
    if n.contains("anthropic") || n.contains("claude") {
        out.push("a vendor's name (anthropic, claude) in a skill name is refused by vendors' skill stores".to_owned());
    }
    out
}

/// A built-in starter skill shipped with Agenium and seeded into `<root>/skills` (ADR-88). The content lives as real
/// files under `templates/skills/<name>/` (SKILL.md plus any `references/` and `scripts/`), embedded at build time.
pub struct Builtin {
    pub name: &'static str,
    /// The full SKILL.md (frontmatter + body).
    pub skill_md: &'static str,
    /// Extra files (relative path -> content): `references/*.md`, `scripts/*`.
    pub files: &'static [(&'static str, &'static str)],
}

impl Builtin {
    pub fn skill_md(&self) -> String {
        self.skill_md.to_owned()
    }
}

/// `include_str!` a skill's SKILL.md and list its extra files, from `templates/skills/<name>/`.
macro_rules! skill {
    ($name:literal $(, $file:literal)* $(,)?) => {
        Builtin {
            name: $name,
            skill_md: include_str!(concat!("../templates/skills/", $name, "/SKILL.md")),
            files: &[ $( ($file, include_str!(concat!("../templates/skills/", $name, "/", $file))) ),* ],
        }
    };
}

/// The starter skill library: engineering methodologies written as one-job skills (our own words, learned from the
/// agency-agents catalogue's logic, not its files). Each ends with a Proof section — the evidence the agent must
/// produce before claiming the work is done.
pub const LIBRARY: &[Builtin] = &[
    skill!("code-review", "references/checklist.md"),
    skill!("root-cause-debugging"),
    skill!("perf-audit"),
    skill!("a11y-audit", "references/checklist.md"),
    skill!("api-contract-testing"),
    skill!("doc-writing"),
    skill!("writing-tests", "references/patterns.md"),
    skill!("refactoring", "references/catalog.md"),
    skill!("commit-and-pr", "scripts/check_commit.py"),
    skill!("migration-safety", "references/checklist.md"),
    skill!("secure-coding", "references/checklist.md"),
    skill!(
        "release-readiness",
        "references/checklist.md",
        "scripts/preflight.py"
    ),
    skill!(
        "app-ui-testing",
        "references/ui-review.md",
        "scripts/ui.mjs"
    ),
    skill!("bug-report", "references/gotchas.md"),
    skill!("reverse-engineering", "references/targets.md"),
    skill!(
        "binary-exploitation",
        "references/classes.md",
        "references/mitigations.md"
    ),
    skill!("firmware-analysis"),
    skill!("web-pentest", "references/classes.md"),
    skill!("recon-enumeration"),
];

use crate::app::App;
use crate::Result;

/// The body of a SKILL.md (everything after the frontmatter block), for linting and near-duplicate checks.
pub fn strip_frontmatter(text: &str) -> String {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            return rest[end + 4..].trim_start_matches(['\n', '\r']).to_owned();
        }
    }
    text.to_owned()
}

impl App {
    /// An empty SKILL.md body with the recommended sections (ADR-88), for the authoring form and the CLI.
    pub fn skill_skeleton(&self) -> String {
        skill_skeleton()
    }

    /// Advisory checks on a skill's body against the library (ADR-88): missing/empty sections, too long,
    /// near-duplicate. Never blocks a save; the same checks run when an agent proposes a skill update.
    pub fn lint_skill(&self, name: &str, body: &str) -> Vec<String> {
        lint_skill(name, body, &self.library_skill_bodies(name))
    }

    /// [`lint_skill`] with the checks of the skill's name and description ([`lint_description`]) first.
    pub fn lint_skill_md(&self, name: &str, description: &str, body: &str) -> Vec<String> {
        let mut notes = lint_description(name, description);
        notes.extend(self.lint_skill(name, body));
        notes
    }

    /// The body (frontmatter stripped) of an agent's overlay of `skill`, for a quality read after a self-upgrade
    /// (ADR-90). `None` if the agent has no overlay of it.
    pub fn agent_overlay_body(&self, agent: agenium_core::AgentId, skill: &str) -> Option<String> {
        let dir = self
            .skill_registry()
            .dir_of(&agenium_skills::Owner::Agent(agent.to_string()));
        std::fs::read_to_string(dir.join(skill).join(agenium_skills::SKILL_FILE))
            .ok()
            .map(|t| strip_frontmatter(&t))
    }

    /// The body (frontmatter stripped) of a library skill, for a shape check after import. `None` if absent.
    /// The shape checks of a library skill as saved: its description ([`lint_description`]) and its body.
    pub fn library_skill_notes(&self, name: &str) -> Vec<String> {
        let dir = self
            .skill_registry()
            .dir_of(&agenium_skills::Owner::Library);
        let text = std::fs::read_to_string(dir.join(name).join(agenium_skills::SKILL_FILE))
            .unwrap_or_default();
        let description = agenium_skills::parse(&text)
            .ok()
            .and_then(|(fm, _)| fm.get("description").map(str::to_owned))
            .unwrap_or_default();
        self.lint_skill_md(name, &description, &strip_frontmatter(&text))
    }

    pub fn library_skill_body(&self, name: &str) -> Option<String> {
        let dir = self
            .skill_registry()
            .dir_of(&agenium_skills::Owner::Library);
        std::fs::read_to_string(dir.join(name).join(agenium_skills::SKILL_FILE))
            .ok()
            .map(|t| strip_frontmatter(&t))
    }

    fn library_skill_bodies(&self, except: &str) -> Vec<(String, String)> {
        let reg = self.skill_registry();
        let dir = reg.dir_of(&agenium_skills::Owner::Library);
        reg.library()
            .skills
            .into_iter()
            .filter(|s| s.meta.name != except)
            .filter_map(|s| {
                std::fs::read_to_string(dir.join(&s.meta.name).join("SKILL.md"))
                    .ok()
                    .map(|t| (s.meta.name.clone(), strip_frontmatter(&t)))
            })
            .collect()
    }

    /// Seeds the built-in starter skill library into `<root>/skills` (ADR-88). Idempotent: writes only a skill whose
    /// folder is absent, so a user's edits and removals are never overwritten. Returns how many were written.
    pub fn seed_skill_library(&self) -> Result<usize> {
        let dir = self
            .skill_registry()
            .dir_of(&agenium_skills::Owner::Library);
        std::fs::create_dir_all(&dir)?;
        let mut n = 0;
        for s in LIBRARY {
            let folder = dir.join(s.name);
            if folder.join(agenium_skills::SKILL_FILE).exists() || folder.exists() {
                continue;
            }
            std::fs::create_dir_all(&folder)?;
            std::fs::write(folder.join(agenium_skills::SKILL_FILE), s.skill_md)?;
            for (rel, content) in s.files {
                let p = folder.join(rel);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&p, content)?;
            }
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_flags_every_section_until_filled() {
        let empty = skill_skeleton();
        assert_eq!(lint_skill("x", &empty, &[]).len(), SKILL_SECTIONS.len());
        let filled: String = SKILL_SECTIONS
            .iter()
            .map(|s| format!("## {s}\nSomething concrete about {s}.\n"))
            .collect();
        assert!(
            lint_skill("x", &filled, &[]).is_empty(),
            "a filled body passes"
        );
    }

    #[test]
    fn a_description_says_what_and_when_in_the_third_person() {
        let good = "Reviews a change for correctness before it is approved. Use when reviewing a pull request.";
        assert!(lint_description("code-review", good).is_empty());
        let notes = lint_description(
            "code-review",
            "Review a change for correctness and safety, with evidence.",
        );
        assert!(notes[0].contains("when to use"), "{notes:?}");
        let notes = lint_description(
            "x",
            "I can help you review code. Use when asked to review a change.",
        );
        assert!(
            notes.iter().any(|n| n.contains("third person")),
            "{notes:?}"
        );
        let notes = lint_description("claude-helper", "<b>Reviews</b> code. Use when asked.");
        assert_eq!(notes.len(), 3, "{notes:?}");
    }

    #[test]
    fn a_built_in_skill_passes_its_own_checks() {
        // Each skill against every other: no missing section, not too long, and not a near-duplicate of another.
        let bodies: Vec<(String, String)> = LIBRARY
            .iter()
            .map(|b| (b.name.to_owned(), strip_frontmatter(&b.skill_md())))
            .collect();
        let mut names = std::collections::HashSet::new();
        for b in LIBRARY {
            assert!(names.insert(b.name), "duplicate skill name {}", b.name);
            let notes = lint_skill(b.name, strip_frontmatter(&b.skill_md()).as_str(), &bodies);
            assert!(notes.is_empty(), "{}: {notes:?}", b.name);
            let (fm, _) = agenium_skills::parse(&b.skill_md()).unwrap();
            let notes = lint_description(b.name, fm.get("description").unwrap_or(""));
            assert!(notes.is_empty(), "{}: {notes:?}", b.name);
            assert!(b.skill_md().starts_with("---\nname: "));
            // Every built-in carries at least one domain tag (ADR-123) so an agent's fields are visible.
            let (fm, _) = agenium_skills::parse(&b.skill_md()).unwrap();
            assert!(
                !agenium_skills::parse_tags(fm.get("tags").unwrap_or("")).is_empty(),
                "{} has no tags",
                b.name
            );
        }
    }

    #[test]
    fn near_duplicate_is_flagged() {
        let body: String = SKILL_SECTIONS
            .iter()
            .map(|s| {
                format!(
                    "## {s}\nreview the change for correctness safety and clarity with evidence.\n"
                )
            })
            .collect();
        let others = vec![("code-review".to_string(), body.clone())];
        let notes = lint_skill("copy", &body, &others);
        assert!(
            notes.iter().any(|n| n.contains("nearly the same")),
            "{notes:?}"
        );
    }
}
