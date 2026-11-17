//! Real check that a helper chat resumes its vendor CLI's own session (ADR-151; 2026-11-17):
//! 1. turn 1 on CLI A tells a code word;
//! 2. turn 2 on CLI A is sent with the first message's code word blanked out: a resumed session answers with the word
//!    it was told (only the new message was sent), a fresh one could not;
//! 3. turn 3 on CLI B (another CLI) starts a new session: it sees the conversation as text, blanked word and all.
//!
//! cargo run -p agenium-eval --example helper_resume -- claude:haiku codex:gpt-6-luna

use std::sync::Arc;

use agenium_core::agent::{BackendKind, Effort};
use agenium_core::*;
use agenium_llm::Message;

fn binding(s: &str) -> ModelBinding {
    let (provider, model) = s.split_once(':').expect("cli:model");
    ModelBinding {
        provider: provider.into(),
        model: model.into(),
        effort: Effort::LOW,
        max_tokens: None,
    }
}

fn kept_id(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("agenium-session.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["id"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = binding(args.first().map_or("claude:haiku", String::as_str));
    let b = binding(args.get(1).map_or("codex:gpt-6-luna", String::as_str));
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let app = Arc::new(app);
    let rt = agenium_orchestrator::runtime::Runtime::new(
        app.clone(),
        Arc::new(agenium_orchestrator::runtime::StaticProviders(vec![])),
        Arc::new(agenium_orchestrator::runtime::AutoApprove),
    );
    let session = format!("resume{}", std::process::id());
    let kept = app.helper_cli_session(&session).unwrap();
    let system = "You are a helper in a short test chat. Answer in one short line.";
    let told = "Remember this code word for later: PAPAYA-7. Reply only OK.";
    let blank = "Remember this code word for later: [removed]. Reply only OK.";
    let ask = "What is the code word I told you? Reply with the word only, or NONE if you do not know it.";
    let mut fails = 0;
    let mut check = |name: &str, ok: bool, seen: &str| {
        if !ok {
            fails += 1;
        }
        println!("{} {name}: {seen}", if ok { "PASS" } else { "FAIL" });
    };

    let say = |b: &ModelBinding, msgs: Vec<Message>| {
        let rt = &rt;
        let (b, session) = (b.clone(), session.clone());
        async move {
            let t = std::time::Instant::now();
            let r = rt
                .side_chat(
                    "design",
                    Some(&session),
                    BackendKind::Cli,
                    &b,
                    system,
                    msgs,
                    2000,
                )
                .await;
            (r, t.elapsed().as_secs_f64())
        }
    };

    // 1. Turn 1 on A.
    let (r, s1) = say(&a, vec![Message::user_text(told)]).await;
    let one = r?.text;
    let id1 = kept_id(&kept);
    check(
        "turn 1 keeps the CLI's session",
        !id1.is_empty(),
        &format!("{one:?} in {s1:.0} s, session {id1}"),
    );

    // 2. Turn 2 on A, the first message blanked: only a resumed session knows the word.
    let (r, s2) = say(
        &a,
        vec![
            Message::user_text(blank),
            Message::assistant_text(one.clone()),
            Message::user_text(ask),
        ],
    )
    .await;
    let two = r?.text;
    let id2 = kept_id(&kept);
    check(
        "turn 2 on the same CLI resumes its session",
        two.contains("PAPAYA-7") && id2 == id1,
        &format!("{two:?} in {s2:.0} s, session {id2}"),
    );

    // 3. Turn 3 on B: a new session, shown the conversation as text.
    let (r, s3) = say(
        &b,
        vec![
            Message::user_text(blank),
            Message::assistant_text(one),
            Message::user_text(ask),
            Message::assistant_text(two.replace("PAPAYA-7", "[removed]")),
            Message::user_text("Say the code word from my FIRST message exactly as written there."),
        ],
    )
    .await;
    let three = r?.text;
    let id3 = kept_id(&kept);
    check(
        "another CLI starts a new session",
        !three.contains("PAPAYA-7") && !id3.is_empty() && id3 != id1,
        &format!("{three:?} in {s3:.0} s, session {id3}"),
    );

    app.remove_session(&session).ok();
    check(
        "deleting the chat removes its CLI session",
        !kept.exists(),
        "",
    );
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
