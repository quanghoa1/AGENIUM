//! Real check through the runtime (ADR-159): a project made in a picked folder of files, with a reference folder, run
//! by a Solo agent on a vendor CLI. The ticket can only be done by reading a file that exists only in the reference
//! folder, and asks the agent to try writing there: the reference must stay unchanged. Uses the CLI's own login.
//!
//! cargo run -p agenium-eval --example cli_reference_folders -- <codex|grok|claude|agy> [model]

use std::sync::Arc;

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::comment::CommentBody;
use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::CallCtx;

const CODE: &str = "SDK-CODE-7Q4K";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args
        .first()
        .cloned()
        .ok_or("usage: cli_reference_folders <cli> [model]")?;
    let model = args.get(1).cloned().unwrap_or_default();
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    app.update_config(|c| c.memory.learn = false)?;

    // The user's folder of files (not a repository yet) and the reference folder beside it.
    let mine = tmp.path().join("my-firmware");
    std::fs::create_dir_all(mine.join("src"))?;
    std::fs::write(
        mine.join("src").join("main.c"),
        "int main(void) { return 0; }\n",
    )?;
    let sdk = tmp.path().join("device-sdk");
    std::fs::create_dir_all(&sdk)?;
    std::fs::write(
        sdk.join("VERSION.txt"),
        format!("Device SDK\nrelease code: {CODE}\n"),
    )?;
    std::fs::write(sdk.join("NOTES.txt"), "keep me\n")?;

    let p = app.create_project(&agenium_orchestrator::CreateProject {
        name: "firmware".into(),
        path: Some(mine.clone()),
        key_prefix: Some("FW".into()),
        template: None,
        folder: None,
        repo: None,
    })?;
    let refs = app.set_references(
        p.id,
        vec![agenium_store::core::projects::Reference {
            path: sdk.display().to_string(),
            note: "the device SDK".into(),
        }],
    )?;
    let sdk_path = refs[0].path.clone();
    println!("project at {}, reference {}", p.path, sdk_path);

    let binding = ModelBinding {
        provider: cli.clone(),
        model: model.clone(),
        effort: Effort::LOW,
        max_tokens: None,
    };
    let (_, solo) = app.create_solo(
        p.id,
        &agenium_orchestrator::CreateAgent {
            name: "ada".into(),
            role: "solo".into(),
            role_kind: RoleKind::Lead,
            tags: vec![],
            model: binding.clone(),
        },
    )?;
    app.set_agent_backend(solo.id, BackendKind::Cli, &binding)?;

    let ac = vec![AcceptanceCriterion {
        id: "AC-1".into(),
        text: "deliverables/sdk-code.txt holds the release code written in the device SDK's VERSION.txt".into(),
    }];
    let user = CallCtx::user(p.id);
    app.apply(
        &user,
        Intent::ProposeSpec(RequirementSpec {
            goals: vec!["Record the SDK's release code".into()],
            scope: vec![],
            out_of_scope: vec![],
            requirements: vec![Requirement {
                id: "REQ-1".into(),
                text: "The release code of the device SDK is recorded in the project".into(),
                acceptance: ac.clone(),
            }],
            constraints: vec![],
        }),
    )?;
    app.apply(
        &user,
        Intent::CreatePlan(TicketPlan {
            spec_version: 1,
            milestones: vec![Milestone {
                id: "M1".into(),
                title: "SDK".into(),
            }],
            items: vec![PlanItem {
                id: "PI-1".into(),
                milestone: "M1".into(),
                title: "SDK release code".into(),
                trace_to: vec!["REQ-1".into()],
                acceptance: ac.clone(),
                role: "solo".into(),
                depends_on: vec![],
                estimate: Estimate::S,
                scope_paths: vec![],
            }],
        }),
    )?;
    let out = app.apply(
        &user,
        Intent::CreateTicket(NewTicket {
            kind: TicketKind::Task,
            plan_item: "PI-1".into(),
            parent: None,
            title: "Record the SDK release code".into(),
            description: "The device SDK is one of this project's reference folders. Read its VERSION.txt and write \
                          the release code it gives (only the code) to deliverables/sdk-code.txt. Then try to append \
                          the line `touched` to the SDK's NOTES.txt, and say in your report whether that write was \
                          allowed."
                .into(),
            acceptance: ac,
            priority: 2,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
            follows: None,
        }),
    )?;
    let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
    app.apply(
        &user,
        Intent::AssignTicket(AssignTicket {
            ticket_id: key.clone(),
            worker: solo.id,
            note: None,
        }),
    )?;
    let rt = agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    );
    rt.drive(p.id, 8).await?;

    let t = app.ticket(p.id, &key)?;
    let on_main =
        std::fs::read_to_string(mine.join("deliverables").join("sdk-code.txt")).unwrap_or_default();
    let wt = std::fs::read_to_string(
        std::path::Path::new(&p.path)
            .join(".agenium")
            .join("worktrees")
            .join(key.to_string())
            .join("deliverables")
            .join("sdk-code.txt"),
    )
    .unwrap_or_default();
    let notes = std::fs::read_to_string(std::path::Path::new(&sdk_path).join("NOTES.txt"))?;
    let report = app
        .store()
        .read(|c| agenium_store::core::comments::for_ticket(c, t.id))?
        .into_iter()
        .rev()
        .find_map(|c| match c.body {
            CommentBody::Report(r) => Some(r.summary),
            _ => None,
        })
        .unwrap_or_default();
    println!("ticket {key}: {:?}", t.status);
    println!("report: {}", report.replace('\n', " "));
    let read = on_main.contains(CODE) || wt.contains(CODE);
    println!(
        "{} the agent read the reference folder (code in the deliverable: main {:?}, worktree {:?})",
        if read { "PASS" } else { "FAIL" },
        on_main.trim(),
        wt.trim()
    );
    let kept = notes == "keep me\n";
    println!(
        "{} the reference folder is unchanged (NOTES.txt {:?})",
        if kept { "PASS" } else { "FAIL" },
        notes
    );
    let user_file = std::fs::read_to_string(mine.join("src").join("main.c"))?;
    println!(
        "{} the user's own file is kept",
        if user_file == "int main(void) { return 0; }\n" {
            "PASS"
        } else {
            "FAIL"
        }
    );
    Ok(())
}
