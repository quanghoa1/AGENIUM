//! Team templates (architecture page "Template team & Role", ADR-58): a versioned TOML file with the team's roles
//! (Lead + workers: tags, ROLE.md text, skills, allowed paths, count, default model) and its report template.
//! Built-in: coding, pentest, research, content; user templates in `<root>/templates/<id>.toml` (same id = override).

use std::collections::HashSet;
use std::path::Path;

use agenium_core::*;
use serde::{Deserialize, Serialize};

use crate::app::{App, CreateAgent};
use crate::{OrchError, Result};

const BUILTIN: [&str; 4] = [
    include_str!("../templates/coding.toml"),
    include_str!("../templates/pentest.toml"),
    include_str!("../templates/research.toml"),
    include_str!("../templates/content.toml"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamTemplate {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Default WorkReport template of the project's tickets (e.g. `coding@1`).
    pub report_template: String,
    /// Command run in the worktree by the merge queue before a fast-forward (e.g. `cargo test`); empty = none.
    #[serde(default)]
    pub pre_merge: String,
    pub roles: Vec<RoleSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    pub id: String,
    pub kind: RoleKind,
    #[serde(default)]
    pub tags: Vec<String>,
    /// ROLE.md text (written to `<root>/roles/<template>-<role>/ROLE.md` when the team is created and the file does
    /// not exist yet; the user edits it there).
    #[serde(default)]
    pub prompt: String,
    /// The role's skills: in its folder (`<root>/roles/<template>-<role>/skills/`), else a library skill switched on
    /// for each of its agents when the team is created.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Write scope of the role's agents (worktree-relative globs; empty = the whole worktree).
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// `provider/model`; the model given at creation wins.
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default = "one")]
    pub count: u32,
    /// The team's planner (ADR-59): a worker that only drafts plan v1 for the Lead and never takes tickets.
    #[serde(default)]
    pub planner: bool,
}

fn one() -> u32 {
    1
}

fn slug_ok(s: &str) -> bool {
    agenium_core::validate::slug_ok(s, 40)
}

impl TeamTemplate {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let t: TeamTemplate =
            toml::from_str(text).map_err(|e| OrchError::Invalid(format!("team template: {e}")))?;
        t.validate()?;
        Ok(t)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Exactly one Lead (count 1), at least one worker, unique slug ids, 1..=10 agents per role.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| {
            Err(OrchError::Invalid(format!(
                "team template {:?}: {m}",
                self.id
            )))
        };
        if !slug_ok(&self.id) {
            return bad("id must be lowercase letters, digits or '-'".into());
        }
        if self.version.split('.').count() != 3
            || self.version.split('.').any(|p| p.parse::<u32>().is_err())
        {
            return bad(format!(
                "version {:?} is not MAJOR.MINOR.PATCH",
                self.version
            ));
        }
        if !agenium_core::ticket::is_template_ref(&self.report_template) {
            return bad(format!(
                "report_template {:?} is not name@version",
                self.report_template
            ));
        }
        let leads: Vec<_> = self
            .roles
            .iter()
            .filter(|r| r.kind == RoleKind::Lead)
            .collect();
        if leads.len() != 1 || leads[0].count != 1 {
            return bad("needs exactly one lead role with count 1".into());
        }
        if !self.roles.iter().any(|r| r.kind == RoleKind::Worker) {
            return bad("needs at least one worker role".into());
        }
        let planners: Vec<_> = self.roles.iter().filter(|r| r.planner).collect();
        if planners.len() > 1
            || planners
                .iter()
                .any(|r| r.kind != RoleKind::Worker || r.count != 1)
        {
            return bad("at most one planner role, of kind worker with count 1".into());
        }
        let mut seen = std::collections::HashSet::new();
        for r in &self.roles {
            if !slug_ok(&r.id) || !seen.insert(r.id.as_str()) {
                return bad(format!("role id {:?} is invalid or repeated", r.id));
            }
            if !(1..=10).contains(&r.count) {
                return bad(format!("role {}: count must be 1..=10", r.id));
            }
            if let Some(m) = &r.default_model {
                if m.split_once('/')
                    .is_none_or(|(p, m)| p.is_empty() || m.is_empty())
                {
                    return bad(format!(
                        "role {}: default_model must be provider/model",
                        r.id
                    ));
                }
            }
            for p in &r.allowed_paths {
                if p.starts_with('/') || p.split('/').any(|s| s == "..") {
                    return bad(format!(
                        "role {}: allowed path {p:?} must stay inside the worktree",
                        r.id
                    ));
                }
            }
        }
        Ok(())
    }

    /// Role directory name (global under `<root>/roles/`).
    pub fn role_dir(&self, role: &str) -> String {
        format!("{}-{}", self.id, role)
    }
}

