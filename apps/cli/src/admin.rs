//! Setting up and looking after a root: setup, support bundle, report, doctor, secrets, backup and restore.

use super::*;

pub(crate) async fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Setup { ui_language } => {
            let default_root = dirs_home().join(".agenium");
            // --root is the root chosen in the wizard (remembered in the bootstrap file); AGENIUM_HOME is a
            // per-process override: set up there, but remember nothing machine-wide.
            let env_root = std::env::var_os(agenium_config::root::ENV_HOME)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from);
            let remember = cli.root.is_some() || env_root.is_none();
            let input = agenium_config::setup::SetupInput {
                root: cli.root.clone().or(env_root),
                ui_language: ui_language.clone(),
            };
            let bootstrap = agenium_config::Bootstrap::default_path().filter(|_| remember);
            let out =
                agenium_config::setup::complete_setup(&input, &default_root, bootstrap.as_deref())?;
            println!("Agenium root: {}", out.root.display());
            for c in out.report.warnings() {
                println!("  warning: {} = {} ({})", c.name, c.value, c.message);
            }
            println!(
                "Suggested parallel agents: {}",
                out.report.suggested_parallel
            );
            // The embedding model is required (ADR-47): memory recalls and learns by meaning with it.
            use agenium_memory::local::{self, BGE_SMALL_EN as SPEC};
            let models = agenium_config::RootLayout::new(&out.root).models();
            if !local::installed(&SPEC, &models) {
                println!(
                    "Installing the embedding model ({} MB, once)",
                    SPEC.total_bytes() >> 20
                );
                let http = agenium_llm::http::Http::default_client()?;
                local::install(&SPEC, &models, &http, &|m| println!("  {m}"))
                    .await
                    .map_err(|e| anyhow!("{e}; run `agenium models download` to try again"))?;
            }
        }
        Cmd::Support { out } => {
            let app = open(&root(cli)?)?;
            let dir = app.support_bundle(out)?;
            println!("support bundle written to {}", dir.display());
        }
        Cmd::Report {
            project: pname,
            out,
        } => {
            let app = open(&root(cli)?)?;
            let p = project(&app, pname)?;
            let md = app.project_report_md(p.id)?;
            match out {
                Some(f) => {
                    std::fs::write(f, &md)?;
                    println!("report written to {}", f.display());
                }
                None => println!("{md}"),
            }
        }
        Cmd::Doctor => {
            let r = root(cli)?;
            let rep = agenium_config::sysreq::check(&r);
            println!("root: {}\nos: {}", r.display(), rep.os);
            for c in &rep.checks {
                println!("  [{:?}] {} = {} ({})", c.level, c.name, c.value, c.message);
            }
            let sb = agenium_sandbox::detect();
            println!("sandbox: {:?} - {}", sb.backend, sb.reason);
            if sb.backend.is_some() {
                println!("sandbox network: {}", agenium_sandbox::network_report());
            }
            println!("shell: {}", agenium_sandbox::Shell::detect().describe());
            println!("agent CLIs (CliBackend, M2.5; Agenium never logs in for you):");
            for c in agenium_cli_backend::detect_all() {
                let login = match c.login {
                    agenium_cli_backend::Login::LoggedIn => "logged in",
                    agenium_cli_backend::Login::NotLoggedIn => "not logged in",
                    agenium_cli_backend::Login::Unknown => "login unknown",
                };
                let driven = if agenium_cli_backend::driver(c.kind).is_some() {
                    ", usable"
                } else {
                    ", not driven yet"
                };
                match &c.version {
                    Some(v) => println!(
                        "  {:<9} {v:<12} {login}{driven}{}",
                        c.kind.program(),
                        c.note
                            .as_deref()
                            .map(|n| format!(" ({n})"))
                            .unwrap_or_default()
                    ),
                    None => println!(
                        "  {:<9} {}",
                        c.kind.program(),
                        c.note.as_deref().unwrap_or("-")
                    ),
                }
            }
            let models = agenium_config::RootLayout::new(&r).models();
            if agenium_memory::local::installed(&agenium_memory::local::BGE_SMALL_EN, &models) {
                println!("embedding model: bge-small-en-v1.5 (local)");
            } else {
                println!("embedding model: not installed, memory uses keyword-level hashing (agenium models download)");
            }
            match agenium_secrets::OsKeyring::status() {
                Ok(()) => println!("keyring: available"),
                Err(e) => println!("keyring: {e}"),
            }
            if !agenium_config::setup::needs_setup(&r) {
                let app = open(&r)?;
                let cfg = app.config();
                println!(
                    "approval policy: {:?}{}",
                    cfg.sandbox.approval,
                    match (cfg.sandbox.approval, sb.backend.is_some()) {
                        (ApprovalPolicy::Never, false) =>
                            " (YOLO is refused: no sandbox backend, fail-closed)",
                        (ApprovalPolicy::Sandboxed, true) =>
                            " (commands run inside the sandbox without asking)",
                        (ApprovalPolicy::Sandboxed, false) =>
                            " (no sandbox backend: commands need your approval)",
                        _ => "",
                    }
                );
                println!("secrets: {}", app.secrets().describe());
                println!(
                    "projects: {}, agents: {}",
                    app.projects()?.len(),
                    app.agents()?.len()
                );
            }
        }
        Cmd::Secret(sc) => {
            let app = open(&root(cli)?)?;
            match sc {
                SecretCmd::Set { name } => {
                    let v = read_secret_stdin()?;
                    app.secrets().set(name, v.as_bytes())?;
                    println!("stored {name}");
                }
                SecretCmd::List => app.secrets().list()?.iter().for_each(|n| println!("{n}")),
                SecretCmd::Delete { name } => {
                    app.secrets().delete(name)?;
                    println!("deleted {name}");
                }
            }
        }
        Cmd::Backup { kind } => {
            let app = open(&root(cli)?)?;
            if kind == "list" {
                for m in agenium_store::backup::list_sets(&app.backups_dir())? {
                    println!(
                        "{}\t{:?}\t{}\t{} db(s)",
                        m.id,
                        m.kind,
                        m.created_at,
                        m.dbs.len()
                    );
                }
                return Ok(());
            }
            let req = match kind.as_str() {
                "full" => BackupRequest::Full,
                "incr" => BackupRequest::Incremental,
                _ => BackupRequest::Scheduled,
            };
            match app.backup(req, chrono::Utc::now())? {
                Some(m) => println!(
                    "backup {} ({:?}) in {}",
                    m.id,
                    m.kind,
                    app.backups_dir().display()
                ),
                None => println!("no backup due"),
            }
            let cfg = app.config().backup;
            let pruned = agenium_store::backup::prune(
                &app.backups_dir(),
                agenium_store::backup::Retention {
                    hourly: cfg.hourly_keep as usize,
                    daily: cfg.daily_keep as usize,
                    weekly: cfg.weekly_keep as usize,
                },
            )?;
            if !pruned.is_empty() {
                println!("pruned {} old set(s)", pruned.len());
            }
        }
        Cmd::Restore { set, into } => {
            let r = root(cli)?;
            let app = open(&r)?;
            let m = agenium_orchestrator::maintenance::restore_into_new_root(
                &r,
                &app.backups_dir(),
                set,
                into,
            )?;
            println!(
                "restored {} into {} (content hashes verified)",
                m.id,
                into.display()
            );
        }
        _ => unreachable!("dispatched by main"),
    }
    Ok(())
}
