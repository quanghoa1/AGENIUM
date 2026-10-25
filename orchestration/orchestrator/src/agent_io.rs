//! Agent export / import as JSONL (M15-5) and agent deletion (decision 32).
//!
//! Export file (UTF-8 JSONL, zstd-compressed when the name ends in `.zst`), one object per line:
//! 1. `{"kind":"agenium-agent","version":1,...}` header,
//! 2. `{"kind":"profile",...}` (name, role, tags, model binding, MCP bindings, skill binding; no ids, no team),
//! 3. `{"kind":"file","path":..,"text"|"hex":..}` for every file in `<root>/agents/<id>/` (SOUL.md, skills),
//! 4. the memory database as `table`/`row` lines (facts with their history, usage log, conversations, overlays),
//! 5. `{"kind":"end","lines":n,"sha256":..}` over the preceding lines.
//!
//! The export never contains secrets: a known vault value or a secret-looking string refuses the export (the
//! latter can be overridden). Vectors are not exported; the importer re-embeds from the text.

use std::io::{BufRead, Write};
use std::path::{Component, Path, PathBuf};

use agenium_core::*;
use agenium_memory::Embedder;
use agenium_store::core::agents;
use agenium_store::StoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app::{App, CreateAgent};
use crate::{OrchError, Result};

pub const FORMAT: &str = "agenium-agent";
pub const VERSION: u32 = 1;
/// Upper bound for the files of an agent folder in one export.
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub role: String,
    pub role_kind: RoleKind,
    pub tags: Vec<String>,
    pub backend: BackendKind,
    pub model: ModelBinding,
    pub mcp: serde_json::Value,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ExportSummary {
    pub path: PathBuf,
    pub files: u64,
    pub memory_rows: u64,
    pub lines: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Export even when secret-looking strings are found (known vault values are always refused).
    pub allow_secret_findings: bool,
}

#[derive(Debug, Clone)]
pub struct ImportRequest {
    /// The imported agent becomes the Solo agent of this project (team mode import arrives with M4).
    pub project: ProjectId,
    /// New name (default: the exported name).
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeleteReport {
    pub agent: AgentId,
    pub name: String,
    /// Paths that could not be removed now (retried at the next start).
    pub pending: Vec<String>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

fn io(path: &Path, e: std::io::Error) -> OrchError {
    OrchError::Invalid(format!("{}: {e}", path.display()))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // staging folders of skill imports
        }
        let ft = e.file_type().map_err(|x| io(&e.path(), x))?;
        if ft.is_symlink() {
            return Err(OrchError::Invalid(format!(
                "symbolic link in the agent folder: {}",
                e.path().display()
            )));
        }
        if ft.is_dir() {
            walk(&e.path(), base, out)?;
        } else if ft.is_file() {
            let rel = e
                .path()
                .strip_prefix(base)
                .unwrap_or(&e.path())
                .to_path_buf();
            out.push((rel.to_string_lossy().replace('\\', "/"), e.path()));
        }
    }
    Ok(())
}

/// A file path from an export: relative, no `..`, no root.
fn safe_rel(p: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => out.push(s),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    let f = std::fs::File::open(path).map_err(|e| io(path, e))?;
    let reader: Box<dyn std::io::Read> = if path.extension().is_some_and(|e| e == "zst") {
        Box::new(zstd::Decoder::new(f).map_err(|e| io(path, e))?)
    } else {
        Box::new(f)
    };
    std::io::BufReader::new(reader)
        .lines()
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| io(path, e))
}

