#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::app::FixedCheck;
use agenium_orchestrator::*;
use agenium_secrets::MemoryKeyring;
use agenium_store::core::{agents, projects};

mod skills;
#[allow(unused_imports)]
pub use skills::*;

/// Fields drop in declaration order: the temp dir must come last, after the app has closed its databases. Windows
/// cannot delete an open file, `TempDir` ignores the failure, and the whole tree stayed behind (5 500 dirs, 8.8 GB on
/// C: in three days, 2026-10-25).
pub struct Fixture {
    pub root: PathBuf,
    pub keyring: Arc<MemoryKeyring>,
    pub app: App,
    pub project: projects::Project,
    pub _tmp: Scratch,
}

/// The test's folder. The commands of a worktree keep their TEMP outside it, under the system TEMP (`run_temp_dir`);
/// only a ticket that closes removes its own, and the test's other worktrees left theirs (2026-10-31). Before the
/// folder goes, every folder of its tree has its TEMP removed, under the path as written and as resolved (the user's
/// TEMP can be spelled with a Windows short name).
pub struct Scratch(tempfile::TempDir);

impl Scratch {
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let tmp = self.0.path();
        let mut dirs: Vec<PathBuf> = [Some(tmp.to_path_buf()), std::fs::canonicalize(tmp).ok()]
            .into_iter()
            .flatten()
            .collect();
        while let Some(d) = dirs.pop() {
            agenium_sandbox::remove_run_temp_dir(&d);
            if let Ok(entries) = std::fs::read_dir(&d) {
                dirs.extend(
                    entries
                        .flatten()
                        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                        .map(|e| e.path()),
                );
            }
        }
    }
}

pub struct Checks(pub Option<(bool, String)>);
impl CheckFactory for Checks {
    fn for_project(
        &self,
        _: &projects::Project,
        _: &agenium_orchestrator::manifest::ProjectManifest,
    ) -> Arc<dyn agenium_vcs::CheckRunner> {
        match &self.0 {
            Some((ok, log)) => Arc::new(FixedCheck(*ok, log.clone())),
            None => Arc::new(agenium_vcs::NoCheck),
        }
    }
}

pub fn open_app(root: &Path, keyring: Arc<MemoryKeyring>, check: Option<(bool, String)>) -> App {
    open_app_checked(root, keyring, Arc::new(Checks(check)))
}

pub fn open_app_checked(
    root: &Path,
    keyring: Arc<MemoryKeyring>,
    checks: Arc<dyn CheckFactory>,
) -> App {
    App::open(root, AppOptions { keyring, checks }).unwrap().0
}

pub fn fixture() -> Fixture {
    fixture_with(None)
}

pub fn fixture_with(check: Option<(bool, String)>) -> Fixture {
    fixture_checked(Arc::new(Checks(check)))
}

/// A fixture whose pre-merge checks come from `checks`.
pub fn fixture_checked(checks: Arc<dyn CheckFactory>) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let keyring = Arc::new(MemoryKeyring::default());
    let app = open_app_checked(&root, keyring.clone(), checks);
    let project = app
        .create_project(&CreateProject {
            name: "demo".into(),
            path: None,
            key_prefix: None,
            template: None,
            folder: None,
            repo: None,
        })
        .unwrap();
    Fixture {
        _tmp: Scratch(tmp),
        root,
        keyring,
        app,
        project,
    }
}

pub fn model() -> ModelBinding {
    ModelBinding {
        provider: "mock".into(),
        model: "scripted".into(),
        effort: Effort::MEDIUM,
        max_tokens: None,
    }
}

pub fn agent_spec(name: &str, role: &str, kind: RoleKind, tags: &[&str]) -> CreateAgent {
    CreateAgent {
        name: name.into(),
        role: role.into(),
        role_kind: kind,
        tags: tags.iter().map(|s| s.to_string()).collect(),
        model: model(),
    }
}

pub fn spec() -> RequirementSpec {
    RequirementSpec {
        goals: vec!["Greet users".into()],
        scope: vec![],
        out_of_scope: vec![],
        requirements: vec![Requirement {
            id: "REQ-1".into(),
            text: "A greeting file exists".into(),
            acceptance: vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: "hello.txt says hello".into(),
            }],
        }],
        constraints: vec![],
    }
}

pub fn plan(items: &[(&str, &[&str])]) -> TicketPlan {
    TicketPlan {
        spec_version: 1,
        milestones: vec![Milestone {
            id: "M1".into(),
            title: "Greeting".into(),
        }],
        items: items
            .iter()
            .map(|(id, deps)| PlanItem {
                id: id.to_string(),
                milestone: "M1".into(),
                title: format!("item {id}"),
                trace_to: vec!["REQ-1".into()],
                acceptance: vec![AcceptanceCriterion {
                    id: "AC-1".into(),
                    text: "done".into(),
                }],
                role: "worker".into(),
                depends_on: deps.iter().map(|s| s.to_string()).collect(),
                estimate: Estimate::S,
                scope_paths: vec![],
            })
            .collect(),
    }
}

pub fn new_ticket(item: &str, title: &str, deps: &[&str], scope: &[&str]) -> NewTicket {
    NewTicket {
        kind: TicketKind::Task,
        plan_item: item.into(),
        parent: None,
        title: title.into(),
        description: "Write the greeting".into(),
        acceptance: vec![AcceptanceCriterion {
            id: "AC-1".into(),
            text: "hello.txt says hello".into(),
        }],
        priority: 2,
        depends_on: deps.iter().map(|d| d.parse().unwrap()).collect(),
        required_role: None,
        required_tags: vec![],
        scope_paths: scope.iter().map(|s| s.to_string()).collect(),
        report_template: None,
        follows: None,
    }
}

