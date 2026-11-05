//! Tickets: their detail, comments, forms, changes.

use super::*;

#[tauri::command]
pub fn board(d: State<'_, Desktop>, project: String) -> R<Value> {
    Ok(json!(d.opened()?.app.board(pid(&project)?).map_err(err)?))
}

#[tauri::command]
pub fn plan(d: State<'_, Desktop>, project: String) -> R<Value> {
    Ok(json!(d
        .opened()?
        .app
        .plan_view(pid(&project)?)
        .map_err(err)?))
}

#[tauri::command]
pub fn ticket(d: State<'_, Desktop>, project: String, key: String) -> R<Value> {
    let o = d.opened()?;
    let p = pid(&project)?;
    let k = self::key(&key)?;
    let t = o.app.ticket(p, &k).map_err(err)?;
    let names = names(&o)?;
    let cs = o
        .app
        .store()
        .read(|c| comments::for_ticket(c, t.id))
        .map_err(err)?;
    let rs: Vec<Value> = o
        .app
        .store()
        .read(|c| runs::for_ticket(c, t.id))
        .map_err(err)?
        .iter()
        .map(|r| run_json(r, &names))
        .collect();
    let ws = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?;
    let author = |a: &Author| match a {
        Author::Agent(id) => names.get(id).cloned().unwrap_or_else(|| id.to_string()),
        other => other.to_string(),
    };
    let comments: Vec<Value> = cs
        .iter()
        .map(|c| {
            json!({ "id": c.id, "kind": c.kind, "author": author(&c.author), "reply_to": c.reply_to,
                          "body": c.body, "resolved": c.resolved, "created_at": c.created_at })
        })
        .collect();
    // Links to other tickets (related, aggregates...), each with the other ticket's key and title and its direction.
    let all = o.app.tickets(p).map_err(err)?;
    // The tickets that wait for this one (their depends_on names it): the other side of "depends on".
    let dependents: Vec<&TicketKey> = all
        .iter()
        .filter(|x| x.depends_on.contains(&t.key))
        .map(|x| &x.key)
        .collect();
    let dependents = json!(dependents);
    let others: HashMap<_, _> = all.into_iter().map(|x| (x.id, (x.key, x.title))).collect();
    let links: Vec<Value> = o
        .app
        .store()
        .read(|c| tickets::links_of(c, t.id))
        .map_err(err)?
        .into_iter()
        .filter_map(|l| {
            let out = l.src == t.id;
            let (key, title) = others.get(if out { &l.dst } else { &l.src })?;
            Some(json!({ "kind": l.kind, "out": out, "key": key, "title": title,
                         "proposed_by": l.proposed_by, "confirmed": l.confirmed_by.is_some(), "note": l.note }))
        })
        .collect();
    // ADR-147: a cancelled ticket whose work nothing carries on waits for the user's waiver (or a replacement).
    let waivable = t.status == agenium_core::TicketStatus::Cancelled
        && o.app
            .store()
            .read(|c| agenium_orchestrator::coverage::accounted(c, &t))
            .map_err(err)?
            == agenium_orchestrator::coverage::Accounted::No;
    Ok(json!({
        "ticket": t, "assignee_name": t.assignee.and_then(|a| names.get(&a).cloned()),
        "comments": comments, "runs": rs, "workspace": ws, "links": links, "dependents": dependents,
        "waivable": waivable,
        "audit": o.app.audit(p, Some(&k)).map_err(err)?,
    }))
}

/// The ticket's changes: the live branch against its base, or what was merged (base..merged commit).
#[tauri::command]
pub async fn ticket_diff(d: State<'_, Desktop>, project: String, key: String) -> R<Value> {
    let o = d.opened()?;
    // git diff of a whole branch: off the window's thread (audit H4).
    super::actions::blocking(move || ticket_diff_now(&o, &project, &key)).await
}

