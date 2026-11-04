//! Plugins (ADR-157): a vendor's set of skills and MCP servers, imported from its repository into
//! `<root>/plugins/<name>/` and kept as the vendor made it. The set is one thing: an agent enables or disables the
//! whole plugin (`plugin:<name>` in its skill binding), never one of its skills or servers. Its skills are read-only:
//! an agent's learning, its skill proposals and overlays never touch them. An update replaces the whole set with the
//! vendor's newer commit, after the user saw what changes. Agenium runs no vendor code of a plugin: hooks, commands,
//! agents and programs are left out, and said so.
//!
//! The formats are Claude Code's, not new ones: an installed plugin is `agenium-plugin.json`, written by Agenium in
//! Claude Code's `plugin.json` format with what it took (the vendor's descriptive fields, `skills`, the `mcpServers`
//! taken), and the skills taken under `skills/`. Where each came from is kept in `<root>/plugins/installed_plugins.json`,
//! shaped like Claude Code's (`name@source` -> install path, version, `gitCommitSha`, dates).

pub mod hooks;
mod layout;
pub mod source;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agenium_config::McpServerConfig;
use agenium_core::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::{OrchError, Result};
pub use source::Source;

/// Where each installed plugin came from, as Claude Code keeps it (`~/.claude/plugins/installed_plugins.json`).
pub const INSTALLED: &str = "installed_plugins.json";

/// One install, in Claude Code's fields; `source`, `ref` and `entry` say where an update reads from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Install {
    scope: String,
    install_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    installed_at: String,
    last_updated: String,
    git_commit_sha: String,
    source: String,
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry: Option<String>,
    /// What the install did not take from the vendor's repository, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    skipped: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Installed {
    #[serde(default = "two")]
    version: u32,
    #[serde(default)]
    plugins: BTreeMap<String, Vec<Install>>,
}

fn two() -> u32 {
    2
}

impl Installed {
    fn load(dir: &Path) -> Installed {
        std::fs::read_to_string(dir.join(INSTALLED))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&mut self, dir: &Path) -> Result<()> {
        self.version = 2;
        let text = serde_json::to_string_pretty(self).map_err(|e| invalid(e.to_string()))?;
        let tmp = dir.join(format!(".{INSTALLED}.tmp"));
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, dir.join(INSTALLED))?;
        Ok(())
    }

    /// The install of the plugin `name` (keys are `name@source`).
    fn get(&self, name: &str) -> Option<&Install> {
        self.plugins
            .iter()
            .find(|(k, _)| k.split('@').next() == Some(name))
            .and_then(|(_, v)| v.first())
    }

    fn remove(&mut self, name: &str) {
        self.plugins
            .retain(|k, _| k.split('@').next() != Some(name));
    }
}

