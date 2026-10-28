//! Text work on skill overlays: patches applied and undone, three-way merges, diffs, frontmatter kept.

use super::*;

pub(super) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

/// Applies a unified diff to `text` with `git apply` in a scratch folder (`a/`/`b/` prefixes or none).
pub(super) fn apply_patch(text: &str, patch: &str) -> Result<String> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("SKILL.md"), text)?;
    let patch_file = dir.path().join("change.patch");
    let mut p = patch.replace("\r\n", "\n");
    if !p.ends_with('\n') {
        p.push('\n');
    }
    // A diff with `a/` `b/` paths is meant for -p1: report that attempt's error, not the -p0 fallback's (which only
    // says "b/SKILL.md: No such file" and hid the real cause from agents in the learning-loop runs).
    let prefixed = p
        .lines()
        .any(|l| l.starts_with("--- a/") || l.starts_with("+++ b/"));
    std::fs::write(&patch_file, &p)?;
    let mut errors = Vec::new();
    for strip in ["-p1", "-p0"] {
        let out = git()
            .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"])
            .args(["apply", "--recount", "--whitespace=nowarn", strip])
            .arg(&patch_file)
            .current_dir(dir.path())
            .output()?;
        if out.status.success() {
            let t = std::fs::read_to_string(dir.path().join("SKILL.md"))?;
            return Ok(t.replace("\r\n", "\n"));
        }
        errors.push(String::from_utf8_lossy(&out.stderr).trim().to_owned());
    }
    // Fallback seen in the real runs: a hunk with no context lines numbered from the body the agent read (so git cannot
    // place it). When every hunk's old text occurs exactly once, apply it by content; anything ambiguous is refused.
    if let Some(t) = apply_by_content(&text.replace("\r\n", "\n"), &p) {
        return Ok(t);
    }
    let reason = if prefixed { &errors[0] } else { &errors[1] };
    Err(invalid(format!(
        "the patch does not apply to the current SKILL.md ({reason}). Copy the context lines exactly as load_skill \
         shows them, or send the complete new body as `content` instead of a patch"
    )))
}

/// Applies a unified diff by text instead of line numbers: each hunk's old lines (context and removed) must occur
/// exactly once in `text` and are replaced by its new lines (context and added). `None` when a hunk is empty, missing
/// or ambiguous — then nothing is applied.
pub(super) fn apply_by_content(text: &str, patch: &str) -> Option<String> {
    let mut hunks: Vec<(Vec<&str>, Vec<&str>)> = Vec::new();
    for line in patch.lines() {
        if line.starts_with("@@") {
            hunks.push((Vec::new(), Vec::new()));
            continue;
        }
        let Some((old, new)) = hunks.last_mut() else {
            continue; // file headers before the first hunk
        };
        match line.chars().next() {
            Some(' ') => {
                old.push(&line[1..]);
                new.push(&line[1..]);
            }
            Some('-') => old.push(&line[1..]),
            Some('+') => new.push(&line[1..]),
            Some('\\') => {} // "\ No newline at end of file"
            None => {
                old.push("");
                new.push("");
            }
            _ => return None,
        }
    }
    // Match whole lines only ("Step 1" must not match inside "Step 10"): wrap in newlines while matching.
    let mut out = format!("\n{text}\n");
    for (old, new) in hunks {
        if old.iter().all(|l| l.trim().is_empty()) {
            return None;
        }
        let (old, new) = (
            format!("\n{}\n", old.join("\n")),
            format!("\n{}\n", new.join("\n")),
        );
        if out.matches(&old).count() != 1 {
            return None;
        }
        out = out.replacen(&old, &new, 1);
    }
    Some(out[1..out.len() - 1].to_owned())
}

/// The SKILL.md front block (`---` … `---` and its line end), or "" when there is none.
pub(super) fn frontmatter(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let close = 3 + end + 4;
            let eol = text[close..]
                .find('\n')
                .map(|i| i + 1)
                .unwrap_or(text.len() - close);
            return &text[..close + eol];
        }
    }
    ""
}

