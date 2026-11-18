//! Real check of the project map (ADR-166) with a real agent through its CLI: three kinds of work.
//! - code: a new project (a small Python tool in three modules) is built and mapped; a follow-up fix asks the map first.
//! - review: an existing code base is reviewed and its structure mapped.
//! - research: a tool's public face (its repository, releases, documentation) is mapped from pages Agenium reads itself.
//!
//! - trivial: a one-file script; the map must stay empty (no noise: the map is for code bases and systems).
//!
//! Each map entry must have passed Agenium's checks; after the tickets close nothing stays tentative or stale.
//! `unprompted`: the tickets say nothing of the map, so it shows whether agents map on their own what is worth it
//! (a code base, a system they research) and leave out what is not (a one-file script).
//!
//! cargo run -p agenium-eval --example map_real -- <claude|codex> [model] [code|review|research|trivial|all]
//!   [unprompted]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The tickets say nothing of the map (see the module docs).
static UNPROMPTED: AtomicBool = AtomicBool::new(false);

/// `text`, or nothing when the run is unprompted.
fn ask(text: &str) -> &str {
    if UNPROMPTED.load(Ordering::Relaxed) {
        ""
    } else {
        text
    }
}

use agenium_core::agent::{BackendKind, Effort, RoleKind};
use agenium_core::forms::*;
use agenium_core::plan::*;
use agenium_core::*;
use agenium_orchestrator::{App, CallCtx};

fn check(fails: &mut u32, name: &str, ok: bool, seen: &str) {
    if !ok {
        *fails += 1;
    }
    println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
}

struct Project {
    app: Arc<App>,
    id: ProjectId,
    solo: AgentId,
    root: std::path::PathBuf,
    n: usize,
}

