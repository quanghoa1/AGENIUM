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

/// A built-in starter skill shipped with Agenium and seeded into `<root>/skills` (ADR-88).
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

impl Builtin {
    /// The full SKILL.md (frontmatter + body).
    pub fn skill_md(&self) -> String {
        format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}",
            self.name,
            self.description,
            self.body.trim_start()
        )
    }
}

/// The starter skill library: engineering methodologies written as one-job skills (our own words, learned from the
/// agency-agents catalogue's logic, not its files). Each ends with a Proof section — the evidence the agent must
/// produce before claiming the work is done.
pub const LIBRARY: &[Builtin] = &[
    Builtin {
        name: "code-review",
        description: "Review a change for correctness, safety and clarity before it is approved; report issues with evidence.",
        body: CODE_REVIEW,
    },
    Builtin {
        name: "root-cause-debugging",
        description: "Find and fix the cause of a bug (not the symptom): reproduce, isolate, fix, and prove it with a regression test.",
        body: ROOT_CAUSE_DEBUGGING,
    },
    Builtin {
        name: "perf-audit",
        description: "Find and fix a performance problem by measuring first, changing one thing, and measuring again.",
        body: PERF_AUDIT,
    },
    Builtin {
        name: "a11y-audit",
        description: "Check a UI change against accessibility basics (keyboard, labels, contrast, roles) and fix what fails.",
        body: A11Y_AUDIT,
    },
    Builtin {
        name: "api-contract-testing",
        description: "Verify an API endpoint against its stated contract: status codes, shapes, errors and edge cases.",
        body: API_CONTRACT_TESTING,
    },
    Builtin {
        name: "doc-writing",
        description: "Write or update user-facing docs for a change so a reader can use it without the code.",
        body: DOC_WRITING,
    },
];

use crate::app::App;
use crate::{OrchError, Result};

/// The body of a SKILL.md (everything after the frontmatter block), for linting and near-duplicate checks.
pub(crate) fn strip_frontmatter(text: &str) -> String {
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

    /// The body (frontmatter stripped) of a library skill, for a shape check after import. `None` if absent.
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
        std::fs::create_dir_all(&dir).map_err(|e| OrchError::Invalid(e.to_string()))?;
        let mut n = 0;
        for s in LIBRARY {
            let folder = dir.join(s.name);
            if folder.join(agenium_skills::SKILL_FILE).exists() || folder.exists() {
                continue;
            }
            std::fs::create_dir_all(&folder).map_err(|e| OrchError::Invalid(e.to_string()))?;
            std::fs::write(folder.join(agenium_skills::SKILL_FILE), s.skill_md())
                .map_err(|e| OrchError::Invalid(e.to_string()))?;
            n += 1;
        }
        Ok(n)
    }
}

const CODE_REVIEW: &str = "## Job\nReview one change and decide whether it is correct, safe and clear enough to approve.\n\n## When to use\nBefore approving a ticket's work, or when asked to review a diff or a branch.\n\n## Steps\n1. Read the ticket's goal and acceptance criteria; review against them, not your own taste.\n2. Read the diff in full. For each hunk ask: does it do what the ticket asked, and nothing it should not?\n3. Check the risky edges: error paths, empty/None/large inputs, concurrency, resource cleanup, and anything touching secrets or money.\n4. Confirm tests exist for the new behaviour and that they would fail without the change.\n5. Prefer the smallest change that is correct; note reuse or simplification only when it clearly helps.\n\n## Tools\nUse read_file and grep to read the changed files and their callers; run the project's tests and linter through shell and read the output, do not assume it passed.\n\n## Proof\nList each issue as file:line with a one-line reason and, where you can, the concrete input that breaks it. State explicitly which acceptance criteria you verified and how (which test, which command output). A review with no evidence is not a review.\n";