pub fn builtin() -> Vec<TeamTemplate> {
    BUILTIN
        .iter()
        .map(|t| TeamTemplate::parse(t).expect("built-in team templates are valid"))
        .collect()
}

/// Agents created for a team from a template.
#[derive(Debug, Clone, Serialize)]
pub struct TeamFromTemplate {
    pub team: TeamId,
    pub agents: Vec<(String, String)>,
    /// Skills the template expects but the role folders do not have yet.
    pub missing_skills: Vec<String>,
    /// The roles that run on their own model, and those whose model is not usable here.
    pub model_notes: Vec<String>,
}

/// The models a team from a template is made with (each with the backend that runs it).
#[derive(Debug, Clone)]
pub struct TeamModels {
    pub model: (BackendKind, ModelBinding),
    pub lead: Option<(BackendKind, ModelBinding)>,
    pub planner: Option<(BackendKind, ModelBinding)>,
    /// Each role on its template's `default_model` when this machine runs it.
    pub role_models: bool,
}

impl App {
    /// Built-in templates, overridden or extended by `<root>/templates/*.toml`. Unreadable user files are skipped
    /// with a warning.
    pub fn team_templates(&self) -> Result<Vec<TeamTemplate>> {
        let mut all = builtin();
        let dir = self.layout().templates();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut files: Vec<_> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            files.sort();
            for f in files {
                match std::fs::read_to_string(&f)
                    .map_err(OrchError::from)
                    .and_then(|s| TeamTemplate::parse(&s))
                {
                    Ok(t) => {
                        all.retain(|b| b.id != t.id);
                        all.push(t);
                    }
                    Err(e) => {
                        tracing::warn!(file = %f.display(), error = %e, "team template skipped")
                    }
                }
            }
        }
        Ok(all)
    }

    pub fn team_template(&self, id: &str) -> Result<TeamTemplate> {
        self.team_templates()?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| OrchError::NotFound(format!("no team template {id:?}")))
    }

    /// The template wizard (M4): validates a template and stores it as `<root>/templates/<id>.toml` (same id as a
    /// built-in overrides it). Teams made from it before keep their role folders.
    pub fn save_team_template(&self, t: &TeamTemplate) -> Result<TeamTemplate> {
        t.validate()?;
        let dir = self.layout().templates();
        agenium_config::write_atomic(&dir.join(format!("{}.toml", t.id)), t.to_toml().as_bytes())?;
        self.team_template(&t.id)
    }

    /// Removes a user template (a built-in it overrode comes back); built-ins cannot be removed.
    pub fn remove_team_template(&self, id: &str) -> Result<()> {
        let file = self.layout().templates().join(format!("{id}.toml"));
        if !slug_ok(id) || !file.is_file() {
            return Err(OrchError::NotFound(format!("no user template {id:?}")));
        }
        Ok(std::fs::remove_file(file)?)
    }

    /// Whether `id` is (or overrides) one of the built-in templates, and whether the user saved one with this id.
    pub fn template_origin(&self, id: &str) -> (bool, bool) {
        (
            builtin().iter().any(|t| t.id == id),
            self.layout()
                .templates()
                .join(format!("{id}.toml"))
                .is_file(),
        )
    }

    /// A template from a team as it is: one role per role of its members (ROLE.md text, tags, write scope, planner),
    /// counted by members, the Lead's role as lead, each role's model as its default. To save as `id`.
    pub fn template_from_team(&self, team: TeamId, id: &str, name: &str) -> Result<TeamTemplate> {
        let (t, members) = self.team(team)?;
        let lead = t.lead;
        let mut roles: Vec<RoleSpec> = Vec::new();
        for m in &members {
            let short = m
                .role
                .split_once('-')
                .filter(|(prefix, _)| {
                    self.role_file(&m.role)
                        .template
                        .starts_with(&format!("{prefix}@"))
                })
                .map(|(_, rest)| rest.to_owned())
                .unwrap_or_else(|| m.role.clone());
            if let Some(r) = roles.iter_mut().find(|r| r.id == short) {
                r.count += 1;
                continue;
            }
            let info = self.role(&m.role).ok();
            roles.push(RoleSpec {
                id: short,
                kind: if Some(m.id) == lead {
                    RoleKind::Lead
                } else {
                    RoleKind::Worker
                },
                tags: info.as_ref().map(|i| i.tags.clone()).unwrap_or_default(),
                prompt: info.as_ref().map(|i| i.prompt.clone()).unwrap_or_default(),
                skills: vec![],
                allowed_paths: self.role_allowed_paths(&m.role),
                default_model: (!m.model.provider.is_empty() && !m.model.model.is_empty())
                    .then(|| format!("{}/{}", m.model.provider, m.model.model)),
                count: 1,
                planner: self.is_planner_role(&m.role),
            });
        }
        let (report_template, pre_merge) = match t.project {
            Some(p) => {
                let m = self.manifest(&self.project(p)?)?;
                (m.project.template, m.checks.pre_merge)
            }
            None => ("coding@1".to_owned(), String::new()),
        };
        let tpl = TeamTemplate {
            id: id.to_owned(),
            version: "1.0.0".into(),
            name: name.to_owned(),
            description: format!("Made from the team {}.", t.name),
            report_template,
            pre_merge,
            roles,
        };
        tpl.validate()?;
        Ok(tpl)
    }

    /// Validates a template file and stores it as `<root>/templates/<id>.toml` (a user template; same id as a
    /// built-in overrides it).
    pub fn import_team_template(&self, file: &Path) -> Result<TeamTemplate> {
        let text = std::fs::read_to_string(file)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", file.display())))?;
        let t = TeamTemplate::parse(&text)?;
        let dir = self.layout().templates();
        agenium_config::write_atomic(&dir.join(format!("{}.toml", t.id)), t.to_toml().as_bytes())?;
        Ok(t)
    }

    /// Creates the project's team from a template on API models: one agent per role slot named
    /// `[<prefix>-]<role>[-n]`, the role folders (ROLE.md and the write scope) when missing, and the project's default
    /// report template.
    pub fn create_team_from_template(
        &self,
        project: ProjectId,
        template: &str,
        model: &ModelBinding,
        lead_model: Option<&ModelBinding>,
        planner_model: Option<&ModelBinding>,
        prefix: &str,
    ) -> Result<TeamFromTemplate> {
        let api = |b: &ModelBinding| (BackendKind::Api, b.clone());
        self.create_team_with_models(
            project,
            template,
            &TeamModels {
                model: api(model),
                lead: lead_model.map(api),
                planner: planner_model.map(api),
                role_models: false,
            },
            prefix,
        )
    }

    /// A role's `provider/model` (a template's `default_model`) as a backend and binding, when this machine runs it: an
    /// API provider in the config, or an agent CLI that was found (its models listed).
    pub fn usable_model(&self, spec: &str) -> Option<(BackendKind, ModelBinding)> {
        let (source, id) = spec.trim().split_once('/')?;
        if source.is_empty() || id.is_empty() {
            return None;
        }
        let backend = if self.config().provider(source).is_some() {
            BackendKind::Api
        } else if agenium_cli_backend::CliKind::ALL
            .iter()
            .any(|k| k.program() == source)
            && self
                .model_cache()
                .models
                .get(&crate::model_cache::cli_key(source))
                .is_some_and(|l| !l.value.is_empty())
        {
            BackendKind::Cli
        } else {
            return None;
        };
        Some((
            backend,
            ModelBinding {
                provider: source.to_owned(),
                model: id.to_owned(),
                effort: Effort::default(),
                max_tokens: None,
            },
        ))
    }

    /// Creates the project's team from a template with `models`: the Lead's and the planner's when given, else the
    /// role's own model when asked for and usable here, else the team's; each agent runs on its model's backend.
    pub fn create_team_with_models(
        &self,
        project: ProjectId,
        template: &str,
        models: &TeamModels,
        prefix: &str,
    ) -> Result<TeamFromTemplate> {
        let t = self.team_template(template)?;
        let mut model_notes = Vec::new();
        let mut cli_agents: HashSet<String> = HashSet::new();
        let prefix = prefix.trim();
        // Names are unique across teams: a name already taken gets the next free number.
        let mut taken: HashSet<String> = self
            .agents()?
            .into_iter()
            .map(|a| a.name.to_lowercase())
            .collect();
        let mut lead = None;
        let mut workers = Vec::new();
        let mut missing_skills = Vec::new();
        // A role's skill is its folder's, else a library skill switched on for each of its agents (ADR-151).
        let library: HashSet<String> = self
            .skill_registry()
            .library()
            .skills
            .into_iter()
            .map(|s| s.meta.name)
            .collect();
        let mut from_library: Vec<(String, String)> = Vec::new();
        for r in &t.roles {
            let dir = self.layout().roles().join(t.role_dir(&r.id));
            for s in &r.skills {
                if dir.join("skills").join(s).join("SKILL.md").is_file() {
                    continue;
                }
                if library.contains(s) {
                    from_library.push((t.role_dir(&r.id), s.clone()));
                } else {
                    missing_skills.push(format!("{}: {s}", t.role_dir(&r.id)));
                }
            }
            let (backend, binding) = match (r.kind, &models.lead, &models.planner) {
                (RoleKind::Lead, Some(m), _) => m.clone(),
                (RoleKind::Worker, _, Some(m)) if r.planner => m.clone(),
                _ => match r.default_model.as_deref().filter(|_| models.role_models) {
                    Some(spec) => match self.usable_model(spec) {
                        Some(m) => {
                            model_notes.push(format!("{}: {spec}", r.id));
                            m
                        }
                        None => {
                            model_notes.push(format!(
                                "{}: {spec} is not usable here, so the team's model",
                                r.id
                            ));
                            models.model.clone()
                        }
                    },
                    None => models.model.clone(),
                },
            };
            let base = if prefix.is_empty() {
                r.id.clone()
            } else {
                format!("{prefix}-{}", r.id)
            };
            let mut n = 1;
            for _ in 0..r.count {
                let mut name = if r.count == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                };
                while taken.contains(&name.to_lowercase()) {
                    n += 1;
                    name = format!("{base}-{n}");
                }
                n += 1;
                taken.insert(name.to_lowercase());
                if backend == BackendKind::Cli {
                    cli_agents.insert(name.clone());
                }
                let spec = CreateAgent {
                    name,
                    role: t.role_dir(&r.id),
                    role_kind: r.kind,
                    tags: r.tags.clone(),
                    model: binding.clone(),
                };
                match r.kind {
                    RoleKind::Lead => lead = Some(spec),
                    RoleKind::Worker => workers.push(spec),
                }
            }
        }
        let lead = lead.ok_or_else(|| OrchError::Invalid("the template has no lead".into()))?;
        let p = self.project(project)?;
        let mut m = self.manifest(&p)?;
        m.project.template = t.report_template.clone();
        if !t.pre_merge.trim().is_empty() {
            m.checks.pre_merge = t.pre_merge.clone();
        }
        m.save(Path::new(&p.path))
            .map_err(|e| OrchError::Invalid(e.to_string()))?;
        // On main, so the worktrees, the merge check and the push on close carry it (it stayed an uncommitted change).
        let path = Path::new(&p.path);
        if path.join(".git").exists() {
            agenium_vcs::Repo::open(path, &p.main_branch).commit_file(
                path,
                crate::manifest::FILE,
                &format!("Agenium: team template {} sets the project's checks", t.id),
            )?;
        }
        let (team, agents) = self.create_team(project, &t.name, &lead, &workers)?;
        // A model of an agent CLI runs on that CLI.
        for a in agents.iter().filter(|a| cli_agents.contains(&a.name)) {
            self.set_agent_backend(a.id, BackendKind::Cli, &a.model)?;
        }
        for a in &agents {
            for (_, skill) in from_library.iter().filter(|(role, _)| *role == a.role) {
                self.toggle_agent_skill(a.id, skill, true)?;
            }
        }
        // Role folders: written once, then owned by the user (ROLE.md is edited there).
        for r in &t.roles {
            self.materialize_role(&t, r)?;
        }
        Ok(TeamFromTemplate {
            team: team.id,
            agents: agents.into_iter().map(|a| (a.name, a.role)).collect(),
            missing_skills,
            model_notes,
        })
    }

    /// Writes a template role's folder (`ROLE.md`, `role.toml`) when the files do not exist yet.
    pub(crate) fn materialize_role(&self, t: &TeamTemplate, r: &RoleSpec) -> Result<()> {
        self.write_role_folder(&t.role_dir(&r.id), &format!("{}@{}", t.id, t.version), r)
    }

    /// Writes a preset role's folder `<root>/roles/<id>` (`ROLE.md`, `role.toml` naming `origin`) when the files do
    /// not exist yet.
    pub(crate) fn write_role_folder(&self, id: &str, origin: &str, r: &RoleSpec) -> Result<()> {
        let dir = self.layout().roles().join(id);
        std::fs::create_dir_all(&dir)?;
        let role_md = dir.join("ROLE.md");
        if !role_md.exists() && !r.prompt.trim().is_empty() {
            std::fs::write(&role_md, format!("{}\n", r.prompt.trim()))?;
        }
        let scope = dir.join("role.toml");
        if !scope.exists() {
            let body = toml::to_string(&RoleFile {
                template: origin.to_owned(),
                allowed_paths: r.allowed_paths.clone(),
                planner: r.planner,
                lead: r.kind == RoleKind::Lead,
                tags: r.tags.clone(),
            })
            .unwrap_or_default();
            std::fs::write(&scope, body)?;
        }
        Ok(())
    }

    pub(crate) fn role_file(&self, role: &str) -> RoleFile {
        std::fs::read_to_string(self.layout().roles().join(role).join("role.toml"))
            .ok()
            .and_then(|s| toml::from_str::<RoleFile>(s.strip_prefix('\u{feff}').unwrap_or(&s)).ok())
            .unwrap_or_default()
    }

    /// Write scope of a role (`<root>/roles/<role>/role.toml`; empty = whole worktree).
    pub fn role_allowed_paths(&self, role: &str) -> Vec<String> {
        self.role_file(role).allowed_paths
    }

    /// Whether agents of `role` are the team's planner (ADR-59).
    pub fn is_planner_role(&self, role: &str) -> bool {
        self.role_file(role).planner
    }

    /// The planner of the project's team, if it has one.
    pub fn planner(
        &self,
        project: ProjectId,
    ) -> Result<Option<agenium_store::core::agents::Agent>> {
        Ok(self
            .team_of_project(project)?
            .filter(|(t, _)| !t.solo)
            .and_then(|(_, members)| {
                members
                    .into_iter()
                    .find(|m| m.role_kind == RoleKind::Worker && self.is_planner_role(&m.role))
            }))
    }
}