fn ticket_diff_now(o: &crate::state::Opened, project: &str, key: &str) -> R<Value> {
    let p = o.app.project(pid(project)?).map_err(err)?;
    let t = o.app.ticket(p.id, &self::key(key)?).map_err(err)?;
    let Some(ws) = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?
    else {
        return Ok(json!({ "diff": "", "note": "no worktree yet" }));
    };
    let repo = o.app.repo(&p);
    let diff = if !ws.removed {
        repo.diff(std::path::Path::new(&ws.worktree), &ws.base_sha)
            .map_err(err)?
    } else if let Some(m) = &ws.merged_sha {
        repo.git()
            .check(
                std::path::Path::new(&p.path),
                [
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    &format!("{}..{m}", ws.base_sha),
                ],
            )
            .map_err(err)?
    } else {
        String::new()
    };
    const MAX: usize = 400_000;
    let cut = diff.len() > MAX;
    let diff: String = if cut {
        diff.chars().take(MAX).collect()
    } else {
        diff
    };
    Ok(json!({ "diff": diff, "truncated": cut, "branch": ws.branch, "merged": ws.merged_sha }))
}

/// One changed file of a ticket, before and after (for a side-by-side diff): the content at the ticket's base, and
/// the worktree's file (or, once merged, the merge's). `null` = the file does not exist on that side.
/// A screenshot of a ticket's report (ADR-158), by its file name: its media type and bytes (base64).
#[tauri::command]
pub fn evidence_image(
    d: State<'_, Desktop>,
    project: String,
    key: String,
    image: String,
) -> R<Value> {
    use base64::Engine as _;
    let o = d.opened()?;
    let p = o.app.project(pid(&project)?).map_err(err)?;
    let (media_type, bytes) =
        agenium_orchestrator::evidence::read(std::path::Path::new(&p.path), &key, &image)
            .ok_or_else(|| format!("{key} has no screenshot {image}"))?;
    Ok(
        json!({ "media_type": media_type, "base64": base64::engine::general_purpose::STANDARD.encode(bytes) }),
    )
}

#[tauri::command]
pub async fn ticket_file(
    d: State<'_, Desktop>,
    project: String,
    key: String,
    path: String,
) -> R<Value> {
    let o = d.opened()?;
    // git show and file reads: off the window's thread (audit H4).
    super::actions::blocking(move || ticket_file_now(&o, &project, &key, &path)).await
}

fn ticket_file_now(o: &crate::state::Opened, project: &str, key: &str, path: &str) -> R<Value> {
    const MAX: usize = 2_000_000;
    let p = o.app.project(pid(project)?).map_err(err)?;
    let t = o.app.ticket(p.id, &self::key(key)?).map_err(err)?;
    let rel = std::path::Path::new(&path);
    if path.is_empty()
        || rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!("{path:?} is not a path inside the project"));
    }
    let ws = o
        .app
        .store()
        .read(|c| workspaces::get(c, t.id))
        .map_err(err)?
        .ok_or("no worktree yet")?;
    let repo = o.app.repo(&p);
    let root = std::path::Path::new(&p.path);
    let at = |rev: &str| -> Option<String> {
        repo.git()
            .check(
                root,
                ["show", &format!("{rev}:{}", path.replace('\\', "/"))],
            )
            .ok()
    };
    let original = at(&ws.base_sha);
    let modified = if !ws.removed {
        std::fs::read(std::path::Path::new(&ws.worktree).join(rel))
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    } else {
        ws.merged_sha.as_deref().and_then(at)
    };
    let binary = [&original, &modified]
        .iter()
        .any(|s| s.as_deref().is_some_and(|s| s.contains('\0')));
    let big = [&original, &modified]
        .iter()
        .any(|s| s.as_ref().is_some_and(|s| s.len() > MAX));
    if binary || big {
        return Ok(json!({ "original": null, "modified": null, "binary": binary, "too_big": big }));
    }
    Ok(json!({ "original": original, "modified": modified, "binary": false, "too_big": false }))
}

#[tauri::command]
pub fn audit(
    d: State<'_, Desktop>,
    project: String,
    key: Option<String>,
    format: Option<String>,
) -> R<Value> {
    let o = d.opened()?;
    let k = key.as_deref().map(self::key).transpose()?;
    let entries = o.app.audit(pid(&project)?, k.as_ref()).map_err(err)?;
    Ok(match format.as_deref() {
        Some("md") => json!(agenium_orchestrator::views::audit_markdown(
            &k.map(|k| k.to_string()).unwrap_or_else(|| project.clone()),
            &entries
        )),
        _ => json!(entries),
    })
}