const ROOT_CAUSE_DEBUGGING: &str = "## Job\nFix the cause of a bug, not its symptom.\n\n## When to use\nA ticket reports a defect, a test fails, or behaviour is wrong.\n\n## Steps\n1. Reproduce it first: find the smallest input or steps that trigger it, and capture the exact wrong output.\n2. Isolate: narrow to the function or line responsible (bisect, add a failing assertion, read the call path). Do not guess.\n3. Explain the cause in one sentence before changing code.\n4. Fix the cause. Avoid sleeps, broad try/except, or disabling the test — those hide the bug.\n5. Add a regression test that fails before the fix and passes after.\n\n## Tools\ngrep and read_file to trace the path; shell to run the reproducer and the test; read the real output each time.\n\n## Proof\nShow the reproducer, the one-sentence cause, and the regression test passing after and failing before the fix (paste both runs). \"Could not reproduce\" is a valid, honest outcome — say so instead of changing unrelated code.\n";

const PERF_AUDIT: &str = "## Job\nMake a measured part of the system faster without changing what it does.\n\n## When to use\nA ticket reports slowness, or a change must meet a latency/throughput target.\n\n## Steps\n1. Measure first: capture a baseline (time, allocations, queries) for a realistic input. No baseline, no change.\n2. Find the hot spot with a profiler or timing, not intuition.\n3. Change one thing at a time; keep behaviour identical (same outputs on the same inputs).\n4. Measure again against the baseline; keep the change only if it actually helps.\n\n## Tools\nshell to run the benchmark/profiler and the test suite; read the numbers; read_file to find the hot path.\n\n## Proof\nShow the before and after numbers for the same input, and confirm the behaviour is unchanged (tests still pass). A speedup claim with no before/after measurement does not count.\n";

const A11Y_AUDIT: &str = "## Job\nCheck a UI change against accessibility basics and fix what fails.\n\n## When to use\nA ticket adds or changes UI (components, forms, navigation, colour).\n\n## Steps\n1. Keyboard: every control reachable and operable with Tab/Enter/Escape; visible focus; no trap.\n2. Names: inputs have labels; icon-only buttons have an accessible name; images have alt text.\n3. Roles/structure: correct semantic elements or ARIA roles; headings in order.\n4. Colour: text meets contrast (4.5:1 normal, 3:1 large); meaning never by colour alone.\n5. Motion: respect reduced-motion for non-essential animation.\n\n## Tools\nread_file for the markup/styles; shell for any a11y linter or contrast checker the project has; read its output.\n\n## Proof\nList each check as pass/fail with the element and the fix applied. For contrast, give the measured ratio. Do not mark a check passed you did not actually test.\n";

const API_CONTRACT_TESTING: &str = "## Job\nVerify an API endpoint behaves as its contract says.\n\n## When to use\nA ticket adds or changes an endpoint, or an integration depends on one.\n\n## Steps\n1. State the contract: method, path, inputs, success status and body shape, and the error cases.\n2. Test the happy path: correct status and the exact body shape (fields, types).\n3. Test the edges: missing/invalid input, unauthorised, not-found, and large input; each returns the right status and a clear error, not a 500.\n4. Check idempotency and side effects where the contract promises them.\n\n## Tools\nshell to call the endpoint (curl or the project's test client) and to run the test suite; read the real responses; read_file for the handler and its schema.\n\n## Proof\nFor each case show the request and the actual status and body, next to what the contract requires. A claim of \"works\" with no shown response does not count.\n";

const DOC_WRITING: &str = "## Job\nWrite or update the user-facing docs for a change so a reader can use it without reading the code.\n\n## When to use\nA change adds or alters behaviour a user or another developer relies on (a command, flag, API, config, UI).\n\n## Steps\n1. Find who reads this and what they need to do; write for that task, not for the implementation.\n2. Update the right place (README, reference, changelog) rather than adding a new stray file.\n3. Lead with what it does and a working example; then the details and the edge cases.\n4. Keep it accurate: every command, flag and path must be real and current.\n\n## Tools\nread_file and grep to find the existing docs and confirm names/flags; shell to run any example you include and confirm it works.\n\n## Proof\nShow that the example you wrote actually runs (paste the command and its output), and name the file(s) you updated. Do not document a flag or path you have not verified exists.\n";

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
    fn a_built_in_skill_passes_its_own_checks() {
        for b in LIBRARY {
            let notes = lint_skill(b.name, strip_frontmatter(&b.skill_md()).as_str(), &[]);
            assert!(notes.is_empty(), "{}: {notes:?}", b.name);
            assert!(b.skill_md().starts_with("---\nname: "));
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
