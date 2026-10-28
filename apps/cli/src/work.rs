//! Working with a project: chat, runs, the board, the inbox, the plan, answers, permissions, approvals, events.

use super::*;

pub(crate) async fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Chat {
            project: pname,
            message,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let rt = runtime(app);
            let r = rt.chat(p.id, message).await?;
            println!();
            if r.outcome != "Completed" && !r.text.is_empty() {
                eprintln!("{}", r.text);
            }
            print_runs(&[r]);
        }
        Cmd::Run {
            project: pname,
            rounds,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let rt = runtime(app.clone());
            let cancel = rt.cancel.clone();
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    eprintln!("\nstopping after the current step (Ctrl+C)...");
                    cancel.cancel();
                }
            });
            let rs = rt.drive(p.id, *rounds).await?;
            print_runs(&rs);
            if rs.is_empty() {
                println!("nothing to run (no ready tickets or reviews)");
            }
            // What still waits for the user in this project (answered and other projects' items are not shown).
            for n in app.needs_you(p.id)? {
                let on = n.ticket.map(|t| format!("{t}: ")).unwrap_or_default();
                println!("needs you: {on}{}", n.text);
            }
        }
        Cmd::Board { project: pname } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            for col in app.board(p.id)? {
                if col.cards.is_empty() {
                    continue;
                }
                println!("== {} ({})", col.status, col.cards.len());
                for c in &col.cards {
                    let mut flags = vec![];
                    if !c.blocked_by.is_empty() {
                        flags.push(format!(
                            "blocked by {}",
                            c.blocked_by
                                .iter()
                                .map(|k| k.to_string())
                                .collect::<Vec<_>>()
                                .join(",")
                        ));
                    }
                    if c.open_questions > 0 {
                        flags.push(format!("{} open question(s)", c.open_questions));
                    }
                    for (on, name) in [
                        (c.approved, "approved"),
                        (c.merged, "merged"),
                        (c.escalated, "ESCALATED"),
                    ] {
                        if on {
                            flags.push(name.into());
                        }
                    }
                    if c.reopen_count > 0 {
                        flags.push(format!("reopened x{}", c.reopen_count));
                    }
                    println!(
                        "  {}  P{}  {}  {}  [{}]{}",
                        c.key,
                        c.priority,
                        c.plan_item,
                        c.title,
                        c.assignee.as_deref().unwrap_or("-"),
                        if flags.is_empty() {
                            String::new()
                        } else {
                            format!("  ({})", flags.join("; "))
                        }
                    );
                }
            }
        }
        Cmd::Inbox { project: pname } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let items = app.lead_inbox(p.id)?;
            if items.is_empty() {
                println!("The Lead's inbox is empty (Solo agents send their questions to you).");
            }
            for (i, it) in items.iter().enumerate() {
                println!("{}. [{:?}] {}", i + 1, it.kind, it.text);
            }
        }
        Cmd::Plan { project: pname } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let v = app.plan_view(p.id)?;
            let names: std::collections::HashMap<String, String> = app
                .agents()?
                .into_iter()
                .map(|a| (format!("agent:{}", a.id), a.name))
                .collect();
            let who = |l: &str| names.get(l).cloned().unwrap_or_else(|| l.to_owned());
            match &v.spec {
                Some(s) => {
                    println!(
                        "Spec v{} ({})",
                        s.version,
                        match &s.approved_by {
                            Some(by) => format!("approved by {}, locked", who(by)),
                            None => format!(
                                "waits for your approval: agenium approve --project {} spec {}",
                                p.name, s.version
                            ),
                        }
                    );
                    for g in &s.body.goals {
                        println!("  goal: {g}");
                    }
                    for r in &s.body.requirements {
                        println!("  {}: {}", r.id, r.text);
                    }
                }
                None => println!("No spec yet: tell the Lead what you need (agenium chat)."),
            }
            if let Some(pl) = &v.plan {
                let state = match (&pl.approved_by, v.current_plan_version) {
                    (Some(by), _) => format!("approved by {}", who(by)),
                    (None, Some(cur)) => format!(
                        "waits for your approval (v{cur} stays in force): agenium approve --project {} plan {}",
                        p.name, pl.version
                    ),
                    (None, None) => format!("waits for your approval: agenium approve --project {} plan {}", p.name, pl.version),
                };
                println!(
                    "Plan v{} for spec v{} ({state})",
                    pl.version, pl.spec_version
                );
                if let Some(r) = &pl.reason {
                    println!("  revision reason: {r}");
                }
                for i in &v.items {
                    println!(
                        "  [{}] {} {}  {}  {}",
                        if i.done { "x" } else { " " },
                        i.milestone,
                        i.id,
                        i.title,
                        i.tickets
                            .iter()
                            .map(|(k, s)| format!("{k}:{s}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                }
            }
        }
        Cmd::Answer {
            project: pname,
            ticket,
            question,
            answer,
            option,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            match app.route_user_answer(
                p.id,
                &ticket.parse().map_err(|e: IdParseError| anyhow!(e))?,
                question.parse().map_err(|e: IdParseError| anyhow!(e))?,
                answer,
                *option,
            )? {
                agenium_orchestrator::engine::UserAnswer::Answered(out) => {
                    println!("{}", out.result)
                }
                // In a team the user answers the Lead, who answers on the ticket (ADR-122).
                agenium_orchestrator::engine::UserAnswer::ToLead(message) => {
                    let rt = runtime(app);
                    let r = rt.chat(p.id, &message).await?;
                    println!();
                    print_runs(&[r]);
                }
            }
        }
        Cmd::Permission {
            project: pname,
            ticket,
            request,
            decision,
            always,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let d = if decision == "approve" {
                PermissionDecision::Approve
            } else {
                PermissionDecision::Deny
            };
            let out = app.decide_permission(
                p.id,
                &ticket.parse().map_err(|e: IdParseError| anyhow!(e))?,
                request.parse().map_err(|e: IdParseError| anyhow!(e))?,
                d,
                *always,
                None,
            )?;
            println!("{}", out.result);
        }
        Cmd::Approve {
            project: pname,
            what,
            target,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            match what.as_str() {
                "spec" => app.approve_spec(p.id, target.parse()?)?,
                "plan" => app.approve_plan(p.id, target.parse()?)?,
                "role" => {
                    let r = app.decide_role_proposal(p.id, target, true, None, None)?;
                    if let Some(a) = r.agent {
                        println!("{} joined the team", app.agent(a)?.name);
                    }
                }
                _ => {
                    app.user_approve_merge(
                        p.id,
                        &target.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    )?;
                }
            }
            println!("approved {what} {target}");
        }
        Cmd::Decline {
            project: pname,
            what: _,
            target,
            note,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            app.decide_role_proposal(p.id, target, false, None, note.as_deref())?;
            println!("declined role proposal {target}");
        }
        Cmd::Events { since, follow } => {
            let app = open(&root(cli)?)?;
            let mut sub = app.subscribe(*since);
            for e in sub.catch_up().map_err(|e| anyhow!("{e:?}"))? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    e.seq, e.at, e.actor, e.kind, e.payload
                );
            }
            if *follow {
                eprintln!("(following; Ctrl+C to stop)");
                while let Ok(e) = sub.recv().await {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        e.seq, e.at, e.actor, e.kind, e.payload
                    );
                }
            }
        }
        _ => unreachable!("dispatched by main"),
    }
    Ok(())
}
