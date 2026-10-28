//! Team commands. Split out of `main.rs`.

use crate::*;

pub(crate) async fn run(cli: &Cli, tc: &TeamCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match tc {
        TeamCmd::Templates => {
            for t in app.team_templates()? {
                let roles: Vec<String> = t
                    .roles
                    .iter()
                    .map(|r| {
                        if r.count > 1 {
                            format!("{} x{}", r.id, r.count)
                        } else {
                            r.id.clone()
                        }
                    })
                    .collect();
                println!(
                    "{}@{}	{}	[{}]	{}",
                    t.id,
                    t.version,
                    t.name,
                    roles.join(", "),
                    t.description
                );
            }
        }
        TeamCmd::Template { id } => print!("{}", app.team_template(id)?.to_toml()),
        TeamCmd::Import { file } => {
            let t = app.import_team_template(file)?;
            println!("team template {}@{} stored", t.id, t.version);
        }
        TeamCmd::Create {
            project: pname,
            template,
            model,
            lead_model,
            planner_model,
            effort,
            prefix,
        } => {
            let p = project(&app, pname)?;
            let binding = |pm: &str| -> Result<ModelBinding> {
                let (provider, model) = pm
                    .split_once('/')
                    .ok_or_else(|| anyhow!("{pm:?}: use provider/model"))?;
                if app.config().provider(provider).is_none() {
                    bail!("provider {provider:?} is not configured (agenium provider add ...)");
                }
                Ok(ModelBinding {
                    provider: provider.into(),
                    model: model.into(),
                    effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                    max_tokens: None,
                })
            };
            let m = binding(model)?;
            let lm = lead_model.as_deref().map(binding).transpose()?;
            let pm = planner_model.as_deref().map(binding).transpose()?;
            let out = app.create_team_from_template(
                p.id,
                template,
                &m,
                lm.as_ref(),
                pm.as_ref(),
                prefix.as_deref().unwrap_or(""),
            )?;
            println!("team created on {} from {template}:", p.name);
            for (name, role) in &out.agents {
                println!("  {name}	{role}");
            }
            for s in &out.missing_skills {
                println!("  note: skill not installed yet: {s}");
            }
            println!("Next: agenium chat --project {} \"<your request>\" (the Lead plans; you approve spec and plan)", p.name);
        }
        TeamCmd::Lead {
            project: pname,
            agent,
        } => {
            let p = project(&app, pname)?;
            let a = app
                .agents()?
                .into_iter()
                .find(|a| &a.name == agent)
                .ok_or_else(|| anyhow!("no agent named {agent}"))?;
            app.set_team_lead(p.id, a.id)?;
            println!("{} is the Lead of {} now", a.name, p.name);
        }
        TeamCmd::List => {
            for (t, members) in app.teams()? {
                let project = match t.project {
                    Some(p) => app.project(p)?.name,
                    None => "-".into(),
                };
                println!(
                    "{}{}\tproject {project}",
                    t.name,
                    if t.solo { " (solo)" } else { "" }
                );
                for a in members {
                    let lead = if t.lead == Some(a.id) { " (Lead)" } else { "" };
                    println!(
                        "  {}{lead}\t{}\t{}/{}",
                        a.name, a.role, a.model.provider, a.model.model
                    );
                }
            }
            let pool: Vec<_> = app
                .agents()?
                .into_iter()
                .filter(|a| a.team.is_none())
                .collect();
            println!(
                "pool: {}",
                if pool.is_empty() {
                    "-".to_owned()
                } else {
                    pool.iter()
                        .map(|a| format!("{} ({})", a.name, a.role))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            );
        }
        TeamCmd::Compose {
            name,
            lead,
            members,
            project: pname,
            solo,
        } => {
            let members: Vec<_> = members
                .iter()
                .map(|m| app.agent_named(m).map(|a| a.id))
                .collect::<std::result::Result<_, _>>()?;
            let lead = match lead {
                Some(l) => app.agent_named(l)?.id,
                None => {
                    let s = app
                        .suggest_lead(&members)?
                        .ok_or_else(|| anyhow!("no agent stands out as Lead: pass --lead"))?;
                    println!(
                        "suggested Lead: {} ({})",
                        app.agent(s.agent)?.name,
                        s.reasons.join(", ")
                    );
                    s.agent
                }
            };
            let members: Vec<_> = members.into_iter().filter(|m| *m != lead).collect();
            let project = pname.as_deref().map(|n| project(&app, n)).transpose()?;
            let t = app.compose_team(&agenium_orchestrator::registry::NewTeam {
                name: name.clone(),
                project: project.map(|p| p.id),
                lead,
                members,
                solo: *solo,
            })?;
            println!("team {} composed ({})", t.name, t.id);
        }
        TeamCmd::Add { team, agent } => {
            let t = team_named(&app, team)?;
            app.add_team_member(t, app.agent_named(agent)?.id)?;
            println!("{agent} joined {team}");
        }
        TeamCmd::Remove { team, agent } => {
            let t = team_named(&app, team)?;
            app.remove_team_member(t, app.agent_named(agent)?.id)?;
            println!("{agent} is back in the pool");
        }
        TeamCmd::Assign {
            team,
            project: pname,
            none: _,
        } => {
            let t = team_named(&app, team)?;
            let p = pname.as_deref().map(|n| project(&app, n)).transpose()?;
            app.set_team_project(t, p.as_ref().map(|p| p.id))?;
            println!(
                "{team} now works on {}",
                p.map(|p| p.name).unwrap_or_else(|| "no project".into())
            );
        }
        TeamCmd::Dissolve { team } => {
            let t = team_named(&app, team)?;
            app.dissolve_team(t)?;
            println!("{team} dissolved; its members are in the pool");
        }
        TeamCmd::Show { project: pname } => {
            let p = project(&app, pname)?;
            let (team, members) = app
                .team_of_project(p.id)?
                .ok_or_else(|| anyhow!("{} has no team", p.name))?;
            println!(
                "{} ({}{})",
                team.name,
                if team.solo { "solo, " } else { "" },
                team.template
            );
            for a in members {
                println!(
                    "  {}{}	{}	[{}]	{}	{}/{}",
                    a.name,
                    if team.lead == Some(a.id) {
                        " (Lead)"
                    } else if app.is_planner_role(&a.role) {
                        " (planner)"
                    } else {
                        ""
                    },
                    a.role,
                    a.tags.join(","),
                    a.status,
                    a.model.provider,
                    a.model.model
                );
            }
        }
    }
    Ok(())
}
