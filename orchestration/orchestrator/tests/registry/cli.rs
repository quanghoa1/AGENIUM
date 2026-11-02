//! ADR-153: a CLI version that answered Agenium's check after an update on this machine is kept in the root, so the
//! page can say it was checked here instead of warning that it is newer than the version Agenium verified.

use crate::common::*;
use agenium_orchestrator::cli_updates::CliCheck;

fn check(version: &str, updater_had_none: bool) -> CliCheck {
    CliCheck {
        version: version.into(),
        at: "2026-11-02T01:00:00Z".into(),
        updater_had_none,
    }
}

/// Each update's check is kept under the CLI's program, and a later one adds to the file rather than replacing it.
#[test]
fn a_cli_check_is_kept_in_the_root_and_merged() {
    let f = fixture();
    f.app
        .record_cli_check("opencode", check("2.2.5", false))
        .unwrap();
    f.app
        .record_cli_check("agy", check("1.4.12", true))
        .unwrap();
    let path = f.app.layout().cli_checks();
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(kept["opencode"]["version"], "2.2.5");
    assert_eq!(kept["opencode"]["updater_had_none"], false);
    assert_eq!(kept["opencode"]["at"], "2026-11-02T01:00:00Z");
    assert_eq!(kept["agy"]["version"], "1.4.12");
    assert_eq!(kept["agy"]["updater_had_none"], true);
}
