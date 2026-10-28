//! DERIVED FROM CODEX — keep in step with the vendored upstream files:
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/policy.rs   (`build_request`: policy shape)
//!   vendor/codex-sandbox/upstream/codex-rs/mxc-sandbox/src/windows.rs  (`platform_read_roots`)
//!   (openai/codex @ f021168bde5357009fceb0477835ad90bd54c17c)
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

/// Command line for `CreateProcess` from an argv.
pub fn command_line(argv: &[String]) -> Result<String, CommandLineError> {
    cmdline_from_argv_for_context(argv, CommandLineContext::WindowsCreateProcess)
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
