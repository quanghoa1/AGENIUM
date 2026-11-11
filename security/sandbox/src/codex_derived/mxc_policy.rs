//! DERIVED FROM CODEX — keep in step with the vendored upstream files:
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/policy.rs   (`build_request`: policy shape)
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/windows.rs  (`platform_read_roots`)
//!   vendor/codex-sandbox/upstream/codex-rs/windows-sandbox-rs/src/cmd_script.rs  (`cmd_script_command_line`)
//!   (openai/codex @ fe8e540a80f17716bf2a3007f06e8a602192c704)
//! Licensed under the Apache License 2.0 (OpenAI Codex), see NOTICE.
//! Changes from upstream: no permission-profile translation (Agenium passes explicit read/write lists), no managed
//! network proxy (network is either denied in every direction or allowed), platform folders read from the
//! environment instead of `SHGetKnownFolderPath`. As upstream since f021168, MXC's object normalizer is not called:
//! BaseContainer owns the precedence of overlapping paths (upstream README: hard-link names keep their own access).

use std::path::PathBuf;

use mxc_sdk::mxc_common::cmdline::{
    cmdline_from_argv_for_context, CommandLineContext, CommandLineError,
};
use mxc_sdk::mxc_common::models::{
    BaseProcessUiConfig, ContainerPolicy, FallbackPolicy, NetworkAction, NetworkEgressPolicy,
    NetworkIngressPolicy, UiPolicy,
};

/// Windows folders every process needs to start (Windows, Program Files, ProgramData).
pub fn platform_read_roots() -> Vec<PathBuf> {
    [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .filter(|p| p.is_dir())
    .collect()
}

/// Command line for `CreateProcess` from an argv: a `cmd /c` script as cmd reads it ([`cmd_script`]), any other argv
/// with the C runtime's quoting.
pub fn command_line(argv: &[String]) -> Result<String, CommandLineError> {
    match cmd_script(argv) {
        Some(line) => Ok(line),
        None => cmdline_from_argv_for_context(argv, CommandLineContext::WindowsCreateProcess),
    }
}

/// `cmd [/d] [/s] /c SCRIPT` (Agenium's cmd shell passes `/D /S /C`): the script is shell syntax, and the C runtime's
/// quoting would put a backslash before each of its quotes, which cmd keeps. With `/s` cmd strips exactly the outer
/// pair, so the script goes in one pair of quotes as it is. Upstream: `cmd_script_command_line`
/// (windows-sandbox-rs/src/cmd_script.rs), which takes `cmd /c SCRIPT` only; here `/d` is kept when given.
fn cmd_script(argv: &[String]) -> Option<String> {
    let (program, rest) = argv.split_first()?;
    let (script, switches) = rest.split_last()?;
    let name = program.rsplit(['/', '\\']).next()?;
    let is = |s: &String, w: &str| s.eq_ignore_ascii_case(w);
    if !(name.eq_ignore_ascii_case("cmd.exe") || name.eq_ignore_ascii_case("cmd"))
        || !switches.last().is_some_and(|s| is(s, "/c"))
        || !switches
            .iter()
            .all(|s| is(s, "/c") || is(s, "/s") || is(s, "/d"))
        || program.contains(['"', '\0'])
        || script.contains('\0')
    {
        return None;
    }
    let d = if switches.iter().any(|s| is(s, "/d")) {
        " /d"
    } else {
        ""
    };
    Some(format!(r#""{program}"{d} /s /c "{script}""#))
}

/// The container policy: registry read, no DACL fallback, directional network rules, desktop UI isolation.
pub fn container_policy(
    readwrite: Vec<String>,
    readonly: Vec<String>,
    network: bool,
) -> ContainerPolicy {
    let net = if network {
        NetworkAction::Allow
    } else {
        NetworkAction::Deny
    };
    ContainerPolicy {
        capabilities: vec!["registryRead".to_owned()],
        readwrite_paths: readwrite,
        readonly_paths: readonly,
        denied_paths: vec![],
        fallback: FallbackPolicy {
            allow_dacl_mutation: false,
        },
        network_egress: Some(NetworkEgressPolicy {
            default: net,
            ..Default::default()
        }),
        network_ingress: Some(NetworkIngressPolicy {
            default: net,
            host_loopback: net,
        }),
        network_specified: true,
        network_mode_specified: true,
        // PowerShell needs Win32k and desktop handles during DLL startup. Keep clipboard, input injection and
        // system-control restrictions.
        ui: UiPolicy {
            disable: false,
            ..Default::default()
        },
        base_process_ui: BaseProcessUiConfig {
            isolation: "desktop".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_cmd_script_keeps_its_quotes() {
        let script = r#"echo "a b" & dir "C:\Program Files""#;
        assert_eq!(
            command_line(&v(&["cmd", "/D", "/S", "/C", script])).unwrap(),
            format!(r#""cmd" /d /s /c "{script}""#)
        );
        assert_eq!(
            command_line(&v(&[r"C:\Windows\System32\cmd.exe", "/c", "dir"])).unwrap(),
            r#""C:\Windows\System32\cmd.exe" /s /c "dir""#
        );
    }

    #[test]
    fn other_argvs_keep_the_c_runtime_quoting() {
        let ps = command_line(&v(&["powershell.exe", "-Command", r#"echo "x""#])).unwrap();
        assert!(ps.contains(r#"\"x\""#), "{ps}");
        // Not a cmd script: an extra argument after the script, an unknown switch, a NUL.
        assert!(cmd_script(&v(&["cmd", "/c", "dir", "x"])).is_none());
        assert!(cmd_script(&v(&["cmd", "/k", "dir"])).is_none());
        assert!(cmd_script(&v(&["cmd", "/c", "a\0b"])).is_none());
        assert!(cmd_script(&v(&["cmd"])).is_none());
    }
}
