//! The skills a drafting helper is shown (ADR-151; 2026-11-17): the library's and the installed plugins' skills, each
//! as the entry that gives it to an agent (a library skill's name, a plugin's `plugin:<plugin>/<skill>`), its tags and
//! the start of its description (what it is for, which is what a helper picks by). A large library keeps the prompt
//! small: past `FULL_UP_TO` skills, only those closest to the request keep their description, the rest are listed by
//! entry and tags. The helper's draft is still checked against every skill, listed in full or not.

use crate::app::App;
use crate::Result;

/// Up to this many skills, every one is listed with its description.
const FULL_UP_TO: usize = 500;
/// Past it, this many of the closest to the request keep their description.
const RELEVANT: usize = 40;
/// The description is cut to this many characters (its first sentence is the "when to use").
const DESC_CHARS: usize = 80;

/// One skill a helper may give a role.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogueSkill {
    /// How an agent is given it: the library skill's name, or `plugin:<plugin>/<skill>`.
    pub entry: String,
    pub name: String,
    /// The plugin it comes from (its vendor's: never extended or merged into).
    pub plugin: Option<String>,
    pub tags: Vec<String>,
    pub description: String,
}

impl App {
    /// The library's skills, then the installed plugins' skills.
    pub fn catalogue_skills(&self) -> Result<Vec<CatalogueSkill>> {
        let reg = self.skill_registry();
        let mut out: Vec<CatalogueSkill> = reg
            .library()
            .skills
            .into_iter()
            .map(|s| CatalogueSkill {
                entry: s.meta.name.clone(),
                name: s.meta.name,
                plugin: None,
                tags: s.meta.tags,
                description: s.meta.description,
            })
            .collect();
        for p in self.plugins()? {
            let dir = reg.plugins_dir().join(&p.name).join("skills");
            for name in p.skills.keys() {
                let Ok(s) =
                    agenium_skills::validate(&dir.join(name), agenium_skills::Origin::Plugin)
                else {
                    continue;
                };
                out.push(CatalogueSkill {
                    entry: agenium_skills::plugin_skill_entry(&p.name, name),
                    name: name.clone(),
                    plugin: Some(p.name.clone()),
                    tags: s.meta.tags,
                    description: s.meta.description,
                });
            }
        }
        Ok(out)
    }
}

/// The start of `text` on one line, cut at `n` characters.
fn short(text: &str, n: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", one[..i].trim_end()),
        None => one,
    }
}

/// How close a skill is to `words` (the request's): its words that the request has, its name and tags counted twice.
fn closeness(s: &CatalogueSkill, words: &[String]) -> usize {
    let hits = |text: &str| {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3 && words.iter().any(|x| x == w))
            .count()
    };
    2 * (hits(&s.name) + hits(&s.tags.join(" "))) + hits(&s.description)
}

/// The skills as the helper reads them, one per line: `- <entry> [tags]: <description start>`. Past `FULL_UP_TO`, only
/// the `RELEVANT` closest to `request` keep their description, and the text says so.
pub fn catalogue_text(skills: &[CatalogueSkill], request: &str) -> String {
    if skills.is_empty() {
        return "(none yet)\n".into();
    }
    let words: Vec<String> = request
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_owned)
        .collect();
    let described: Vec<usize> = if skills.len() <= FULL_UP_TO {
        (0..skills.len()).collect()
    } else {
        let mut ranked: Vec<(usize, usize)> = skills
            .iter()
            .enumerate()
            .map(|(i, s)| (closeness(s, &words), i))
            .filter(|(c, _)| *c > 0)
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        ranked.into_iter().take(RELEVANT).map(|(_, i)| i).collect()
    };
    let mut text = String::new();
    if skills.len() > FULL_UP_TO {
        text.push_str(&format!(
            "({} skills: the {} closest to the request with what they are for, the others by name and tags; ask the \
             user when a part of the work may have a skill you cannot tell)\n",
            skills.len(),
            described.len()
        ));
    }
    for (i, s) in skills.iter().enumerate() {
        let tags = if s.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", s.tags.join(", "))
        };
        if described.contains(&i) {
            text.push_str(&format!(
                "- {}{tags}: {}\n",
                s.entry,
                short(&s.description, DESC_CHARS)
            ));
        } else {
            text.push_str(&format!("- {}{tags}\n", s.entry));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, plugin: Option<&str>, desc: &str) -> CatalogueSkill {
        CatalogueSkill {
            entry: plugin.map_or(name.to_owned(), |p| format!("plugin:{p}/{name}")),
            name: name.into(),
            plugin: plugin.map(str::to_owned),
            tags: vec!["web".into()],
            description: desc.into(),
        }
    }

    #[test]
    fn each_skill_is_its_entry_tags_and_the_start_of_its_description() {
        let long = "Use when testing a web application end to end, from the login flow to every API it calls and back.";
        let t = catalogue_text(
            &[
                skill("burp-flow", None, long),
                skill("web-pentest", Some("pentest"), "Test web targets."),
            ],
            "",
        );
        assert!(t.contains("- burp-flow [web]: Use when testing a web application end to end, from the login flow to every API…"), "{t}");
        assert!(
            t.contains("- plugin:pentest/web-pentest [web]: Test web targets.\n"),
            "{t}"
        );
    }

    #[test]
    fn a_large_library_describes_only_the_skills_closest_to_the_request() {
        let mut all: Vec<CatalogueSkill> = (0..FULL_UP_TO)
            .map(|i| skill(&format!("filler-{i}"), None, "Format spreadsheets."))
            .collect();
        all.push(skill("apk-reverse", None, "Decompile an Android APK."));
        let t = catalogue_text(&all, "a team that reverses an Android apk");
        assert!(
            t.starts_with(&format!("({} skills:", FULL_UP_TO + 1)),
            "{}",
            &t[..200]
        );
        assert!(
            t.contains("- apk-reverse [web]: Decompile an Android APK."),
            "the close one keeps its description"
        );
        assert!(
            t.contains("- filler-3 [web]\n"),
            "the others by name and tags"
        );
        assert!(
            t.lines().count() == FULL_UP_TO + 2,
            "every skill is still listed"
        );
    }
}
