//! A plugin's pieces read from its repository (ADR-157): the Claude Code plugin layout (`.claude-plugin/plugin.json`,
//! `skills/<name>/SKILL.md`, `.mcp.json` or the manifest's `mcpServers`), the one plugin a marketplace names, or a plain
//! repository with `skills/`. Only skills and MCP servers are taken; hooks, commands, agents, `bin/` and servers that
//! run code from the plugin's own folder are listed as skipped: Agenium runs no vendor code of a plugin.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use agenium_config::McpServerConfig;
use serde_json::Value;

use crate::{OrchError, Result};

fn invalid(m: impl Into<String>) -> OrchError {
    OrchError::Invalid(m.into())
}

/// What a plugin's folder holds, as Agenium takes it.
#[derive(Debug, Clone, Default)]
pub struct Pieces {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    /// Each skill's name and folder.
    pub skills: Vec<(String, PathBuf)>,
    /// Its servers, ids `<plugin>-<server>`.
    pub mcp: Vec<McpServerConfig>,
    /// What was not taken, and why.
    pub skipped: Vec<String>,
    /// The vendor's manifest (`plugin.json`), for the fields Agenium keeps.
    pub manifest: Value,
    /// The servers taken, under their own names, as the manifest's `mcpServers` writes them.
    pub mcp_raw: serde_json::Map<String, Value>,
    /// The hooks taken, in Claude Code's shape (`{event: [{matcher, hooks: [{type: "command", command, ...}]}]}`),
    /// only of the events Agenium runs (ADR-157).
    pub hooks: serde_json::Map<String, Value>,
    /// The folders the hooks' scripts are in: (path in the plugin, the folder in the repository).
    pub hook_dirs: Vec<(String, PathBuf)>,
}

/// The hook events Agenium runs, and when: `SessionStart` at the start of every run, `UserPromptSubmit` on each of the
/// user's messages to the Lead or Solo, `PreToolUse` before each tool call (it may only refuse or ask).
pub const HOOK_EVENTS: [&str; 3] = ["SessionStart", "UserPromptSubmit", "PreToolUse"];

/// Why the other events are not run.
fn unrun(event: &str) -> &'static str {
    match event {
        "PostToolUse" | "Stop" | "SubagentStop" | "PreCompact" => {
            "its text would stay in the stored conversation after the plugin is gone"
        }
        "SubagentStart" => "Agenium's agents are not subagents; each run starts with SessionStart",
        _ => "an event of Claude Code that Agenium has not",
    }
}

/// An installed plugin's manifest, Agenium's own file in Claude Code's `plugin.json` format (ADR-157).
pub const MANIFEST: &str = "agenium-plugin.json";

/// The fields of Claude Code's `plugin.json` that Agenium keeps (what it runs is left out: hooks, commands, agents).
const KEPT: [&str; 7] = [
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
];

impl Pieces {
    /// Agenium's manifest of what it took: the vendor's descriptive fields, `skills` and the servers taken, in
    /// Claude Code's `plugin.json` format.
    pub fn manifest_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("name".into(), Value::String(self.name.clone()));
        for k in KEPT {
            if !self.manifest[k].is_null() {
                m.insert(k.into(), self.manifest[k].clone());
            }
        }
        if !self.skills.is_empty() {
            m.insert("skills".into(), Value::String("./skills/".into()));
        }
        if !self.mcp_raw.is_empty() {
            m.insert("mcpServers".into(), Value::Object(self.mcp_raw.clone()));
        }
        if !self.hooks.is_empty() {
            m.insert("hooks".into(), Value::Object(self.hooks.clone()));
        }
        Value::Object(m)
    }
}

fn read_json(p: &Path) -> Result<Option<Value>> {
    if !p.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(p)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    serde_json::from_str(text)
        .map(Some)
        .map_err(|e| invalid(format!("{}: {e}", p.display())))
}

