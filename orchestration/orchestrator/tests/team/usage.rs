//! What runs cost and what the vendors' limits read: the price catalogue, the vendors' usage.

use super::*;

/// Runs of a model without a price cost 0 until the price catalogue knows the model; storing the catalogue prices
/// them, matched to the configured provider by its URL. The model list shows which models are free.
#[test]
fn the_price_catalogue_prices_earlier_runs() {
    use agenium_llm::pricing::Catalogue;
    let t = team();
    let f = &t.f;
    f.app
        .update_config(|c| {
            c.providers.push(agenium_config::ProviderConfig {
                body: vec![],
                effort_field: None,
                headers: Default::default(),
                id: "go".into(),
                kind: agenium_config::ProviderKind::OpenaiCompat,
                base_url: Some("https://opencode.ai/zen/go/v1".into()),
                api_key_ref: String::new(),
                max_concurrency: 1,
                models: vec![],
                model_caps: vec![],
            })
        })
        .unwrap();
    planned(&t);
    let k = started(f, t.web, "PI-1", "Price me", &[], &[]);
    let run = f
        .app
        .start_run(f.project.id, &k, "go", "deepseek-v4.1-flash")
        .unwrap();
    // Half of the input was served from the cache: it is billed once, at the cache price.
    let usage = UsageSummary {
        tokens_in: 2_000_000,
        tokens_out: 1_000_000,
        cache_read: 1_000_000,
        ..Default::default()
    };
    f.app.finish_run(run.run, "Completed", 3, &usage).unwrap();
    assert!(f.app.price_view().providers["go"].is_empty());
    let c = Catalogue::from_models_dev(
        &serde_json::json!({"opencode-go": {"api": "https://opencode.ai/inference/go/openai/v1", "models": {
            "deepseek-v4.1-flash": {"cost": {"input": 0.15, "output": 0.6, "cache_read": 0.015}},
            "beam-free": {"cost": {"input": 0, "output": 0}}
        }}}),
        "test",
        &now_iso(),
    );
    assert_eq!(f.app.save_price_catalogue(&c).unwrap(), 1);
    let r = f.app.store().read(|c| runs::get(c, run.run)).unwrap();
    assert!(
        (r.usage.estimated_cost_usd - 0.765).abs() < 1e-9,
        "{:?}",
        r.usage
    );
    assert_eq!(f.app.price_unpriced_runs().unwrap(), 0, "priced once");
    let view = f.app.price_view();
    assert!(view.providers["go"]["beam-free"].is_free());
    assert!(!f.app.price_catalogue_stale());
}

/// A vendor's reading is kept per vendor; a failed reading keeps the earlier windows with its error. Rows follow
/// the CLIs installed and the OpenCode Go providers configured.
#[test]
fn vendor_usage_keeps_the_last_good_reading() {
    use agenium_llm::vendor_usage::{UsageWindow, VendorUsage};
    let f = fixture();
    let good = VendorUsage {
        windows: vec![UsageWindow {
            window: "5h".into(),
            used_percent: 42.0,
            resets_at: None,
        }],
        ..VendorUsage::new("codex", "2026-10-25T00:00:00Z")
    };
    f.app.record_vendor_usage(good).unwrap();
    f.app
        .record_vendor_usage(VendorUsage {
            error: Some("codex app-server did not answer in 40 s".into()),
            ..VendorUsage::new("codex", "2026-10-25T00:15:00Z")
        })
        .unwrap();
    let codex = agenium_cli_backend::CliInfo {
        kind: agenium_cli_backend::CliKind::Codex,
        path: Some("codex".into()),
        version: Some("0.163.0".into()),
        login: agenium_cli_backend::Login::LoggedIn,
        compatible: true,
        note: None,
    };
    let cli = |kind| agenium_cli_backend::CliInfo {
        kind,
        ..codex.clone()
    };
    let (devin, grok) = (
        cli(agenium_cli_backend::CliKind::Devin),
        cli(agenium_cli_backend::CliKind::Grok),
    );
    let rows = f.app.vendor_usage(&[grok, devin, codex]);
    let ids: Vec<&str> = rows.iter().map(|r| r.vendor.as_str()).collect();
    assert_eq!(ids, ["codex", "devin", "grok"]);
    assert_eq!(rows[0].windows[0].used_percent, 42.0);
    assert_eq!(
        rows[0].updated_at, "2026-10-25T00:00:00Z",
        "the good reading's time"
    );
    assert!(rows[0].error.as_deref().unwrap().contains("did not answer"));
    // Devin and Grok are listed without numbers, saying where their quota is (ADR-67).
    assert!(rows[1].windows.is_empty() && rows[1].note.as_deref().unwrap().contains("/usage"));
    assert!(rows[2].windows.is_empty() && rows[2].note.as_deref().unwrap().contains("grok usage"));
}