fn digest(lines: &[String]) -> String {
    let mut h = Sha256::new();
    for l in lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

impl App {
    /// Writes an agent export to `dst` (atomically: temp file + rename).
    pub fn export_agent(
        &self,
        id: AgentId,
        dst: &Path,
        embedder: &dyn Embedder,
        opts: &ExportOptions,
    ) -> Result<ExportSummary> {
        let a = self.agent(id)?;
        if self.store().read(|c| agents::is_deleted(c, id))? {
            return Err(OrchError::NotFound(format!("agent {id}")));
        }
        let mut lines = vec![serde_json::json!({
            "kind": FORMAT,
            "version": VERSION,
            "exported_at": chrono::Utc::now().to_rfc3339(),
            "agent": id,
            "name": a.name,
            "embedder": embedder.model_id(),
        })
        .to_string()];
        let profile = AgentProfile {
            name: a.name.clone(),
            role: a.role.clone(),
            role_kind: a.role_kind,
            tags: a.tags.clone(),
            backend: a.backend,
            model: a.model.clone(),
            mcp: a.mcp.clone(),
            skills: a.skills.clone(),
        };
        lines.push(serde_json::json!({ "kind": "profile", "profile": profile }).to_string());
        let dir = self.layout().agent_dir(&id);
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files)?;
        let mut total = 0u64;
        for (rel, path) in &files {
            let bytes = std::fs::read(path).map_err(|e| io(path, e))?;
            total += bytes.len() as u64;
            if total > MAX_FILE_BYTES {
                return Err(OrchError::Invalid(format!(
                    "the agent folder holds more than {} MiB",
                    MAX_FILE_BYTES >> 20
                )));
            }
            let line = match String::from_utf8(bytes) {
                Ok(t) => serde_json::json!({ "kind": "file", "path": rel, "text": t }),
                Err(e) => {
                    serde_json::json!({ "kind": "file", "path": rel, "hex": hex(e.as_bytes()) })
                }
            };
            lines.push(line.to_string());
        }
        let mem = self.agent_store(id, embedder.dim(), embedder.model_id())?;
        let mut buf = Vec::new();
        let memory_rows = mem.export_jsonl(&mut buf)?;
        lines.extend(String::from_utf8_lossy(&buf).lines().map(str::to_owned));

        // No secrets leave the machine.
        let mut findings = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if self.redactor().redact(l) != *l {
                return Err(OrchError::Invalid(format!(
                    "line {} of the export contains a stored secret; export refused",
                    i + 1
                )));
            }
            for f in agenium_secrets::scan::scan_text(l) {
                findings.push(format!("line {}: {} ({})", i + 1, f.rule, f.preview));
            }
        }
        if !findings.is_empty() && !opts.allow_secret_findings {
            return Err(OrchError::Invalid(format!(
                "possible secrets in the agent's data, export refused (pass allow_secret_findings to override): {}",
                findings.join("; ")
            )));
        }
        let n = lines.len() as u64;
        let end = serde_json::json!({ "kind": "end", "lines": n, "sha256": digest(&lines) });
        lines.push(end.to_string());

        let tmp = dst.with_extension(format!(
            "{}.partial",
            dst.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        {
            let f = std::fs::File::create(&tmp).map_err(|e| io(&tmp, e))?;
            let mut w: Box<dyn Write> = if dst.extension().is_some_and(|e| e == "zst") {
                Box::new(
                    zstd::Encoder::new(f, 3)
                        .map_err(|e| io(&tmp, e))?
                        .auto_finish(),
                )
            } else {
                Box::new(std::io::BufWriter::new(f))
            };
            for l in &lines {
                writeln!(w, "{l}").map_err(|e| io(&tmp, e))?;
            }
            w.flush().map_err(|e| io(&tmp, e))?;
        }
        std::fs::rename(&tmp, dst).map_err(|e| io(dst, e))?;
        Ok(ExportSummary {
            path: dst.to_path_buf(),
            files: files.len() as u64,
            memory_rows,
            lines: n + 1,
        })
    }

    /// Imports an export as a new agent (new id). Everything is validated before anything is created; a failure
    /// after creation deletes the half-imported agent again.
    pub fn import_agent(
        &self,
        src: &Path,
        req: &ImportRequest,
        embedder: &dyn Embedder,
    ) -> Result<agents::Agent> {
        let mut lines = read_lines(src)?;
        let bad = |m: &str| OrchError::Invalid(format!("{}: {m}", src.display()));
        let end: serde_json::Value = serde_json::from_str(lines.pop().as_deref().unwrap_or(""))
            .map_err(|_| bad("missing end line"))?;
        if end["kind"] != "end"
            || end["lines"].as_u64() != Some(lines.len() as u64)
            || end["sha256"].as_str() != Some(digest(&lines).as_str())
        {
            return Err(bad(
                "the file is truncated or was modified (checksum mismatch)",
            ));
        }
        let parsed: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l))
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| bad(&format!("invalid JSON line: {e}")))?;
        let head = parsed.first().ok_or_else(|| bad("empty file"))?;
        if head["kind"] != FORMAT {
            return Err(bad("not an Agenium agent export"));
        }
        if head["version"].as_u64() != Some(VERSION as u64) {
            return Err(bad(&format!(
                "unsupported export version {}",
                head["version"]
            )));
        }
        let profile: AgentProfile = parsed
            .iter()
            .find(|v| v["kind"] == "profile")
            .map(|v| serde_json::from_value(v["profile"].clone()))
            .transpose()
            .map_err(|e| bad(&format!("invalid profile: {e}")))?
            .ok_or_else(|| bad("no profile"))?;
        let mut files = Vec::new();
        for v in parsed.iter().filter(|v| v["kind"] == "file") {
            let p = v["path"].as_str().unwrap_or_default();
            let rel = safe_rel(p).ok_or_else(|| bad(&format!("unsafe file path {p:?}")))?;
            let bytes = match (v["text"].as_str(), v["hex"].as_str()) {
                (Some(t), _) => t.as_bytes().to_vec(),
                (None, Some(h)) => unhex(h).ok_or_else(|| bad(&format!("bad hex in {p}")))?,
                _ => return Err(bad(&format!("file {p} has no content"))),
            };
            files.push((rel, bytes));
        }
        let memory: Vec<serde_json::Value> = parsed
            .iter()
            .filter(|v| v["kind"] == "table" || v["kind"] == "row")
            .cloned()
            .collect();
        let name = req.name.clone().unwrap_or_else(|| profile.name.clone());
        self.check_names_free(&[&name])?;
        if let Some(b) = profile
            .skills
            .iter()
            .find(|s| !agenium_skills::valid_name(s))
        {
            return Err(bad(&format!("invalid skill name {b:?} in the binding")));
        }

        // A Solo agent is Lead + Worker (decision 6).
        let role = if profile.role_kind == RoleKind::Lead {
            profile.role.clone()
        } else {
            "solo".into()
        };
        let (_, agent) = self.create_solo(
            req.project,
            &CreateAgent {
                name,
                role,
                role_kind: RoleKind::Lead,
                tags: profile.tags.clone(),
                model: profile.model.clone(),
            },
        )?;
        let finish = (|| -> Result<()> {
            self.store()
                .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                    agents::set_agent_skills(w.tx, agent.id, &profile.skills)?;
                    agents::set_agent_mcp(w.tx, agent.id, &profile.mcp)?;
                    Ok(())
                })?;
            let dir = self.layout().agent_dir(&agent.id);
            for (rel, bytes) in &files {
                let p = dir.join(rel);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
                }
                std::fs::write(&p, bytes).map_err(|e| io(&p, e))?;
            }
            let mem = self.agent_store(agent.id, embedder.dim(), embedder.model_id())?;
            mem.import_jsonl(&memory)?;
            mem.reindex(|t| embedder.embed(t))?;
            mem.reindex_entities(|t| embedder.embed(t))?;
            Ok(())
        })();
        if let Err(e) = finish {
            let _ = self.remove_agent(agent.id);
            return Err(e);
        }
        self.agent(agent.id)
    }

    /// Deletes an agent for good (decision 32): `typed_name` must equal its name; refused while it works or still
    /// has open tickets (as assignee, or as the Lead of a project with open tickets). Its memory database, skills,
    /// SOUL.md history and folder are removed; the database row stays as a tombstone for the history.
    pub fn delete_agent(&self, id: AgentId, typed_name: &str) -> Result<DeleteReport> {
        let a = self.agent(id)?;
        if self.store().read(|c| agents::is_deleted(c, id))? {
            return Err(OrchError::NotFound(format!("agent {id}")));
        }
        if typed_name != a.name {
            return Err(OrchError::Invalid(
                "type the agent's exact name to confirm the deletion".into(),
            ));
        }
        if a.status == AgentStatus::Working {
            return Err(OrchError::Conflict(format!(
                "{} is working; wait for or stop its run first",
                a.name
            )));
        }
        let (team, members) = match a.team {
            Some(t) => (
                Some(self.store().read(|c| agents::get_team(c, t))?),
                self.store().read(|c| agents::agents_of_team(c, t))?,
            ),
            None => (None, vec![]),
        };
        let leads = team.as_ref().is_some_and(|t| t.lead == Some(id));
        if leads && members.iter().any(|m| m.id != id) {
            return Err(OrchError::Conflict(format!(
                "{} leads a team with other members; replace the Lead first",
                a.name
            )));
        }
        let mut open = Vec::new();
        for p in self.projects()? {
            for t in self.tickets(p.id)? {
                let leads_it = leads && team.as_ref().is_some_and(|t| t.project == Some(p.id));
                if !t.status.is_terminal() && (t.assignee == Some(id) || leads_it) {
                    open.push(t.key.to_string());
                }
            }
        }
        if !open.is_empty() {
            return Err(OrchError::Conflict(format!(
                "{} still has open tickets ({}); reassign, close or cancel them first",
                a.name,
                open.join(", ")
            )));
        }
        let pending = self.remove_agent(id)?;
        Ok(DeleteReport {
            agent: id,
            name: a.name,
            pending,
        })
    }

    /// Tombstones the row (freeing the project of a team left empty) and removes the agent's files.
    fn remove_agent(&self, id: AgentId) -> Result<Vec<String>> {
        let a = self.agent(id)?;
        self.store()
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::tombstone_agent(w.tx, id)?;
                if let Some(t) = a.team {
                    if agents::agents_of_team(w.tx, t)?.is_empty() {
                        agents::set_team_project(w.tx, t, None)?;
                    }
                }
                w.emit(DomainEvent::AgentDeleted {
                    agent: id,
                    name: a.name.clone(),
                })?;
                Ok(())
            })?;
        Ok(self.purge_agent_files(id))
    }

    /// Removes the files of a deleted agent; returns what is still locked (retried by `purge_deleted_agents`).
    pub(crate) fn purge_agent_files(&self, id: AgentId) -> Vec<String> {
        self.forget_agent_store(id);
        let mut pending = Vec::new();
        let db = self.layout().agent_db(&id);
        for suffix in ["", "-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", db.display()));
            if p.exists() && std::fs::remove_file(&p).is_err() {
                pending.push(p.display().to_string());
            }
        }
        for dir in [
            self.layout().agent_dir(&id),
            self.layout().mcp_work().join(id.to_string()),
        ] {
            if dir.exists() && std::fs::remove_dir_all(&dir).is_err() {
                pending.push(dir.display().to_string());
            }
        }
        pending
    }

    /// Finishes deletions whose files were locked (called at startup).
    pub fn purge_deleted_agents(&self) -> Result<()> {
        let ids: Vec<AgentId> = self.store().read(agents::deleted_agents)?;
        for id in ids {
            let left = self.purge_agent_files(id);
            if !left.is_empty() {
                tracing::warn!(agent = %id, ?left, "files of a deleted agent are still locked");
            }
        }
        Ok(())
    }
}