/// `rel` under `base`, never leaving it.
fn inside(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// The plugin's folder in the repository: the root with a plugin manifest, else the plugin a marketplace lists
/// (`plugin` names it when it lists several), else the root.
fn plugin_dir(root: &Path, plugin: Option<&str>) -> Result<PathBuf> {
    if root.join(MANIFEST).is_file() {
        return Ok(root.to_path_buf());
    }
    let manifest = read_json(&root.join(".claude-plugin/plugin.json"))?;
    let market = read_json(&root.join(".claude-plugin/marketplace.json"))?;
    let root_name = manifest.as_ref().and_then(|m| m["name"].as_str());
    if manifest.is_some() && (plugin.is_none() || plugin == root_name) {
        return Ok(root.to_path_buf());
    }
    let Some(market) = market else {
        return match plugin {
            Some(p) if manifest.is_some() => {
                Err(invalid(format!("the repository's plugin is not {p:?}")))
            }
            _ => Ok(root.to_path_buf()),
        };
    };
    let listed: Vec<(&str, &str)> = market["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["name"].as_str()?, p["source"].as_str()?)))
        .collect();
    let chosen = match plugin {
        Some(p) => listed.iter().find(|(n, _)| *n == p),
        None if listed.len() == 1 => listed.first(),
        None => {
            let names: Vec<&str> = listed.iter().map(|(n, _)| *n).collect();
            return Err(invalid(format!(
                "this marketplace lists {} plugins: name one ({})",
                names.len(),
                names.join(", ")
            )));
        }
    };
    let Some((name, src)) = chosen else {
        return Err(invalid(format!(
            "the marketplace lists no plugin {:?} in this repository",
            plugin.unwrap_or_default()
        )));
    };
    inside(root, src).filter(|d| d.is_dir()).ok_or_else(|| {
        invalid(format!(
            "plugin {name:?} is not a folder of this repository ({src})"
        ))
    })
}

/// A name as Agenium's skill names (`a-z`, `0-9`, single hyphens).
fn clean_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_ascii_lowercase().chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '-' };
        if !(c == '-' && (out.is_empty() || out.ends_with('-'))) {
            out.push(c);
        }
    }
    out.trim_end_matches('-')
        .chars()
        .take(agenium_skills::MAX_NAME)
        .collect()
}

fn paths(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}

