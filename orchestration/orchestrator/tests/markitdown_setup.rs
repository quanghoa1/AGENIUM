//! ADR-152: Agenium's own markitdown, installed for real into a throwaway root (network, a few minutes each; run with
//! `--ignored`). The copy is self-tested on Agenium's samples, used for documents, updated side by side, kept when an
//! update fails, and removed with everything it fetched.

mod common;

use std::sync::{Arc, Mutex};

use agenium_orchestrator::markitdown_setup::{Method, VERSION};
use common::*;

fn steps() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    (seen, move |m: &str| {
        eprintln!("  · {m}");
        s.lock().unwrap().push(m.to_owned());
    })
}

#[tokio::test]
#[ignore = "installs markitdown from the network"]
async fn the_found_python_installs_a_self_tested_copy_the_tools_use() {
    let f = fixture();
    let (_, progress) = steps();
    let st = f
        .app
        .install_markitdown(None, None, &progress)
        .await
        .unwrap();
    assert_eq!(st.source, "agenium");
    assert_eq!(st.version.as_deref(), Some(VERSION));
    assert!(st.tests.iter().all(|(_, r)| r == "ok"), "{:?}", st.tests);
    assert_eq!(st.tests.len(), 4);
    // The tools take Agenium's copy, and a document reads through it.
    let m = agenium_documents::markitdown::find().unwrap();
    assert!(
        m.program.starts_with(f.app.markitdown_dir()),
        "{}",
        m.program.display()
    );
    let s = agenium_documents::samples::samples();
    let c = f.app.read_document(s[0].name, &s[0].bytes).await.unwrap();
    assert_eq!(c.format, "markitdown", "{c:?}");
    assert_eq!(f.app.test_markitdown().await.unwrap().len(), 4);
    let st = f.app.remove_markitdown().await.unwrap();
    assert_ne!(st.source, "agenium");
    assert!(!f.app.markitdown_dir().exists());
}

#[tokio::test]
#[ignore = "downloads uv and a Python, installs markitdown from the network"]
async fn a_downloaded_uv_installs_updates_side_by_side_and_keeps_the_copy_when_an_update_fails() {
    let f = fixture();
    let (seen, progress) = steps();
    let st = f
        .app
        .install_markitdown(Some("0.1.8".into()), Some(Method::UvStandalone), &progress)
        .await
        .unwrap();
    assert_eq!(st.method.as_deref(), Some("uv-standalone"));
    assert_eq!(st.version.as_deref(), Some("0.1.8"));
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .any(|m| m.starts_with("downloading uv")));
    assert!(
        f.app.markitdown_dir().join("python").is_dir(),
        "uv's Python stays in the folder"
    );
    assert!(
        !f.app.markitdown_dir().join("cache").exists(),
        "the cache is cleared"
    );

    let st = f.app.check_markitdown_update().await.unwrap();
    assert!(st.update, "{st:?}");
    let st = f.app.update_markitdown(&progress).await.unwrap();
    assert_ne!(st.version.as_deref(), Some("0.1.8"));
    assert!(st.tests.iter().all(|(_, r)| r == "ok"), "{:?}", st.tests);
    let envs: Vec<_> = std::fs::read_dir(f.app.markitdown_dir().join("envs"))
        .unwrap()
        .collect();
    assert_eq!(envs.len(), 1, "the replaced environment is removed");

    // A version that cannot be installed leaves the copy in use as it was.
    let before = st.version.clone();
    let e = f
        .app
        .install_markitdown(Some("0.0.0".into()), Some(Method::UvStandalone), &progress)
        .await
        .unwrap_err();
    assert!(e.to_string().contains("could not be installed"), "{e}");
    let st = f.app.markitdown_status().await;
    assert_eq!(st.version, before);
    assert_eq!(st.source, "agenium");

    f.app.remove_markitdown().await.unwrap();
    assert!(!f.app.markitdown_dir().exists());
}