impl Project {
    fn open(
        tmp: &Path,
        name: &str,
        prefix: &str,
        cli: &str,
        model: &str,
    ) -> Result<Project, Box<dyn std::error::Error>> {
        let (app, _) = App::open(
            &tmp.join("root"),
            agenium_orchestrator::AppOptions {
                keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
                checks: Arc::new(agenium_orchestrator::NoCheckFactory),
            },
        )?;
        let app = Arc::new(app);
        app.update_config(|c| c.memory.learn = false)?;
        let p = app.create_project(&agenium_orchestrator::CreateProject {
            name: name.into(),
            path: Some(tmp.join(name)),
            key_prefix: Some(prefix.into()),
            template: None,
            folder: None,
            repo: None,
        })?;
        let binding = ModelBinding {
            provider: cli.into(),
            model: model.into(),
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
        Ok(Project {
            app,
            id: p.id,
            solo: solo.id,
            root: std::path::PathBuf::from(&p.path),
            n: 0,
        })
    }

    fn commit(&self, files: &[(&str, &str)]) -> Result<(), Box<dyn std::error::Error>> {
        for (f, body) in files {
            let path = self.root.join(f);
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(path, body)?;
        }
        let p = self.app.project(self.id)?;
        self.app.repo(&p).commit_all(
            &self.root,
            "The code",
            ("eval", "eval@agenium.local"),
            &[],
        )?;
        Ok(())
    }

    /// The spec and the plan: one requirement and plan item per `(title, criterion)`.
    fn plan(&self, items: &[(&str, &str)]) -> Result<(), Box<dyn std::error::Error>> {
        let ac = |t: &str| {
            vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: t.into(),
            }]
        };
        let user = CallCtx::user(self.id);
        self.app.apply(
            &user,
            Intent::ProposeSpec(RequirementSpec {
                goals: items.iter().map(|(t, _)| (*t).to_owned()).collect(),
                scope: vec![],
                out_of_scope: vec![],
                requirements: items
                    .iter()
                    .enumerate()
                    .map(|(i, (t, a))| Requirement {
                        id: format!("REQ-{}", i + 1),
                        text: (*t).into(),
                        acceptance: ac(a),
                    })
                    .collect(),
                constraints: vec![],
            }),
        )?;
        self.app.apply(
            &user,
            Intent::CreatePlan(TicketPlan {
                spec_version: 1,
                milestones: vec![Milestone {
                    id: "M1".into(),
                    title: "Work".into(),
                }],
                items: items
                    .iter()
                    .enumerate()
                    .map(|(i, (t, a))| PlanItem {
                        id: format!("PI-{}", i + 1),
                        milestone: "M1".into(),
                        title: (*t).into(),
                        trace_to: vec![format!("REQ-{}", i + 1)],
                        acceptance: ac(a),
                        role: "solo".into(),
                        depends_on: vec![],
                        estimate: Estimate::S,
                        scope_paths: vec![],
                    })
                    .collect(),
            }),
        )?;
        Ok(())
    }

    /// The ticket of the next plan item, assigned to the Solo agent; its key.
    fn ticket(
        &mut self,
        title: &str,
        description: &str,
        ac: &str,
        after: Option<&TicketKey>,
    ) -> Result<TicketKey, Box<dyn std::error::Error>> {
        self.n += 1;
        let user = CallCtx::user(self.id);
        let out = self.app.apply(
            &user,
            Intent::CreateTicket(NewTicket {
                kind: TicketKind::Task,
                plan_item: format!("PI-{}", self.n),
                parent: None,
                title: title.into(),
                description: description.into(),
                acceptance: vec![AcceptanceCriterion {
                    id: "AC-1".into(),
                    text: ac.into(),
                }],
                priority: 2,
                depends_on: after.into_iter().cloned().collect(),
                required_role: None,
                required_tags: vec![],
                scope_paths: vec![],
                report_template: None,
                follows: None,
            }),
        )?;
        let key: TicketKey = serde_json::from_value(out.result["ticket_id"].clone())?;
        self.app.apply(
            &user,
            Intent::AssignTicket(AssignTicket {
                ticket_id: key.clone(),
                worker: self.solo,
                note: None,
            }),
        )?;
        Ok(key)
    }

    async fn drive(&self, key: &TicketKey) -> Result<TicketStatus, Box<dyn std::error::Error>> {
        let rt = agenium_orchestrator::runtime::Runtime::new(
            self.app.clone(),
            Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
            Arc::new(agenium_orchestrator::runtime::AutoApprove),
        );
        let started = std::time::Instant::now();
        rt.drive(self.id, 24).await?;
        self.app.map_settle(self.id)?;
        let t = self.app.ticket(self.id, key)?;
        println!(
            "  ticket {key}: {:?} in {:.0} s",
            t.status,
            started.elapsed().as_secs_f64()
        );
        Ok(t.status)
    }

    /// Every logged step of the ticket's runs, one per line.
    fn log(&self, key: &TicketKey) -> String {
        let Ok(t) = self.app.ticket(self.id, key) else {
            return String::new();
        };
        let mut s = String::new();
        for r in self
            .app
            .store()
            .read(|c| agenium_store::core::runs::for_ticket(c, t.id))
            .unwrap_or_default()
        {
            for e in self
                .app
                .store()
                .read(|c| agenium_store::core::runs::log_entries(c, r.id))
                .unwrap_or_default()
            {
                s.push_str(&e.kind);
                s.push(' ');
                s.push_str(&e.data.to_string());
                s.push('\n');
            }
        }
        s
    }

    fn dump(&self) -> agenium_orchestrator::map::MapGraph {
        let g = self.app.map_around(self.id, None, 1).unwrap_or_default();
        for n in &g.nodes {
            println!("    node {} [{}]", n.key, n.status);
        }
        for l in &g.links {
            println!(
                "    {} --{}--> {} [{}] ({})",
                l.src, l.kind, l.dst, l.evidence, l.status
            );
        }
        g
    }
}

fn map_calls(log: &str, tool: &str) -> usize {
    log.lines()
        .filter(|l| l.starts_with("tool ") && l.contains(&format!("{tool}\"")))
        .count()
}

const MAP_ASK: &str = "When it works, put what you built on the project map with map_add: each module (file:...) and \
its main functions (symbol:<file>#<name>), and the calls and imports between them, each link with the file and line \
of the source that shows it. Fix and add again anything the map refuses.";

