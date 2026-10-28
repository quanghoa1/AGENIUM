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
                    let p = app.create_project(&CreateProject {
                        name: name.clone(),
                        path: path.clone(),
                        key_prefix: prefix.clone(),
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