/// The skill text an update produces: the current frontmatter with the new body (`content`; a frontmatter the agent
/// copied in is dropped, so name and description cannot change this way), or the current text with the diff applied
/// (`patch`). Exactly one of the two.
pub(super) fn updated_text(current: &str, u: &ProposeSkillUpdate) -> Result<String> {
    let content = u
        .content
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match (content, u.patch.trim().is_empty()) {
        (Some(_), false) => Err(invalid(
            "give either the new body as content or a patch, not both",
        )),
        (None, true) => Err(invalid("give the new body as content (or a patch)")),
        (Some(body), true) => {
            let body = crate::skill_shape::strip_frontmatter(body);
            Ok(format!(
                "{}\n{}\n",
                frontmatter(&current.replace("\r\n", "\n")).trim_end(),
                body.trim()
            ))
        }
        (None, false) => apply_patch(current, &u.patch),
    }
}

/// Reverse-applies `patch` (a base -> overlay diff) to `overlay`, reconstructing the base it was built on.
pub(super) fn unapply_patch(overlay: &str, patch: &str) -> Result<String> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("SKILL.md"), overlay)?;
    let patch_file = dir.path().join("change.patch");
    let mut p = patch.replace("\r\n", "\n");
    if !p.ends_with('\n') {
        p.push('\n');
    }
    std::fs::write(&patch_file, p)?;
    let mut last = String::new();
    // `diff_texts` writes under a/ and b/ subdirs, so git's own a/ b/ prefix makes the paths a/a/SKILL.md, b/b/SKILL.md
    // (strip 2); -p1/-p0 cover a patch stored in another shape.
    for strip in ["-p2", "-p1", "-p0"] {
        let out = git()
            .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf"])
            .args([
                "apply",
                "--reverse",
                "--recount",
                "--whitespace=nowarn",
                strip,
            ])
            .arg(&patch_file)
            .current_dir(dir.path())
            .output()?;
        if out.status.success() {
            let t = std::fs::read_to_string(dir.path().join("SKILL.md"))?;
            return Ok(t.replace("\r\n", "\n"));
        }
        last = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    }
    Err(invalid(format!(
        "cannot reconstruct the base the overlay was built on: {last}"
    )))
}

/// 3-way merge (ADR-86): start from `new_base` and fold in the agent's `old_base` -> `overlay` changes. Returns the
/// merged text and whether it carries conflict markers. `git merge-file` needs no repository.
pub(super) fn merge_three(new_base: &str, old_base: &str, overlay: &str) -> Result<(String, bool)> {
    let dir = tempfile::tempdir()?;
    let cur = dir.path().join("current");
    let anc = dir.path().join("ancestor");
    let oth = dir.path().join("other");
    std::fs::write(&cur, new_base.replace("\r\n", "\n"))?;
    std::fs::write(&anc, old_base.replace("\r\n", "\n"))?;
    std::fs::write(&oth, overlay.replace("\r\n", "\n"))?;
    let out = git()
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "merge-file",
            "-p",
            "--diff3",
            "-L",
            "new base",
            "-L",
            "previous base",
            "-L",
            "your overlay",
        ])
        .arg(&cur)
        .arg(&anc)
        .arg(&oth)
        .output()?;
    // Exit status: 0 = clean, > 0 = number of conflicts, < 0 = error. Treat a non-zero code as a conflict; a real git
    // error leaves the merged text with markers too, which the caller surfaces for the user.
    let merged = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    let conflict = out.status.code().unwrap_or(1) != 0;
    Ok((merged, conflict))
}

/// Unified diff `a/SKILL.md` -> `b/SKILL.md` (git, no repository needed).
pub(super) fn diff_texts(base: &str, overlay: &str) -> Result<String> {
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    for (d, t) in [(&a, base), (&b, overlay)] {
        std::fs::create_dir_all(d)?;
        std::fs::write(d.join("SKILL.md"), t)?;
    }
    let out = git()
        .args([
            "diff",
            "--no-index",
            "--no-color",
            "a/SKILL.md",
            "b/SKILL.md",
        ])
        .current_dir(dir.path())
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `git` without a console window (the desktop app has none: each call would open a terminal window).
pub(super) fn git() -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}
