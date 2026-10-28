//! Role commands. Split out of `main.rs`.

use crate::*;

pub(crate) async fn run(cli: &Cli, rc: &RoleCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match rc {
        RoleCmd::Proposals { project: pname } => {
            let p = project(&app, pname)?;
            for r in app.role_proposals(p.id)? {
                println!(
                    "{}	{}	{:?}	{}: {}",
                    r.id, r.body.role_id, r.status, r.body.title, r.body.description
                );
            }
        }
        RoleCmd::List => {
            for r in app.roles()? {
                let flags = [
                    r.lead.then_some("lead"),
                    r.planner.then_some("planner"),
                    (!r.exists).then_some("preset"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(",");
                println!(
                    "{}	{}	[{}]	{} agent(s)	{}",
                    r.id,
                    if r.template.is_empty() {
                        "yours"
                    } else {
                        &r.template
                    },
                    r.tags.join(","),
                    r.agents,
                    flags
                );
            }
        }
        RoleCmd::Put {
            id,
            prompt_file,
            tags,
            allowed_paths,
            planner,
            lead,
        } => {
            let prompt = std::fs::read_to_string(prompt_file)
                .with_context(|| prompt_file.display().to_string())?;
            let r = app.put_role(&agenium_orchestrator::registry::RoleDef {
                id: id.clone(),
                prompt,
                tags: tags.clone(),
                allowed_paths: allowed_paths.clone(),
                planner: *planner,
                lead: *lead,
            })?;
            println!("role {} saved", r.id);
        }
        RoleCmd::Remove { id } => {
            app.remove_role(id)?;
            println!("role {id} removed");
        }
    }
    Ok(())
}
