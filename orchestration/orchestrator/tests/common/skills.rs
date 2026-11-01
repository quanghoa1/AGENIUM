//! Library skills written for the tests. Agenium ships no skills (ADR-88 amended): a test that needs some in the
//! library adds these first. Each carries the lines its tests compare (a subject, a directive).

use agenium_orchestrator::App;

/// Name, description, body.
const SKILLS: &[(&str, &str, &str)] = &[
    (
        "code-review",
        "Reviews a change for correctness, safety and clarity before it is approved. Use when a change is ready for review.",
        "## Job\nReview a change for correctness, safety and clarity, with evidence.\n\n## When to use\nA change is ready for review.\n\n## Steps\n1. Read the diff and the code it touches.\n2. Run the tests; do not assume they passed.\n3. Go through references/checklist.md.\n\n## Tools\n`read_file`, `grep`, `shell`.\n\n## Proof\nEach issue with its file and line.\n",
    ),
    (
        "writing-tests",
        "Designs tests that catch real regressions in changed behaviour. Use when a ticket adds behaviour that needs tests.",
        "## Job\nAdd tests that would catch a real regression in the behaviour under change.\n\n## When to use\nNew or changed behaviour.\n\n## Steps\n1. Test what the code does, not how it does it.\n\n## Tools\n`shell` to run the tests.\n\n## Proof\nThe new tests failing before the change and passing after.\n",
    ),
    (
        "root-cause-debugging",
        "Fixes the cause of a bug, not its symptom. Use when a test fails or behaviour is wrong.",
        "## Job\nFix the cause of a bug, not its symptom.\n\n## When to use\nA defect or a failing test.\n\n## Steps\n1. Reproduce first.\n2. Fix the cause. Avoid sleeps, broad try/except, or disabling the test: those hide the bug.\n3. Add a regression test.\n\n## Tools\n`shell` to run the reproducer.\n\n## Proof\nThe reproducer before and after.\n",
    ),
    (
        "commit-and-pr",
        "Writes a clear commit and pull request for one logical change. Use when committing work or opening a pull request.",
        "## Job\nTurn a finished change into a clear commit a reviewer can follow.\n\n## When to use\nBefore committing work.\n\n## Steps\n1. Keep one logical change per commit.\n2. Subject: imperative mood, <= 72 characters, says what the change does (not \"fixed stuff\").\n3. Body: why the change is needed.\n\n## Tools\n`shell` for `git diff --staged`.\n\n## Proof\nShow the final message.\n",
    ),
    (
        "a11y-audit",
        "Checks a page for accessibility problems. Use when a ticket changes what users see.",
        "## Job\nFind what keeps people from using a page.\n\n## When to use\nA change to a page.\n\n## Steps\n1. Check contrast, labels and keyboard use.\n\n## Tools\nThe browser.\n\n## Proof\nEach check as pass or fail. Do not mark a check passed you did not actually test.\n",
    ),
    (
        "api-contract-testing",
        "Tests that an API answers as its contract says. Use when a ticket adds or changes an endpoint.",
        "## Job\nProve each endpoint answers as documented.\n\n## When to use\nA new or changed endpoint.\n\n## Steps\n1. Call each endpoint with good and bad input.\n\n## Tools\n`shell` for the requests.\n\n## Proof\nEach request with its answer.\n",
    ),
];

/// Adds the named test skills to the root's library.
pub fn library_skills(app: &App, names: &[&str]) {
    for n in names {
        let (name, description, body) = SKILLS
            .iter()
            .find(|(s, _, _)| s == n)
            .unwrap_or_else(|| panic!("no test skill {n}"));
        let text =
            format!("---\nname: {name}\ndescription: {description}\ntags: coding\n---\n\n{body}");
        // A skill with a file of its own, which a merge must keep.
        let files: Vec<(String, Vec<u8>)> = if *name == "code-review" {
            vec![(
                "references/checklist.md".into(),
                b"- Correctness\n- Safety\n- Clarity\n".to_vec(),
            )]
        } else {
            vec![]
        };
        app.save_library_skill(name, &text, true, &files, &[])
            .unwrap();
    }
}

/// Every test skill.
pub fn all_library_skills(app: &App) {
    let names: Vec<&str> = SKILLS.iter().map(|(n, _, _)| *n).collect();
    library_skills(app, &names);
}