pub fn report(key: &str) -> WorkReport {
    WorkReport {
        ticket_id: key.parse().unwrap(),
        summary: "Wrote hello.txt".into(),
        acceptance: vec![AcResult {
            ac_id: "AC-1".into(),
            status: AcStatus::Met,
            evidence: "deliverables/hello.txt".into(),
        }],
        artifacts: vec![],
        extension: serde_json::json!({}),
        risks: vec![],
        follow_ups: vec![],
        checks_not_run: vec![],
        beyond_ticket: vec![],
        tests_changed: vec![],
        confidence: 0.9,
    }
}

pub fn ctx(f: &Fixture, actor: Actor) -> CallCtx {
    CallCtx {
        project: f.project.id,
        actor,
        intent_id: Some(ulid::Ulid::new().to_string()),
        run: None,
    }
}

/// Solo agent + locked spec + approved plan with the given items.
pub fn solo_with_plan(f: &Fixture, items: &[(&str, &[&str])]) -> agents::Agent {
    let (_, a) = f
        .app
        .create_solo(
            f.project.id,
            &agent_spec("ada", "solo", RoleKind::Lead, &["rust"]),
        )
        .unwrap();
    let lead = Actor::Lead(a.id);
    f.app
        .apply(&ctx(f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    f.app
        .apply(&ctx(f, lead), Intent::CreatePlan(plan(items)))
        .unwrap();
    a
}

/// Solo agent + locked spec + approved plan whose items also declare their write scopes.
pub fn solo_with_scoped_plan(f: &Fixture, items: &[(&str, &[&str], &[&str])]) -> agents::Agent {
    let (_, a) = f
        .app
        .create_solo(
            f.project.id,
            &agent_spec("ada", "solo", RoleKind::Lead, &["rust"]),
        )
        .unwrap();
    let lead = Actor::Lead(a.id);
    f.app
        .apply(&ctx(f, lead), Intent::ProposeSpec(spec()))
        .unwrap();
    let mut p = plan(&[]);
    p.items = items
        .iter()
        .map(|(id, deps, scope)| PlanItem {
            id: (*id).to_string(),
            milestone: "M1".into(),
            title: format!("item {id}"),
            trace_to: vec!["REQ-1".into()],
            acceptance: vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: "done".into(),
            }],
            role: "worker".into(),
            depends_on: deps.iter().map(|s| (*s).to_string()).collect(),
            estimate: Estimate::S,
            scope_paths: scope.iter().map(|s| (*s).to_string()).collect(),
        })
        .collect();
    f.app.apply(&ctx(f, lead), Intent::CreatePlan(p)).unwrap();
    a
}

pub fn key(s: &str) -> TicketKey {
    s.parse().unwrap()
}

pub fn ticket(f: &Fixture, k: &str) -> Ticket {
    f.app.ticket(f.project.id, &key(k)).unwrap()
}

pub fn worktree(f: &Fixture, k: &str) -> PathBuf {
    Path::new(&f.project.path)
        .join(".agenium")
        .join("worktrees")
        .join(k)
}

/// Creates, assigns to `agent` and starts a ticket; returns its key.
pub fn started(
    f: &Fixture,
    agent: AgentId,
    item: &str,
    title: &str,
    deps: &[&str],
    scope: &[&str],
) -> TicketKey {
    let lead = lead_of(f);
    let out = f
        .app
        .apply(
            &ctx(f, lead),
            Intent::CreateTicket(new_ticket(item, title, deps, scope)),
        )
        .unwrap();
    let k: TicketKey = serde_json::from_value(out.result["ticket_id"].clone()).unwrap();
    f.app
        .apply(
            &ctx(f, lead),
            Intent::AssignTicket(AssignTicket {
                ticket_id: k.clone(),
                worker: agent,
                note: None,
            }),
        )
        .unwrap();
    k
}

pub fn lead_of(f: &Fixture) -> Actor {
    let (team, _) = f.app.team_of_project(f.project.id).unwrap().unwrap();
    Actor::Lead(team.lead.unwrap())
}

pub fn write(path: &Path, rel: &str, content: &str) {
    let p = path.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A context a lesson can be validated in that the record backs (ADR-118): `<project>/ticket:<key>` naming a closed
/// ticket of the fixture's project.
pub fn closed_context(f: &Fixture, key: &str) -> String {
    use agenium_store::core::tickets;
    f.app
        .store()
        .commit(
            None,
            "test",
            |w| -> std::result::Result<(), agenium_store::StoreError> {
                let mut t = tickets::insert(
                    w.tx,
                    &tickets::TicketInsert {
                        id: TicketId::new(),
                        key: key.parse().unwrap(),
                        project: f.project.id,
                        parent: None,
                        kind: TicketKind::Task,
                        title: "earlier work".into(),
                        description: String::new(),
                        acceptance: vec![AcceptanceCriterion {
                            id: "AC-1".into(),
                            text: "done".into(),
                        }],
                        plan_item: "PI-0".into(),
                        priority: 2,
                        depends_on: vec![],
                        required_role: None,
                        required_tags: vec![],
                        scope_paths: vec![],
                        report_template: "default".into(),
                        created_at: "2026-10-27T00:00:00.000Z".into(),
                    },
                )?;
                t.status = TicketStatus::Closed;
                tickets::update(w.tx, &t)
            },
        )
        .unwrap();
    format!("{}/ticket:{key}", f.project.id)
}
