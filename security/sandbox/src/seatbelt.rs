//! The Seatbelt profile of a command with the network on (ADR-55). mxc-sdk 1.0.1 opens the network with an unfiltered
//! `(allow network-outbound)`, and in Seatbelt that rule also lets the command connect to any unix socket on the host
//! (Docker's, a database's: macOS run 2026-10-26). Agenium asks mxc-sdk for the profile it would build, through its
//! published 1.0.0 request contract and its own profile builder, narrows that one rule to IP peers and the system
//! resolver's socket, and passes the result back as the profile override. Upstream has no rule for unix sockets yet
//! (microsoft/mxc#1277); once it has, this module goes.
//!
//! Only string work, so it is tested on every OS.

use mxc_sdk::mxc_common::config_parser::load_mxc_request_from_json;
use mxc_sdk::mxc_common::logger::{Logger, Mode};
use mxc_sdk::mxc_common::state_aware_request::MxcRequest;
use mxc_sdk::seatbelt_common::profile_builder::build_profile_with_proxy;
use mxc_sdk::v1::configs::SeatbeltConfig;
use serde_json::json;

/// The rule mxc-sdk emits for an open egress default.
const OPEN: &str = "(allow network-outbound)\n";
/// What it becomes: any IP peer (TCP and UDP, every host and port), and the socket macOS resolves names through
/// (`getaddrinfo` asks mDNSResponder). No other unix socket.
const NARROWED: &str = "(allow network-outbound (remote ip \"*:*\"))\n\
                        (allow network-outbound (literal \"/private/var/run/mDNSResponder\"))\n";

/// The profile of a command with the network on: mxc-sdk's own for these paths, with egress open, no inbound and no
/// host loopback, and the open outbound rule narrowed. `Err` when mxc-sdk's profile no longer has exactly that rule.
pub fn network_on_profile(readwrite: &[String], readonly: &[String]) -> Result<String, String> {
    let sb = SeatbeltConfig::default();
    let doc = json!({
        "version": "1.0.0",
        "containment": "seatbelt",
        "process": {"commandLine": "true"},
        "filesystem": {"readwritePaths": readwrite, "readonlyPaths": readonly},
        "network": {
            "egress": {"default": "allow"},
            "ingress": {"default": "deny", "hostLoopback": "deny"}
        },
        "seatbelt": {
            "guiAccess": sb.gui_access,
            "nestedPty": sb.nested_pty,
            "keychainAccess": sb.keychain_access,
            "extraMachLookups": sb.extra_mach_lookups
        }
    });
    let mut log = Logger::new(Mode::Buffer);
    let request = match load_mxc_request_from_json(&doc.to_string(), &mut log) {
        Ok(MxcRequest::OneShot(r)) => r,
        Ok(_) => return Err("mxc-sdk read the Seatbelt request as a lifecycle request".into()),
        Err(e) => return Err(format!("mxc-sdk refused the Seatbelt request: {e:?}")),
    };
    let profile = build_profile_with_proxy(&request, None)?;
    match profile.matches(OPEN).count() {
        1 => Ok(profile.replacen(OPEN, NARROWED, 1)),
        n => Err(format!(
            "mxc-sdk's Seatbelt profile has {n} open outbound rules, Agenium expects one to narrow"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails when an mxc-sdk update changes the rule, so the narrowing is never silently lost.
    #[test]
    fn the_open_outbound_rule_is_narrowed_to_ip_and_the_resolver() {
        let p = network_on_profile(&["/tmp/wt".into()], &["/usr".into()]).unwrap();
        assert!(!p.contains(OPEN), "{p}");
        assert_eq!(p.matches("(allow network-outbound").count(), 2, "{p}");
        assert!(p.contains("(remote ip \"*:*\")"));
        assert!(p.contains("/private/var/run/mDNSResponder"));
        // The rest is mxc-sdk's: deny by default, the worktree writable, no host loopback after the allow.
        assert!(p.starts_with("(version 1)\n(deny default)\n"));
        assert!(p.contains("\"/private/tmp/wt\""), "{p}");
        let allow = p.find("(remote ip \"*:*\")").unwrap();
        let loopback = p
            .find("(deny network-outbound (remote ip \"localhost:*\"))")
            .unwrap();
        assert!(loopback > allow);
        assert!(!p.contains("network-inbound"));
    }
}
