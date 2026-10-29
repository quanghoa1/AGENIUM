//! Library skills a task's agent has (`use-skill`): written into the library through the same path as the app's skill
//! editor (SKILL.md with its references and scripts in one save) and switched on for the agent. The task can only be
//! done through them: the note's format is in a reference, and the release code is what the skill's script prints.

use agenium_core::AgentId;
use agenium_orchestrator::app::App;

/// The release code the skill's script prints; nothing else in the project holds it.
pub const CODE: &str = "RC-7731";

const SKILL: &str = "---
name: release-stamp
description: Write a release note in the team's format, stamped with the release code. Use when a ticket asks for deliverables/release.txt.
tags: docs
---

## Job
Write the release note in the team's format with the current release code.

## When to use
A ticket asks for the release note (deliverables/release.txt).

## Steps
1. Read `references/format.md` (in this skill's folder) for the note's format.
2. Get the release code: run `scripts/stamp.sh` with sh, or `scripts/stamp.ps1` with PowerShell, through your shell tool (give the script's full path in this skill's folder). It prints the code; never make one up.
3. Write deliverables/release.txt in that format with that code.

## Tools
load_skill, read_file, shell, write_file.

## Proof
deliverables/release.txt follows the format and holds the code the script printed.
";

const FORMAT: &str = "# Release note format\n\nExactly two lines:\n\n1. `RELEASE <code>` where <code> is the release code the stamp script prints.\n2. `Notes: <one sentence on what changed>`\n";

/// Installs the library skills of task `task` and switches them on for `agent`; nothing for other tasks.
pub fn install(app: &App, agent: AgentId, task: &str) -> anyhow::Result<()> {
    if task != "use-skill" {
        return Ok(());
    }
    let files = vec![
        (
            "references/format.md".to_owned(),
            FORMAT.as_bytes().to_vec(),
        ),
        (
            "scripts/stamp.sh".to_owned(),
            format!("echo {CODE}\n").into_bytes(),
        ),
        (
            "scripts/stamp.ps1".to_owned(),
            format!("Write-Output '{CODE}'\n").into_bytes(),
        ),
    ];
    app.save_library_skill("release-stamp", SKILL, true, &files, &[])?;
    app.set_agent_skills(agent, &["release-stamp".to_owned()])?;
    Ok(())
}