async fn code(cli: &str, model: &str, fails: &mut u32) -> Result<(), Box<dyn std::error::Error>> {
    println!("== code: a new project, mapped, then a fix that asks the map");
    let tmp = tempfile::tempdir()?;
    let mut p = Project::open(tmp.path(), "wordcount", "WC", cli, model)?;
    p.plan(&[
        (
            "Build the wordcount tool",
            "`python -m wordcount.cli README.md` prints the number of words of README.md",
        ),
        (
            "count_words ignores punctuation",
            "count_words of \"one - two\" is 2",
        ),
    ])?;
    let k1 = p.ticket(
        "Build the wordcount tool",
        &format!(
            "Write a small Python 3 command-line tool in three modules: wordcount/reader.py with read_text(path) that \
             returns a file's text; wordcount/counter.py with count_words(path) that calls read_text and returns how \
             many words the text has; wordcount/cli.py with main(argv) that calls count_words on the file named in \
             argv and prints the number. Add tests/test_counter.py and run it with `python -m unittest`. {}",
            ask(MAP_ASK)
        ),
        "`python -m wordcount.cli README.md` prints the number of words of README.md",
        None,
    )?;
    // The fix waits for the build: one drive runs both, the fix after the build closed (and its map was confirmed).
    let k2 = p.ticket(
        "count_words ignores punctuation",
        &format!(
            "count_words counts \"one, two!\" as two words already, but a dash on its own (\"one - two\") counts as a \
             word: it must not. Find where words are counted{} and fix it, with a test.",
            ask(" (ask the project map first: map_query)")
        ),
        "count_words of \"one - two\" is 2",
        Some(&k1),
    )?;
    p.commit(&[("README.md", "One two three four five.\n")])?;
    let s1 = p.drive(&k1).await?;
    let g = p.dump();
    let code_links = g
        .links
        .iter()
        .filter(|l| !l.evidence.starts_with("http"))
        .count();
    let log1 = p.log(&k1);
    check(
        fails,
        "the build ticket closed",
        s1 == TicketStatus::Closed,
        &format!("{s1:?}"),
    );
    check(
        fails,
        "the map holds its modules and the calls between them, all checked",
        // Unprompted, the three modules and how they depend on each other are enough (functions are a plus).
        g.nodes.len()
            >= if UNPROMPTED.load(Ordering::Relaxed) {
                3
            } else {
                4
            }
            && code_links >= 2
            && g.links.iter().all(|l| l.status == "confirmed"),
        &format!(
            "{} nodes, {code_links} code links, map_add called {} times",
            g.nodes.len(),
            map_calls(&log1, "map_add")
        ),
    );
    let s2 = p.app.ticket(p.id, &k2)?.status;
    let log2 = p.log(&k2);
    check(
        fails,
        "the fix ticket closed",
        s2 == TicketStatus::Closed,
        &format!("{s2:?}"),
    );
    // The map reaches the fix two ways: in its first message (the links of the nodes the ticket names) or by asking.
    let t2 = p.app.ticket(p.id, &k2)?;
    let given = p.app.map_near(p.id, &t2);
    let tools: std::collections::BTreeSet<String> = log2
        .lines()
        .filter(|l| l.starts_with("tool "))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[5..]).ok())
        .filter_map(|v| v["tool"].as_str().map(str::to_owned))
        .collect();
    println!(
        "  the fix's first message carried the map:\n{}",
        given
            .lines()
            .map(|l| format!("    {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    check(
        fails,
        "the fix had the map before changing code (in its work order, or asked)",
        given.contains("count_words")
            || (UNPROMPTED.load(Ordering::Relaxed) && given.contains("-->"))
            || map_calls(&log2, "map_query") >= 1,
        &format!(
            "map_query called {} times; tools used: {}",
            map_calls(&log2, "map_query"),
            tools.into_iter().collect::<Vec<_>>().join(", ")
        ),
    );
    let st = p.app.map_stats(p.id)?;
    check(
        fails,
        "nothing on the map is tentative or stale at the end",
        st.tentative == 0 && st.stale == 0,
        &format!("{st:?}"),
    );
    Ok(())
}

async fn review(cli: &str, model: &str, fails: &mut u32) -> Result<(), Box<dyn std::error::Error>> {
    println!("== review: an existing code base, mapped while it is reviewed");
    let tmp = tempfile::tempdir()?;
    let mut p = Project::open(tmp.path(), "shop", "SH", cli, model)?;
    p.plan(&[(
        "Review the shop code",
        "deliverables/review.md names the SQL injection in shop/db.py with its line",
    )])?;
    p.commit(&[
        ("shop/__init__.py", ""),
        ("shop/db.py", "import sqlite3\n\n\ndef connect(path):\n    return sqlite3.connect(path)\n\n\ndef find_order(conn, order_id):\n    return conn.execute(\"SELECT * FROM orders WHERE id = \" + str(order_id)).fetchone()\n"),
        ("shop/orders.py", "from shop.db import connect, find_order\n\n\ndef order_total(path, order_id):\n    conn = connect(path)\n    row = find_order(conn, order_id)\n    return row[2] * row[3]\n"),
        ("shop/api.py", "from shop.orders import order_total\n\n\ndef handle(request):\n    return {\"total\": order_total(\"shop.db\", request[\"id\"])}\n"),
    ])?;
    let k = p.ticket(
        "Review the shop code",
        &format!(
            "Review the source of this project for bugs and security risks. Write deliverables/review.md with each \
             finding, its file and line, and why it matters.{}",
            ask(
                " Put the code's structure on the project map as you trace it: the modules, their functions, and how a \
                 request flows from shop/api.py down to the database. Use map_add with the file and line of each call \
                 or import; fix and add again anything the map refuses."
            )
        ),
        "deliverables/review.md names the SQL injection in shop/db.py with its line",
        None,
    )?;
    let s = p.drive(&k).await?;
    let g = p.dump();
    let review = std::fs::read_to_string(p.root.join("deliverables/review.md")).unwrap_or_default();
    check(
        fails,
        "the review ticket closed",
        s == TicketStatus::Closed,
        &format!("{s:?}"),
    );
    check(
        fails,
        "the review names the SQL injection",
        review.to_lowercase().contains("injection"),
        &review.chars().take(160).collect::<String>(),
    );
    // Function by function when asked; unprompted, the modules' chain is enough.
    let path = match p.app.map_path(
        p.id,
        "symbol:shop/api.py#handle",
        "symbol:shop/db.py#find_order",
    )? {
        Some(x) => Some(x),
        None if UNPROMPTED.load(Ordering::Relaxed) => {
            p.app
                .map_path(p.id, "file:shop/api.py", "file:shop/db.py")?
        }
        None => None,
    };
    check(
        fails,
        "the map holds the request's chain from the API to the database",
        path.is_some(),
        &format!("{} nodes, {} links", g.nodes.len(), g.links.len()),
    );
    let st = p.app.map_stats(p.id)?;
    check(
        fails,
        "all of it checked and confirmed",
        st.tentative == 0 && st.stale == 0 && st.links >= 2,
        &format!("{st:?}"),
    );
    Ok(())
}

async fn research(
    cli: &str,
    model: &str,
    fails: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("== research: a tool's public face, mapped from pages");
    let tmp = tempfile::tempdir()?;
    let mut p = Project::open(tmp.path(), "research", "RS", cli, model)?;
    p.plan(&[(
        "Map ripgrep's public face",
        "deliverables/ripgrep.md names ripgrep's repository, its releases and its guide, each with a URL",
    )])?;
    let k = p.ticket(
        "Map ripgrep's public face",
        &format!(
            "Find out where ripgrep (BurntSushi/ripgrep) lives on the web: its source repository, where its releases \
             are downloaded from, and where its user guide is. Write deliverables/ripgrep.md with what you found and \
             the pages you read.{}",
            ask(
                " Put it on the project map with map_add: product:ripgrep and the places as nodes (repository, \
                 domain:..., the guide), joined by links such as hosted_at, releases_at, documented_at, each with the \
                 {\"url\"} of a page you read that names both ends. Fix and add again anything the map refuses."
            )
        ),
        "deliverables/ripgrep.md names ripgrep's repository, its releases and its guide, each with a URL",
        None,
    )?;
    let s = p.drive(&k).await?;
    // The run's tools in order, a refusal with its code: whether the map's reminder came and what the agent did.
    for l in p.log(&k).lines().filter(|l| l.starts_with("tool ")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&l[5..]) {
            let res = v["result"].as_str().unwrap_or_default();
            println!(
                "    {} ok={} {}",
                v["tool"].as_str().unwrap_or_default(),
                v["ok"],
                if v["ok"] == false {
                    res.chars().take(120).collect::<String>()
                } else {
                    String::new()
                }
            );
        }
    }
    let g = p.dump();
    let web = g
        .links
        .iter()
        .filter(|l| l.evidence.starts_with("http"))
        .count();
    check(
        fails,
        "the research ticket closed",
        s == TicketStatus::Closed,
        &format!("{s:?}"),
    );
    check(
        fails,
        "the map holds ripgrep's places, each link backed by a page Agenium read",
        web >= 2
            && g.nodes
                .iter()
                .any(|n| n.name.to_lowercase().contains("ripgrep")),
        &format!("{} nodes, {web} links from pages", g.nodes.len()),
    );
    let st = p.app.map_stats(p.id)?;
    check(
        fails,
        "all of it confirmed",
        st.tentative == 0 && st.stale == 0,
        &format!("{st:?}"),
    );
    Ok(())
}

