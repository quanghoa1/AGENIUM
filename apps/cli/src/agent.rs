//! Agent commands: create, configure, export and import agents. Split out of `main.rs`.

use crate::*;

pub(crate) async fn run(cli: &Cli, ac: &AgentCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match ac {
        AgentCmd::Solo {
            project: pname,
            name,
            provider,
            model,
            effort,
        } => {
            let p = project(&app, pname)?;
            if app.config().provider(provider).is_none() {
                bail!("provider {provider:?} is not configured (agenium provider add ...)");
            }
            let binding = ModelBinding {
                provider: provider.clone(),
                model: model.clone(),
                effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                max_tokens: None,
            };
            let (_, a) = app.create_solo(
                p.id,
                &CreateAgent {
                    name: name.clone(),
                    role: "solo".into(),
                    role_kind: RoleKind::Lead,
                    tags: vec![],
                    model: binding,
                },
            )?;
            println!("solo agent {} ({}) on {}", a.name, a.id, p.name);
        }
        AgentCmd::Create {
            name,
            role,
            provider,
            model,
            effort,
            cli: via_cli,
            tags,
            skills,
            mcp,
        } => {
            let a = app.create_agent(&agenium_orchestrator::registry::NewAgent {
                name: name.clone(),
                role: role.clone(),
                tags: (!tags.is_empty()).then(|| tags.clone()),
                model: ModelBinding {
                    provider: provider.clone(),
                    model: model.clone(),
                    effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                    max_tokens: None,
                },
                backend: if *via_cli {
                    agenium_core::BackendKind::Cli
                } else {
                    agenium_core::BackendKind::Api
                },
                skills: skills.clone(),
                mcp: mcp
                    .iter()
                    .map(|s| agenium_mcp::Binding {
                        server: s.clone(),
                        credentials: Default::default(),
                        keys: Default::default(),
                        tools: None,
                    })
                    .collect(),
            })?;
            println!(
                "agent {} ({}) created in the pool, role {}; next: agenium team compose ...",
                a.name, a.id, a.role
            );
        }
        AgentCmd::List => {
            for a in app.agents()? {
                println!(
                    "{}\t{}\t{}\t{}{}/{} ({})",
                    a.name,
                    a.id,
                    a.status,
                    if a.backend == BackendKind::Cli {
                        "cli:"
                    } else {
                        ""
                    },
                    a.model.provider,
                    a.model.model,
                    a.model.effort.as_str()
                );
            }
        }
        AgentCmd::Model {
            name,
            provider,
            model,
            effort,
        } => {
            let a = app
                .agents()?
                .into_iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("no agent {name}"))?;
            app.set_agent_backend(
                a.id,
                BackendKind::Api,
                &ModelBinding {
                    provider: provider.clone(),
                    model: model.clone(),
                    effort: effort.parse().map_err(|e: String| anyhow!(e))?,
                    max_tokens: None,
                },
            )?;
            println!("{name} now uses {provider}/{model}");
        }
        AgentCmd::Cli { name, cli, model } => {
            let a = app
                .agents()?
                .into_iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("no agent {name}"))?;
            app.set_agent_backend(
                a.id,
                BackendKind::Cli,
                &ModelBinding {
                    provider: cli.clone(),
                    model: model.clone(),
                    effort: a.model.effort,
                    max_tokens: None,
                },
            )?;
            println!("{name} now runs through the {cli} CLI");
            if cli == "claude" {
                eprintln!(
                    "warning: Claude Code runs on your own claude.ai login. Anthropic does not allow third-party apps \
                     to offer claude.ai login unless approved, and Agenium is not: this is your choice and your risk, \
                     account included. An Anthropic API key (an API provider) carries no such risk."
                );
            }
            let driven = agenium_cli_backend::CliKind::ALL
                .into_iter()
                .find(|k| k.program() == cli)
                .and_then(agenium_cli_backend::driver)
                .is_some();
            if !driven {
                println!("note: Agenium does not drive {cli} yet; its runs will fail (see `agenium doctor`)");
            }
        }
        AgentCmd::Skills { name, skills } => {
            let a = app.agent_named(name)?;
            app.set_agent_skills(a.id, skills)?;
            if skills.is_empty() {
                println!("{name} uses only the skills of role {} and its own", a.role);
            } else {
                println!("{name} enables the library skills: {}", skills.join(", "));
            }
        }
        AgentCmd::Mcp {
            name,
            server,
            creds,
            tools,
            remove,
        } => {
            let a = app.agent_named(name)?;
            let mut bindings = agenium_mcp::parse_bindings(&a.mcp)?;
            if let Some(r) = remove {
                bindings.retain(|b| &b.server != r);
            }
            if let Some(s) = server {
                bindings.retain(|b| &b.server != s);
                bindings.push(agenium_mcp::Binding {
                    server: s.clone(),
                    credentials: pairs(creds)?,
                    keys: Default::default(),
                    tools: tools.clone(),
                });
            }
            app.set_agent_mcp(a.id, &bindings)?;
            let list: Vec<&str> = bindings.iter().map(|b| b.server.as_str()).collect();
            println!(
                "{name} MCP servers: {}",
                if list.is_empty() {
                    "(none)".into()
                } else {
                    list.join(", ")
                }
            );
        }
        AgentCmd::Export {
            name,
            file,
            allow_secret_findings,
        } => {
            let a = app.agent_named(name)?;
            let s = app.export_agent(
                a.id,
                file,
                app.embedder().as_ref(),
                &agenium_orchestrator::agent_io::ExportOptions {
                    allow_secret_findings: *allow_secret_findings,
                },
            )?;
            println!(
                "exported {name} to {} ({} files, {} memory rows)",
                s.path.display(),
                s.files,
                s.memory_rows
            );
        }
        AgentCmd::Import {
            file,
            project: pname,
            name,
        } => {
            let p = project(&app, pname)?;
            let a = app.import_agent(
                file,
                &agenium_orchestrator::agent_io::ImportRequest {
                    project: p.id,
                    name: name.clone(),
                },
                app.embedder().as_ref(),
            )?;
            if app.config().provider(&a.model.provider).is_none() {
                println!(
                        "note: provider {:?} is not configured here; add it or run `agenium agent model {} ...`",
                        a.model.provider, a.name
                    );
            }
            println!(
                "imported {} ({}) as the Solo agent of {}",
                a.name, a.id, p.name
            );
        }
        AgentCmd::Delete {
            name,
            confirm,
            export,
        } => {
            let a = app.agent_named(name)?;
            if let Some(file) = export {
                app.export_agent(a.id, file, app.embedder().as_ref(), &Default::default())?;
                println!("exported {name} to {}", file.display());
            }
            let typed = match confirm {
                Some(c) => c.clone(),
                None if interactive() => {
                    eprintln!(
                            "This permanently deletes {name}'s memory, learned skills, SOUL.md history and folder."
                        );
                    let mut out = std::io::stderr();
                    let _ = write!(out, "Type the agent's name to confirm: ");
                    let _ = out.flush();
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line.trim().to_owned()
                }
                None => bail!("pass --confirm <name> to delete without a terminal"),
            };
            let r = app.delete_agent(a.id, &typed)?;
            println!("deleted {}", r.name);
            for p in r.pending {
                println!("still locked, removed at next start: {p}");
            }
        }
        AgentCmd::Resume { name } => {
            let a = app
                .agents()?
                .into_iter()
                .find(|a| &a.name == name)
                .ok_or_else(|| anyhow!("no agent {name}"))?;
            app.set_agent_status(a.id, AgentStatus::Idle)?;
            println!("{name} is Idle");
        }
    }
    Ok(())
}
