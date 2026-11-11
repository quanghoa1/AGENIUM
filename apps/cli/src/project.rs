//! `agenium project`: projects of the root.

use super::*;

pub(crate) async fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Project(pc) => {
            let app = open(&root(cli)?)?;
            match pc {
                ProjectCmd::Create {
                    name,
                    path,
                    prefix,
                    template,
                    repo,
                } => {
                    // The tickets' prefix: the one given (checked), else one made from the name.
                    let prefix = match prefix {
                        Some(p) => {
                            let p = p.trim().to_uppercase();
                            app.check_key_prefix(&p)?;
                            p
                        }
                        None => app.suggest_key_prefix(name)?,
                    };
                    let p = app.create_project(&CreateProject {
                        name: name.clone(),
                        // A folder named on the command line is the one the user picked (ADR-159): a full path.
                        path: path.as_deref().map(std::path::absolute).transpose()?,
                        key_prefix: Some(prefix),
                        template: template.clone(),
                        folder: None,
                        repo: repo.clone(),
                    })?;
                    println!("project {} at {}", p.name, p.path);
                }
                ProjectCmd::List => {
                    for p in app.projects()? {
                        let state = app.run_state(p.id)?;
                        println!(
                            "{}\t{}\t{state:?}\t{}\t{}",
                            p.name, p.status, p.key_prefix, p.path
                        );
                    }
                }
                ProjectCmd::Pause { project: name } => {
                    let p = project(&app, name)?;
                    app.set_project_status(
                        p.id,
                        agenium_store::core::projects::ProjectStatus::Paused,
                    )?;
                    println!(
                        "{} paused (runs in progress stop at their next step in the app)",
                        p.name
                    );
                }
                ProjectCmd::Close { project: name } => {
                    let p = project(&app, name)?;
                    match app.close_project(p.id)? {
                        (Some(t), Some(q)) => println!(
                            "{} closed; team {} went on to {}, which waited for it",
                            p.name,
                            app.team(t)?.0.name,
                            app.project(q)?.name
                        ),
                        (Some(t), None) => println!(
                            "{} closed; team {} is free to take another project",
                            p.name,
                            app.team(t)?.0.name
                        ),
                        (None, _) => println!("{} closed", p.name),
                    }
                }
                ProjectCmd::Reopen {
                    project: name,
                    team,
                } => {
                    use agenium_orchestrator::project_close::Taker;
                    let p = project(&app, name)?;
                    // Default: the team that worked on it, now when free, else once it is free.
                    let taker = match team {
                        Some(t) => Taker::Team(team_named(&app, t)?),
                        None => match app.previous_team(p.id)? {
                            Some(t) => Taker::WaitFor(t),
                            None => Taker::Nobody,
                        },
                    };
                    app.reopen_project(p.id, taker)?;
                    match (app.team_of_project(p.id)?, app.waits_for(p.id)?) {
                        (Some((t, _)), _) => {
                            println!("{} reopened; team {} is on it", p.name, t.name)
                        }
                        (None, Some(t)) => println!(
                            "{} reopened; it waits for team {} (busy on another project)",
                            p.name,
                            app.team(t)?.0.name
                        ),
                        (None, None) => println!("{} reopened (no team yet)", p.name),
                    }
                }
                ProjectCmd::Resume { project: name } => {
                    let p = project(&app, name)?;
                    app.set_project_status(
                        p.id,
                        agenium_store::core::projects::ProjectStatus::Active,
                    )?;
                    println!("{} goes on", p.name);
                }
                ProjectCmd::ReviewBeforeMerge {
                    project: name,
                    value,
                } => {
                    let p = project(&app, name)?;
                    app.set_project_setting(
                        p.id,
                        "review_before_merge",
                        serde_json::json!(value == "on"),
                    )?;
                    println!("review_before_merge = {value}");
                }
            }
        }
        _ => unreachable!("dispatched by main"),
    }
    Ok(())
}
