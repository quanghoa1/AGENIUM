//! Ticket commands. Split out of `main.rs`.

use crate::*;

pub(crate) async fn run(cli: &Cli, tc: &TicketCmd) -> Result<()> {
    let app = open(&root(cli)?)?;
    match tc {
        TicketCmd::List { project: pname } => {
            let p = project(&app, pname)?;
            for t in app.tickets(p.id)? {
                println!(
                    "{}\t{}\t{}{}{}\t{}",
                    t.key,
                    t.status,
                    if t.approved { "approved " } else { "" },
                    if t.merged { "merged " } else { "" },
                    t.plan_item,
                    t.title
                );
            }
        }
        TicketCmd::Show {
            project: pname,
            key,
        } => {
            let p = project(&app, pname)?;
            let t = app.ticket(p.id, &key.parse().map_err(|e: IdParseError| anyhow!(e))?)?;
            println!("{}", agenium_backend_text(&t));
            for c in app.store().read(|c| comments::for_ticket(c, t.id))? {
                println!(
                    "[{}] {} {}: {}",
                    c.created_at,
                    c.kind,
                    c.id,
                    serde_json::to_string(&c.body)?
                        .chars()
                        .take(400)
                        .collect::<String>()
                );
            }
            for r in app.store().read(|c| runs::for_ticket(c, t.id))? {
                println!(
                    "run {} {} {} steps={} tokens={}/{}",
                    r.id,
                    r.started_at,
                    r.outcome.unwrap_or_else(|| "live".into()),
                    r.steps,
                    r.usage.tokens_in,
                    r.usage.tokens_out
                );
            }
        }
        TicketCmd::Audit {
            project: pname,
            key,
            format,
            out,
        } => {
            let p = project(&app, pname)?;
            let k: Option<TicketKey> = key
                .as_deref()
                .map(|k| k.parse().map_err(|e: IdParseError| anyhow!(e)))
                .transpose()?;
            let entries = app.audit(p.id, k.as_ref())?;
            let text = match format.as_str() {
                "md" => agenium_orchestrator::views::audit_markdown(
                    &k.as_ref().map(|k| k.to_string()).unwrap_or(p.name.clone()),
                    &entries,
                ),
                "jsonl" => entries
                    .iter()
                    .map(|e| serde_json::to_string(e).map(|l| l + "\n"))
                    .collect::<Result<String, _>>()?,
                _ => entries
                    .iter()
                    .map(|e| {
                        format!(
                            "{}\t{}\t{}\t{}\t{}\t{}\n",
                            e.at,
                            e.actor,
                            e.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
                            e.action,
                            if e.accepted { "applied" } else { "REFUSED" },
                            e.detail.to_string().chars().take(200).collect::<String>()
                        )
                    })
                    .collect(),
            };
            match out {
                Some(path) => {
                    std::fs::write(path, text)?;
                    println!("{} entries written to {}", entries.len(), path.display());
                }
                None => print!("{text}"),
            }
        }
        TicketCmd::Priority {
            project: pname,
            key,
            priority,
        } => {
            let p = project(&app, pname)?;
            let t = app.user_set_priority(
                p.id,
                &key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                *priority,
            )?;
            println!("{} is P{}", t.key, t.priority);
        }
        TicketCmd::Assign {
            project: pname,
            key,
            agent,
            note,
        } => {
            let p = project(&app, pname)?;
            let a = app
                .agents()?
                .into_iter()
                .find(|a| &a.name == agent)
                .ok_or_else(|| anyhow!("no agent named {agent}"))?;
            user_intent(
                &app,
                p.id,
                Intent::AssignTicket(forms::AssignTicket {
                    ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    worker: a.id,
                    note: note.clone(),
                }),
            )?;
        }
        TicketCmd::Review {
            project: pname,
            key,
            decision,
            feedback,
        } => {
            let p = project(&app, pname)?;
            let ac_feedback = feedback
                .iter()
                .map(|f| {
                    let (ac, text) = f
                        .split_once('=')
                        .ok_or_else(|| anyhow!("--feedback takes AC-n=\"text\""))?;
                    Ok(forms::AcFeedback {
                        ac_id: ac.trim().to_owned(),
                        feedback: text.trim().to_owned(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            user_intent(
                &app,
                p.id,
                Intent::ReviewTicket(forms::ReviewDecision {
                    ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    decision: if decision == "approve" {
                        forms::ReviewVerdict::Approve
                    } else {
                        forms::ReviewVerdict::Reopen
                    },
                    ac_feedback,
                    required_changes: vec![],
                }),
            )?;
        }
        TicketCmd::Close {
            project: pname,
            key,
            reason,
        } => {
            let p = project(&app, pname)?;
            user_intent(
                &app,
                p.id,
                Intent::CloseTicket(forms::CloseTicket {
                    ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    reason: reason.clone(),
                }),
            )?;
        }
        TicketCmd::Cancel {
            project: pname,
            key,
            reason,
        } => {
            let p = project(&app, pname)?;
            user_intent(
                &app,
                p.id,
                Intent::CancelTicket(forms::CancelTicket {
                    ticket_id: key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                    reason: reason.clone(),
                    replaced_by: None,
                }),
            )?;
        }
        TicketCmd::Waive {
            project: pname,
            key,
            note,
        } => {
            let p = project(&app, pname)?;
            app.waive_cancelled(
                p.id,
                &key.parse().map_err(|e: IdParseError| anyhow!(e))?,
                note.as_deref(),
            )?;
            println!("{key} waived: its plan item no longer waits for it");
        }
    }
    Ok(())
}
