//! Role library and the shape of ROLE.md (ADR-64): every role text has the same short sections, so the Lead and the
//! agent read the same kind of contract whatever the role. The check is advice shown with the role (missing or empty
//! sections, too long, nearly the same as another role); it never refuses a save.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::team_templates::RoleSpec;

/// The sections of a ROLE.md, in order.
pub const ROLE_SECTIONS: [&str; 5] = ["Mission", "Rules", "Deliverables", "Workflow", "Done when"];

/// Origin of a library role's folder (`role.toml` `template`).
pub const LIBRARY_ORIGIN: &str = "library@1";

/// Above this a role text crowds the context of every run: procedures belong in skills.
pub const MAX_ROLE_CHARS: usize = 6000;

/// Two roles whose word sets overlap this much are one role twice.
const NEAR_DUPLICATE: f64 = 0.8;

const LIBRARY: &str = include_str!("../templates/roles.toml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    roles: Vec<RoleSpec>,
}

/// The built-in specialist roles (any team can use them; their folder id is the role id).
pub fn library() -> Vec<RoleSpec> {
    toml::from_str::<Library>(LIBRARY)
        .expect("the built-in role library is valid")
        .roles
}

/// An empty ROLE.md with the sections to fill in.
pub fn role_skeleton() -> String {
    ROLE_SECTIONS
        .iter()
        .map(|s| format!("## {s}\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `## Heading` -> its body, in file order.
fn sections(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            out.push((h.trim().to_lowercase(), String::new()));
        } else if let Some((_, body)) = out.last_mut() {
            body.push_str(line);
            body.push('\n');
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

/// Advice about a role text; `others` are the other roles (id, text).
pub fn lint_role(id: &str, text: &str, others: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    let found = sections(text);
    for s in ROLE_SECTIONS {
        match found.iter().find(|(h, _)| *h == s.to_lowercase()) {
            None => out.push(format!("missing section \"## {s}\"")),
            Some((_, body)) if body.trim().is_empty() => {
                out.push(format!("section \"## {s}\" is empty"))
            }
            Some(_) => {}
        }
    }
    let n = text.chars().count();
    if n > MAX_ROLE_CHARS {
        out.push(format!(
            "{n} characters (more than {MAX_ROLE_CHARS}): move procedures into skills"
        ));
    }
    let mine = words(text);
    for (other, theirs) in others {
        if *other == id || mine.is_empty() {
            continue;
        }
        let theirs = words(theirs);
        let union = mine.union(&theirs).count();
        let same = mine.intersection(&theirs).count() as f64 / union.max(1) as f64;
        if same >= NEAR_DUPLICATE {
            out.push(format!(
                "nearly the same as role {other} ({:.0}% of the words): use that role or make the difference clear",
                same * 100.0
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_passes_only_once_filled() {
        let empty = role_skeleton();
        assert_eq!(lint_role("x", &empty, &[]).len(), ROLE_SECTIONS.len());
        let filled: String = ROLE_SECTIONS
            .iter()
            .map(|s| format!("## {s}\nSomething about {s}.\n"))
            .collect();
        assert!(lint_role("x", &filled, &[]).is_empty());
        let prose = "You are a backend developer.";
        assert!(lint_role("x", prose, &[])[0].contains("missing section \"## Mission\""));
        let long = format!("{filled}{}", "word ".repeat(MAX_ROLE_CHARS / 4));
        assert!(lint_role("x", &long, &[])
            .iter()
            .any(|w| w.contains("skills")));
        let dup = lint_role("x", &filled, &[("y", &filled), ("x", &filled)]);
        assert_eq!(dup.len(), 1, "{dup:?}");
        assert!(dup[0].contains("role y"));
    }

    #[test]
    fn library_and_template_roles_follow_the_sections_and_differ() {
        let mut all: Vec<(String, String)> =
            library().into_iter().map(|r| (r.id, r.prompt)).collect();
        for t in crate::team_templates::builtin() {
            for r in &t.roles {
                all.push((t.role_dir(&r.id), r.prompt.clone()));
            }
        }
        let ids: BTreeSet<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids.len(), all.len(), "a library role hides a template role");
        let others: Vec<(&str, &str)> = all.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        for (id, text) in &all {
            assert_eq!(lint_role(id, text, &others), Vec::<String>::new(), "{id}");
        }
    }
}