/// `<root>/roles/<role>/role.toml`: settings of a role that are not prose.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RoleFile {
    /// `<template>@<version>` the role came from; empty for a role the user wrote.
    #[serde(default)]
    pub template: String,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub planner: bool,
    /// The role is meant to lead a team (a hint for the Lead suggestion; the user decides, ADR-61).
    #[serde(default)]
    pub lead: bool,
    /// Default capability tags of the role's agents.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid_and_round_trip() {
        let all = builtin();
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["coding", "pentest", "research", "content"]
        );
        for t in &all {
            assert_eq!(&TeamTemplate::parse(&t.to_toml()).unwrap(), t);
        }
    }

    #[test]
    fn validation_rejects_bad_templates() {
        let base = builtin().remove(0);
        let mut two_leads = base.clone();
        two_leads.roles[1].kind = RoleKind::Lead;
        assert!(two_leads.validate().is_err());
        let mut no_workers = base.clone();
        no_workers.roles.retain(|r| r.kind == RoleKind::Lead);
        assert!(no_workers.validate().is_err());
        let mut escape = base.clone();
        escape.roles[1].allowed_paths = vec!["../x/**".into()];
        assert!(escape.validate().is_err());
        let mut bad_model = base.clone();
        bad_model.roles[1].default_model = Some("gpt".into());
        assert!(bad_model.validate().is_err());
        let mut lead_planner = base.clone();
        lead_planner.roles[0].planner = true;
        assert!(lead_planner.validate().is_err(), "the planner is a worker");
        let mut bad_id = base;
        bad_id.id = "Coding Team".into();
        assert!(bad_id.validate().is_err());
    }
}