fn invalid(m: impl Into<String>) -> OrchError {
    OrchError::Invalid(m.into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    /// `owner/repo` or a folder.
    pub source: String,
    /// The ref an update follows (`HEAD`, a branch, a tag).
    pub reference: String,
    /// The marketplace entry it came from, when its repository lists several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// The commit installed (`local` for a folder).
    pub commit: String,
    /// Each skill and a digest of its files.
    pub skills: BTreeMap<String, String>,
    pub mcp: Vec<McpServerConfig>,
    /// Its hooks Agenium runs in the sandbox: `event: command` (ADR-157).
    #[serde(default)]
    pub hooks: Vec<String>,
    /// What was not taken, and why.
    pub skipped: Vec<String>,
    pub installed_at: String,
}

/// What an install or an update would bring, shown before anything is written.
#[derive(Debug, Clone, Serialize)]
pub struct PluginPreview {
    pub plugin: Plugin,
    /// The installed version it would replace.
    pub installed: Option<Plugin>,
    pub skills_added: Vec<String>,
    pub skills_changed: Vec<String>,
    pub skills_removed: Vec<String>,
    pub mcp_added: Vec<String>,
    pub mcp_changed: Vec<String>,
    pub mcp_removed: Vec<String>,
    /// The agents that have it on.
    pub agents: Vec<String>,
}

impl PluginPreview {
    fn new(plugin: Plugin, installed: Option<Plugin>, agents: Vec<String>) -> Self {
        let none = Plugin {
            skills: BTreeMap::new(),
            mcp: vec![],
            ..plugin.clone()
        };
        let old = installed.as_ref().unwrap_or(&none);
        let skills_added = plugin
            .skills
            .keys()
            .filter(|k| !old.skills.contains_key(*k))
            .cloned()
            .collect();
        let skills_changed = plugin
            .skills
            .iter()
            .filter(|(k, v)| old.skills.get(*k).is_some_and(|o| o != *v))
            .map(|(k, _)| k.clone())
            .collect();
        let skills_removed = old
            .skills
            .keys()
            .filter(|k| !plugin.skills.contains_key(*k))
            .cloned()
            .collect();
        let find = |list: &[McpServerConfig], id: &str| list.iter().find(|m| m.id == id).cloned();
        let mcp_added = plugin
            .mcp
            .iter()
            .filter(|m| find(&old.mcp, &m.id).is_none())
            .map(|m| m.id.clone())
            .collect();
        let mcp_changed = plugin
            .mcp
            .iter()
            .filter(|m| find(&old.mcp, &m.id).is_some_and(|o| o != **m))
            .map(|m| m.id.clone())
            .collect();
        let mcp_removed = old
            .mcp
            .iter()
            .filter(|m| find(&plugin.mcp, &m.id).is_none())
            .map(|m| m.id.clone())
            .collect();
        Self {
            plugin,
            installed,
            skills_added,
            skills_changed,
            skills_removed,
            mcp_added,
            mcp_changed,
            mcp_removed,
            agents,
        }
    }
}

/// A digest of a folder's files (names and bytes), to tell a changed skill.
fn digest(dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for f in files {
        h.update(f.as_bytes());
        h.update([0]);
        h.update(std::fs::read(dir.join(&f))?);
        h.update([0]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<String>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            collect(&p, base, out)?;
        } else {
            out.push(
                p.strip_prefix(base)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

impl App {
    fn plugins_dir(&self) -> PathBuf {
        self.skill_registry().plugins_dir()
    }

    /// The installed plugins, by name.
    pub fn plugins(&self) -> Result<Vec<Plugin>> {
        let installed = Installed::load(&self.plugins_dir());
        let mut names: Vec<&str> = installed
            .plugins
            .keys()
            .filter_map(|k| k.split('@').next())
            .collect();
        names.sort();
        names.dedup();
        Ok(names.into_iter().filter_map(|n| self.plugin(n)).collect())
    }

    pub fn plugin(&self, name: &str) -> Option<Plugin> {
        if !agenium_skills::valid_name(name) {
            return None;
        }
        let dir = self.plugins_dir();
        let install = Installed::load(&dir).get(name)?.clone();
        let pieces = layout::read(&dir.join(name), None, name).ok()?;
        let source = Source::parse(&install.source).ok()?;
        let mut p = plugin_record(
            &source,
            install.entry.as_deref(),
            &install.reference,
            &pieces,
            &install.git_commit_sha,
        )
        .ok()?;
        p.installed_at = install.installed_at;
        p.skipped = install.skipped;
        Some(p)
    }

    /// The agents that have `plugin` on.
    fn plugin_agents(&self, plugin: &str) -> Result<Vec<agenium_store::core::agents::Agent>> {
        let entry = format!("{}{plugin}", agenium_skills::PLUGIN_PREFIX);
        Ok(self
            .store()
            .read(agenium_store::core::agents::agents)?
            .into_iter()
            .filter(|a| a.skills.contains(&entry))
            .collect())
    }

    /// Reads a plugin from its source into a staging folder and gives it to `f` (the folder goes after).
    async fn with_fetched<T>(
        &self,
        source: &Source,
        entry: Option<&str>,
        f: impl FnOnce(&layout::Pieces, &str) -> Result<T>,
    ) -> Result<T> {
        let staging = self
            .plugins_dir()
            .join(format!(".fetch-{}", ulid::Ulid::new()));
        std::fs::create_dir_all(&staging)?;
        let result = async {
            let (root, commit) = source::fetch(source, &staging).await?;
            let pieces = layout::read(&root, entry, &source.fallback_name())?;
            f(&pieces, &commit)
        }
        .await;
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    /// What installing `source` (`owner/repo[@ref]` or a folder) would bring; `entry` names the plugin of a
    /// marketplace listing several. Nothing is written.
    pub async fn preview_plugin(&self, source: &str, entry: Option<&str>) -> Result<PluginPreview> {
        let src = Source::parse(source)?;
        let reference = src.reference().to_owned();
        let plugin = self
            .with_fetched(&src, entry, |p, commit| {
                plugin_record(&src, entry, &reference, p, commit)
            })
            .await?;
        self.previewed(plugin)
    }

    fn previewed(&self, plugin: Plugin) -> Result<PluginPreview> {
        let installed = self.plugin(&plugin.name);
        let agents = self
            .plugin_agents(&plugin.name)?
            .into_iter()
            .map(|a| a.name)
            .collect();
        Ok(PluginPreview::new(plugin, installed, agents))
    }

    /// What updating `name` from its vendor would bring (the newest commit of the ref it follows).
    pub async fn preview_plugin_update(&self, name: &str) -> Result<PluginPreview> {
        let old = self
            .plugin(name)
            .ok_or_else(|| invalid(format!("no plugin {name:?}")))?;
        let src = Source::parse(&old.source)?.at(&old.reference);
        let plugin = self
            .with_fetched(&src, old.entry.as_deref(), |p, commit| {
                plugin_record(&src, old.entry.as_deref(), &old.reference, p, commit)
            })
            .await?;
        if plugin.name != old.name {
            return Err(invalid(format!(
                "the vendor's plugin is now named {:?}, not {name:?}: install it as a new plugin",
                plugin.name
            )));
        }
        self.previewed(plugin)
    }

    /// Installs (or replaces, whole) the plugin `source` at the `commit` its preview showed. The skills and servers
    /// come from that commit only; a source that no longer serves it is refused.
    pub async fn install_plugin(
        &self,
        source: &str,
        reference: &str,
        entry: Option<&str>,
        commit: &str,
    ) -> Result<Plugin> {
        let src = Source::parse(source)?;
        let at = if commit == "local" {
            src.clone()
        } else {
            src.at(commit)
        };
        let dir = self.plugins_dir();
        sweep(&dir);
        let plugin = self
            .with_fetched(&at, entry, |p, got| {
                if got != commit {
                    return Err(invalid(format!(
                        "the source now serves {got}, not the {commit} shown: preview again"
                    )));
                }
                let plugin = plugin_record(&src, entry, reference, p, got)?;
                // Build it whole beside the old one, then swap.
                let fresh = dir.join(format!(".new-{}-{}", p.name, ulid::Ulid::new()));
                let built = (|| -> Result<()> {
                    std::fs::create_dir_all(&fresh)?;
                    let manifest = serde_json::to_string_pretty(&p.manifest_json())
                        .map_err(|e| invalid(e.to_string()))?;
                    std::fs::write(fresh.join(layout::MANIFEST), manifest)?;
                    for (name, from) in &p.skills {
                        let to = fresh.join("skills").join(name);
                        std::fs::create_dir_all(&to)?;
                        agenium_skills::copy_dir(from, &to).map_err(|e| invalid(e.to_string()))?;
                    }
                    // The hooks' scripts, in their folder as the plugin keeps them (`hooks/`).
                    for (folder, from) in &p.hook_dirs {
                        let to = fresh.join(folder);
                        std::fs::create_dir_all(&to)?;
                        agenium_skills::copy_dir(from, &to).map_err(|e| invalid(e.to_string()))?;
                    }
                    Ok(())
                })();
                if let Err(e) = built {
                    let _ = std::fs::remove_dir_all(&fresh);
                    return Err(e);
                }
                let dest = dir.join(&p.name);
                if dest.exists() {
                    let old = dir.join(format!(".old-{}-{}", p.name, ulid::Ulid::new()));
                    std::fs::rename(&dest, &old)?;
                    if let Err(e) = std::fs::rename(&fresh, &dest) {
                        let _ = std::fs::rename(&old, &dest);
                        let _ = std::fs::remove_dir_all(&fresh);
                        return Err(e.into());
                    }
                    let _ = std::fs::remove_dir_all(&old);
                } else {
                    std::fs::rename(&fresh, &dest)?;
                }
                let mut installed = Installed::load(&dir);
                let first = installed.get(&p.name).map(|i| i.installed_at.clone());
                installed.remove(&p.name);
                let now = chrono::Utc::now().to_rfc3339();
                installed.plugins.insert(
                    format!("{}@{}", p.name, src.text()),
                    vec![Install {
                        scope: "user".into(),
                        install_path: dest.clone(),
                        version: p.version.clone(),
                        installed_at: first.unwrap_or_else(|| now.clone()),
                        last_updated: now,
                        git_commit_sha: got.to_owned(),
                        source: src.text(),
                        reference: reference.to_owned(),
                        entry: entry.map(str::to_owned),
                        skipped: p.skipped.clone(),
                    }],
                );
                installed.save(&dir)?;
                Ok(plugin)
            })
            .await?;
        tracing::info!(plugin = %plugin.name, commit = %plugin.commit, "plugin installed");
        Ok(plugin)
    }

    /// Removes a plugin and turns it off for every agent that had it.
    pub fn remove_plugin(&self, name: &str) -> Result<()> {
        let root = self.plugins_dir();
        let mut installed = Installed::load(&root);
        if !agenium_skills::valid_name(name) || installed.get(name).is_none() {
            return Err(invalid(format!("no plugin {name:?}")));
        }
        let dir = root.join(name);
        let entry = format!("{}{name}", agenium_skills::PLUGIN_PREFIX);
        for a in self.plugin_agents(name)? {
            let rest: Vec<String> = a.skills.into_iter().filter(|s| *s != entry).collect();
            self.set_agent_skills(a.id, &rest)?;
            self.forget_plugin_sessions(a.id);
        }
        installed.remove(name);
        installed.save(&root)?;
        // Everything of it goes: skills, hook scripts, the data its hooks wrote.
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        sweep(&root);
        tracing::info!(plugin = name, "plugin removed");
        Ok(())
    }

    /// Ends the agent's kept CLI sessions: a resumed session would carry what a plugin's hooks said in its earlier
    /// runs, after the plugin is off or gone. The next run starts fresh with the instructions of now.
    fn forget_plugin_sessions(&self, id: AgentId) {
        let _ = std::fs::remove_dir_all(self.cli_sessions(id));
    }

    /// Turns a whole plugin on or off for an agent.
    pub fn toggle_agent_plugin(&self, id: AgentId, name: &str, on: bool) -> Result<Vec<String>> {
        if on && self.plugin(name).is_none() {
            return Err(invalid(format!("no plugin {name:?}")));
        }
        let entry = format!("{}{name}", agenium_skills::PLUGIN_PREFIX);
        let mut skills = self.agent(id)?.skills;
        let had = skills.contains(&entry);
        skills.retain(|s| *s != entry);
        if on {
            skills.push(entry);
            skills.sort();
        }
        self.set_agent_skills(id, &skills)?;
        if had && !on {
            self.forget_plugin_sessions(id);
        }
        Ok(skills)
    }

    /// The MCP servers of the plugins an agent has on, and their bindings (every tool of each).
    pub fn plugin_mcp(
        &self,
        skills: &[String],
    ) -> (Vec<agenium_mcp::Binding>, Vec<McpServerConfig>) {
        let mut servers = Vec::new();
        for p in skills.iter().filter_map(|e| agenium_skills::plugin_of(e)) {
            if let Some(p) = self.plugin(p) {
                servers.extend(p.mcp);
            }
        }
        let bindings = servers
            .iter()
            .map(|s| agenium_mcp::Binding {
                server: s.id.clone(),
                credentials: BTreeMap::new(),
                keys: BTreeMap::new(),
                tools: None,
            })
            .collect();
        (bindings, servers)
    }
}

fn plugin_record(
    source: &Source,
    entry: Option<&str>,
    reference: &str,
    p: &layout::Pieces,
    commit: &str,
) -> Result<Plugin> {
    let mut skills = BTreeMap::new();
    for (name, dir) in &p.skills {
        skills.insert(name.clone(), digest(dir)?);
    }
    Ok(Plugin {
        name: p.name.clone(),
        description: p.description.clone(),
        version: p.version.clone(),
        source: source.text(),
        reference: reference.to_owned(),
        entry: entry.map(str::to_owned),
        commit: commit.to_owned(),
        skills,
        mcp: p.mcp.clone(),
        hooks: p
            .hooks
            .iter()
            .flat_map(|(event, groups)| {
                groups
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                    .filter_map(|h| h["command"].as_str().map(|c| format!("{event}: {c}")))
                    .collect::<Vec<_>>()
            })
            .collect(),
        skipped: p.skipped.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Removes what an install or a fetch left half done (`.fetch-*`, `.new-*`, `.old-*`): a stopped app leaves no plugin
/// files behind.
fn sweep(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if [".fetch-", ".new-", ".old-"]
            .iter()
            .any(|p| n.starts_with(p))
        {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}
