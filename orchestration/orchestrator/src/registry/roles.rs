//! Roles: the template presets and the user's own.

use super::*;

/// A role an agent can take: a folder under `<root>/roles/<id>` or a template preset not written yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleInfo {
    pub id: String,
    /// ROLE.md text.
    pub prompt: String,
    pub tags: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub planner: bool,
    /// Meant to lead a team (suggestion only).
    pub lead: bool,
    /// `<template>@<version>` for a template role, empty for the user's own.
    pub template: String,
    /// The folder exists (a preset is written when an agent first takes it).
    pub exists: bool,
    /// Live agents with this role.
    pub agents: usize,
    /// Advice about the ROLE.md text (sections, length, near-duplicate roles; ADR-64).
    pub lint: Vec<String>,
}

/// A role written by the user (or a preset edited before use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleDef {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub planner: bool,
    #[serde(default)]
    pub lead: bool,
}

impl App {
    /// Role folders, then the template roles not written yet (presets).
    pub fn roles(&self) -> Result<Vec<RoleInfo>> {
        let live = self.agents()?;
        let count = |id: &str| live.iter().filter(|a| a.role == id).count();
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.layout().roles()) {
            let mut dirs: Vec<_> = rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect();
            dirs.sort();
            for id in dirs {
                let f = self.role_file(&id);
                let prompt =
                    std::fs::read_to_string(self.layout().roles().join(&id).join("ROLE.md"))
                        .unwrap_or_default();
                out.push(RoleInfo {
                    agents: count(&id),
                    id,
                    prompt: prompt.trim().to_owned(),
                    tags: f.tags,
                    allowed_paths: f.allowed_paths,
                    planner: f.planner,
                    lead: f.lead,
                    template: f.template,
                    exists: true,
                    lint: vec![],
                });
            }
        }
        for t in self.team_templates()? {
            for r in &t.roles {
                let id = t.role_dir(&r.id);
                if out.iter().any(|x| x.id == id) {
                    continue;
                }
                out.push(RoleInfo {
                    agents: count(&id),
                    id,
                    prompt: r.prompt.trim().to_owned(),
                    tags: r.tags.clone(),
                    allowed_paths: r.allowed_paths.clone(),
                    planner: r.planner,
                    lead: r.kind == RoleKind::Lead,
                    template: format!("{}@{}", t.id, t.version),
                    exists: false,
                    lint: vec![],
                });
            }
        }
        for r in crate::role_library::library() {
            if out.iter().any(|x| x.id == r.id) {
                continue;
            }
            out.push(RoleInfo {
                agents: count(&r.id),
                prompt: r.prompt.trim().to_owned(),
                tags: r.tags,
                allowed_paths: r.allowed_paths,
                planner: r.planner,
                lead: r.kind == RoleKind::Lead,
                template: crate::role_library::LIBRARY_ORIGIN.into(),
                exists: false,
                lint: vec![],
                id: r.id,
            });
        }
        let texts: Vec<(String, String)> = out
            .iter()
            .map(|r| (r.id.clone(), r.prompt.clone()))
            .collect();
        let others: Vec<(&str, &str)> = texts
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        for r in &mut out {
            r.lint = crate::role_library::lint_role(&r.id, &r.prompt, &others);
        }
        Ok(out)
    }

    pub fn role(&self, id: &str) -> Result<RoleInfo> {
        self.roles()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| OrchError::NotFound(format!("role {id:?}")))
    }

    /// Writes (or rewrites) a role folder: ROLE.md and role.toml. The template origin of an existing role is kept.
    pub fn put_role(&self, def: &RoleDef) -> Result<RoleInfo> {
        if !slug_ok(&def.id) {
            return Err(invalid(format!(
                "role id {:?}: lowercase letters, digits and '-' (at most 60)",
                def.id
            )));
        }
        if def.prompt.trim().is_empty() {
            return Err(invalid("the role needs a description (ROLE.md)"));
        }
        if def.planner && def.lead {
            return Err(invalid("a planner role cannot lead (ADR-59)"));
        }
        for p in &def.allowed_paths {
            if p.contains("..") || Path::new(p).is_absolute() {
                return Err(invalid(format!(
                    "allowed path {p:?} must stay inside the worktree"
                )));
            }
        }
        let dir = self.layout().roles().join(&def.id);
        agenium_config::write_atomic(
            &dir.join("ROLE.md"),
            format!("{}\n", def.prompt.trim()).as_bytes(),
        )?;
        let old = self.role_file(&def.id);
        let file = RoleFile {
            template: old.template,
            allowed_paths: def.allowed_paths.clone(),
            planner: def.planner,
            lead: def.lead,
            tags: def.tags.clone(),
        };
        agenium_config::write_atomic(
            &dir.join("role.toml"),
            toml::to_string(&file).unwrap_or_default().as_bytes(),
        )?;
        self.role(&def.id)
    }

    /// Deletes a role folder no live agent uses.
    pub fn remove_role(&self, id: &str) -> Result<()> {
        if !slug_ok(id) {
            return Err(invalid(format!("role id {id:?}")));
        }
        let users: Vec<String> = self
            .agents()?
            .into_iter()
            .filter(|a| a.role == id)
            .map(|a| a.name)
            .collect();
        if !users.is_empty() {
            return Err(OrchError::Conflict(format!(
                "agents still have role {id}: {}",
                users.join(", ")
            )));
        }
        let dir = self.layout().roles().join(id);
        if !dir.is_dir() {
            return Err(OrchError::NotFound(format!("role {id:?}")));
        }
        std::fs::remove_dir_all(dir).map_err(io)
    }

    /// Makes sure the role folder exists, writing a template preset when needed.
    pub(super) fn ensure_role(&self, id: &str) -> Result<()> {
        if self.layout().roles().join(id).is_dir() {
            return Ok(());
        }
        for t in self.team_templates()? {
            if let Some(r) = t.roles.iter().find(|r| t.role_dir(&r.id) == id) {
                return self.materialize_role(&t, r);
            }
        }
        if let Some(r) = crate::role_library::library()
            .into_iter()
            .find(|r| r.id == id)
        {
            return self.write_role_folder(id, crate::role_library::LIBRARY_ORIGIN, &r);
        }
        Err(OrchError::NotFound(format!(
            "role {id:?}: create it first or pick a template role"
        )))
    }

    // ------------------------------------------------------------ agents
}