/// A one-file script: nothing in it is worth mapping, so the map stays empty.
async fn trivial(
    cli: &str,
    model: &str,
    fails: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("== trivial: a one-file script, nothing to map");
    let tmp = tempfile::tempdir()?;
    let mut p = Project::open(tmp.path(), "hello", "HI", cli, model)?;
    p.plan(&[(
        "Greeting script",
        "`python hello.py Ann` prints Hello, Ann!",
    )])?;
    let k = p.ticket(
        "Greeting script",
        "Write hello.py: it prints \"Hello, <name>!\" for the name given as its argument. Run it once to check.",
        "`python hello.py Ann` prints Hello, Ann!",
        None,
    )?;
    let s = p.drive(&k).await?;
    let g = p.dump();
    check(
        fails,
        "the script's ticket closed",
        s == TicketStatus::Closed,
        &format!("{s:?}"),
    );
    check(
        fails,
        "the map stays empty for a one-file script",
        g.nodes.is_empty() && g.links.is_empty(),
        &format!("{} nodes, {} links", g.nodes.len(), g.links.len()),
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = args.first().cloned().unwrap_or_else(|| "claude".into());
    let model = args.get(1).cloned().unwrap_or_else(|| {
        if cli == "claude" {
            "haiku"
        } else {
            "gpt-6-luna"
        }
        .into()
    });
    let which = args.get(2).cloned().unwrap_or_else(|| "all".into());
    UNPROMPTED.store(args.iter().any(|a| a == "unprompted"), Ordering::Relaxed);
    if UNPROMPTED.load(Ordering::Relaxed) {
        println!("(unprompted: the tickets say nothing of the map)");
    }
    let mut fails = 0;
    if matches!(which.as_str(), "code" | "all") {
        code(&cli, &model, &mut fails).await?;
    }
    if matches!(which.as_str(), "review" | "all") {
        review(&cli, &model, &mut fails).await?;
    }
    if matches!(which.as_str(), "research" | "all") {
        research(&cli, &model, &mut fails).await?;
    }
    if matches!(which.as_str(), "trivial" | "all") {
        trivial(&cli, &model, &mut fails).await?;
    }
    println!(
        "{}",
        if fails == 0 {
            "ALL PASS"
        } else {
            "SOME FAILED"
        }
    );
    std::process::exit(i32::from(fails > 0));
}