/// Reads the plugin in `root` (a repository's folder); `fallback` names it when its manifest does not.
pub fn read(root: &Path, plugin: Option<&str>, fallback: &str) -> Result<Pieces> {
    let dir = plugin_dir(root, plugin)?;
    let manifest = match read_json(&dir.join(MANIFEST))? {
        Some(m) => m,
        None => read_json(&dir.join(".claude-plugin/plugin.json"))?.unwrap_or(Value::Null),
    };
    let name = clean_name(manifest["name"].as_str().or(plugin).unwrap_or(fallback));
    if !agenium_skills::valid_name(&name) {
        return Err(invalid(format!("cannot name the plugin from {fallback:?}")));
    }
    let mut out = Pieces {
        description: manifest["description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_owned(),
        version: manifest["version"].as_str().map(str::to_owned),
        name,
        manifest: manifest.clone(),
        ..Default::default()
    };

    // Skills: `skills/` and the folders the manifest adds.
    let mut skill_dirs = vec!["skills".to_owned()];
    skill_dirs.extend(paths(&manifest["skills"]));
    for rel in skill_dirs {
        let Some(base) = inside(&dir, &rel).filter(|d| d.is_dir()) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&base)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        entries.sort();
        // A folder that is one skill itself (a manifest naming the skill's own folder).
        if base.join(agenium_skills::SKILL_FILE).is_file() {
            entries = vec![base.clone()];
        }
        for d in entries {
            if !d.join(agenium_skills::SKILL_FILE).is_file() {
                continue;
            }
            let shown = d
                .strip_prefix(&dir)
                .unwrap_or(&d)
                .display()
                .to_string()
                .replace('\\', "/");
            match agenium_skills::validate(&d, agenium_skills::Origin::Plugin) {
                Ok(s) if out.skills.iter().any(|(n, _)| *n == s.meta.name) => out.skipped.push(
                    format!("skill {shown}: a second skill named {}", s.meta.name),
                ),
                Ok(s) => out.skills.push((s.meta.name, d)),
                Err(e) => out.skipped.push(format!("skill {shown}: {e}")),
            }
        }
    }

    // MCP servers: `.mcp.json`, or the manifest's `mcpServers` (an object, or a file it names).
    let servers = match &manifest["mcpServers"] {
        Value::Object(_) => Some(manifest["mcpServers"].clone()),
        Value::String(rel) => match inside(&dir, rel) {
            Some(p) => read_json(&p)?,
            None => None,
        },
        _ => read_json(&dir.join(".mcp.json"))?,
    };
    if let Some(v) = servers {
        let map = v.get("mcpServers").cloned().unwrap_or(v);
        for (server, mut cfg) in crate::mcp::servers_from_json(&map)? {
            let text = format!(
                "{:?} {:?} {:?} {:?}",
                cfg.command, cfg.args, cfg.env, cfg.url
            );
            if text.contains("CLAUDE_PLUGIN_ROOT") || text.contains("CLAUDE_PLUGIN_DATA") {
                out.skipped.push(format!(
                    "MCP server {server}: runs code from the plugin's own folder"
                ));
                continue;
            }
            cfg.id = format!("{}-{}", out.name, cfg.id);
            if !agenium_config::valid_mcp_id(&cfg.id) {
                out.skipped
                    .push(format!("MCP server {server}: no usable id ({})", cfg.id));
                continue;
            }
            out.mcp.push(cfg);
            out.mcp_raw.insert(server.clone(), map[&server].clone());
        }
    }

    // Hooks: run by Agenium in its sandbox (ADR-157), only of the events it has.
    let mut configs: Vec<(Option<String>, Value)> = Vec::new();
    match &manifest["hooks"] {
        Value::Object(o) => configs.push((None, Value::Object(o.clone()))),
        Value::Null => {
            if let Some(v) = read_json(&dir.join("hooks/hooks.json"))? {
                configs.push((Some("hooks/hooks.json".into()), v));
            }
        }
        v => {
            for rel in paths(v) {
                let rel = rel.trim_start_matches("./").to_owned();
                if let Some(v) = inside(&dir, &rel)
                    .map(|p| read_json(&p))
                    .transpose()?
                    .flatten()
                {
                    configs.push((Some(rel), v));
                }
            }
        }
    }
    for (file, v) in configs {
        if v.get("modules").is_some_and(|m| !m.is_null()) {
            out.skipped.push(format!(
                "{}: Claude Code interface modules",
                file.as_deref().unwrap_or("hooks")
            ));
        }
        let events = v.get("hooks").cloned().unwrap_or(v);
        let Some(events) = events.as_object() else {
            continue;
        };
        for (event, groups) in events {
            if event == "modules" {
                continue;
            }
            if !HOOK_EVENTS.contains(&event.as_str()) {
                out.skipped
                    .push(format!("hook {event}: not run, {}", unrun(event)));
                continue;
            }
            for g in groups.as_array().into_iter().flatten() {
                let mut g = g.clone();
                let kept: Vec<Value> = g["hooks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|h| {
                        let ok = h["type"].as_str() == Some("command") && h["command"].is_string();
                        if !ok {
                            out.skipped.push(format!(
                                "hook {event}: a {} hook (only commands are run)",
                                h["type"].as_str().unwrap_or("?")
                            ));
                        }
                        ok
                    })
                    .cloned()
                    .collect();
                if kept.is_empty() {
                    continue;
                }
                g["hooks"] = Value::Array(kept);
                out.hooks
                    .entry(event.clone())
                    .or_insert_with(|| Value::Array(vec![]))
                    .as_array_mut()
                    .expect("an array")
                    .push(g);
            }
        }
        // The scripts sit beside their config (`hooks/`), as Claude Code's plugins keep them.
        let folder = file
            .as_deref()
            .and_then(|f| Path::new(f).parent())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "hooks".into());
        if let Some(src) = inside(&dir, &folder).filter(|d| d.is_dir()) {
            if !out.hook_dirs.iter().any(|(f, _)| *f == folder) {
                out.hook_dirs.push((folder, src));
            }
        }
    }

    // What Agenium does not take.
    let has = |rel: &str| dir.join(rel).exists();
    let mut not_taken: BTreeMap<&str, &str> = BTreeMap::new();
    if has("commands") || !manifest["commands"].is_null() {
        not_taken.insert("commands", "slash commands of a vendor's CLI");
    }
    if has("agents") || !manifest["agents"].is_null() {
        not_taken.insert(
            "agents",
            "a vendor CLI's subagents; Agenium's agents are its own",
        );
    }
    if has("bin") {
        not_taken.insert("bin", "programs: not run");
    }
    for (k, why) in not_taken {
        out.skipped.push(format!("{k}: {why}"));
    }
    if out.skills.is_empty() && out.mcp.is_empty() {
        return Err(invalid(format!(
            "plugin {} has no skill or MCP server Agenium can use ({})",
            out.name,
            if out.skipped.is_empty() {
                "nothing found".into()
            } else {
                out.skipped.join("; ")
            }
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    fn skill(dir: &Path, name: &str) {
        write(
            &dir.join(name).join("SKILL.md"),
            &format!("---\nname: {name}\ndescription: Use {name}.\n---\nDo {name}.\n"),
        );
    }

    #[test]
    fn skills_and_servers_are_taken_and_the_rest_named() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(
            &r.join(".claude-plugin/plugin.json"),
            r#"{"name":"Super Powers","version":"7.0.2","description":"Core skills.","hooks":["./hooks/h.json"]}"#,
        );
        skill(&r.join("skills"), "brainstorming");
        skill(&r.join("skills"), "writing-plans");
        write(&r.join("skills/bad/SKILL.md"), "no frontmatter");
        write(&r.join("hooks/h.json"), "{}");
        write(&r.join("commands/go.md"), "go");
        write(
            &r.join(".mcp.json"),
            r#"{"mcpServers":{"docs":{"command":"npx","args":["-y","docs-mcp"]},
                "local":{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/server.js"]}}}"#,
        );
        let p = read(r, None, "repo").unwrap();
        assert_eq!(p.name, "super-powers");
        assert_eq!(p.version.as_deref(), Some("7.0.2"));
        let names: Vec<&str> = p.skills.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["brainstorming", "writing-plans"]);
        assert_eq!(p.mcp.len(), 1);
        assert_eq!(p.mcp[0].id, "super-powers-docs");
        let skipped = p.skipped.join("\n");
        for k in ["skill skills/bad", "MCP server local", "commands:"] {
            assert!(skipped.contains(k), "{k} in {skipped}");
        }
    }

    #[test]
    fn a_marketplace_gives_the_plugin_it_names() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        write(
            &r.join(".claude-plugin/marketplace.json"),
            r#"{"plugins":[{"name":"one","source":"./plugins/one"},{"name":"two","source":"./plugins/two"},
                {"name":"out","source":"../elsewhere"}]}"#,
        );
        skill(&r.join("plugins/one/skills"), "a");
        skill(&r.join("plugins/two/skills"), "b");
        assert!(read(r, None, "repo")
            .unwrap_err()
            .to_string()
            .contains("one, two"));
        let two = read(r, Some("two"), "repo").unwrap();
        assert_eq!((two.name.as_str(), two.skills[0].0.as_str()), ("two", "b"));
        assert!(read(r, Some("out"), "repo").is_err());
        // A plain repository with skills/ is a plugin named after it.
        let plain = tempfile::tempdir().unwrap();
        skill(&plain.path().join("skills"), "c");
        assert_eq!(read(plain.path(), None, "My_Repo").unwrap().name, "my-repo");
        assert!(read(tempfile::tempdir().unwrap().path(), None, "x").is_err());
    }
}
