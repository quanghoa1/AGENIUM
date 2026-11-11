# Sandbox maintenance guide

Audience: whoever keeps Agenium's process sandbox safe and up to date. Decisions: ADR-41 (layering and vendoring),
ADR-53 (Windows: MXC), ADR-54 (Linux: bubblewrap), ADR-42/43 (earlier Linux/macOS plans). Spike evidence:
`docs/spike-reports/S2/`.

*Rewritten 2026-10-24 for the fresh implementation; the previous version described the M0 trial's layout.*

## 1. Layering (what lives where)

```
vendor/codex-sandbox/
  upstream/<path>              OpenAI Codex files, byte-for-byte (Apache-2.0); NEVER edited
  FILES.txt                    which upstream paths are vendored (edit this to add/remove a file)
  FILES.lock                   sha256 of every vendored file (generated)
  VENDOR.md                    upstream commit + map: upstream file -> Agenium code that uses it
devtools/scripts/sync-codex-sandbox.sh  compare / update the vendored files against any upstream ref
security/sandbox/src/
  codex_derived/               the ONLY Agenium code adapted from Codex; every file names its upstream source
    mxc_probe.rs               <- codex-rs/mxc-sandbox/src/availability.rs
    mxc_policy.rs              <- codex-rs/mxc-sandbox/src/policy.rs, windows.rs, windows-sandbox-rs/src/cmd_script.rs
  mxc.rs                       Windows backend glue (toolchain roots, long paths, run loop) — Agenium's own
  unix.rs                      Linux (bubblewrap) and macOS (Seatbelt) backend through mxc-sdk v1 — Agenium's own
  seatbelt.rs                  macOS, network on: mxc-sdk's own Seatbelt profile with its open outbound rule narrowed (no host unix sockets)
  lib.rs                       detect() (fail-closed), SandboxPolicy, Executor, clean environment, process-tree kill
```

Third-party library (not vendored): `mxc-sdk` (Microsoft, MIT) from crates.io, pinned `=1.0.1` (1.0.0 until 2026-10-26; 1.0.1 resolves DNS through a host loopback stub resolver). On a bump, `seatbelt::tests` must still pass: it fails when the SDK's Seatbelt profile no longer has the one open outbound rule Agenium narrows.

Rules:

1. Never edit `vendor/codex-sandbox/upstream/`. `cargo xtask vendor-check` (a local-CI step) fails on any drift.
2. Code taken or adapted from Codex goes into `codex_derived/` and nowhere else, with the header `DERIVED FROM CODEX`
   naming the upstream file(s) and commit. Agenium-only behaviour stays outside that module, so an upstream change
   maps to a small, known set of files.
3. `NOTICE` lists the derived files; keep it in step when a derived file is added or removed.
4. Fail closed: a backend is reported only when it actually works on the machine (probe, not version strings).

## 2. When Codex or mxc-sdk changes

0. `devtools/scripts/upstream-watch.sh` (weekly, and before any sandbox work) reports in one go: the vendored Codex
   files against `openai/codex` main, the `mxc-sdk` pin against crates.io, the latest `microsoft/mxc` release, and the
   upstream issues/PRs an open Agenium item waits on (its `WATCH` list; add one whenever an ADR leaves something to
   upstream). Exit 1 means something to look at.
1. `devtools/scripts/sync-codex-sandbox.sh main --summary` — prints `same / changed / new / missing-upstream` per vendored file.
   Without `--summary` it prints the full unified diff.
2. For each **changed** file look up its row in `VENDOR.md`:
   * **derived** -> port the change into the matching `codex_derived/` file (update its commit line), rebuild, run the
     sandbox tests on Windows (and the S2 matrix in `docs/spike-reports/S2/evidence/` for policy changes);
   * **reference** -> read the diff for hardening worth adopting (new denied syscalls, bwrap flags, Seatbelt deny
     rules); adopt it in `unix.rs` if it applies to Agenium's allow-list model.
3. `devtools/scripts/sync-codex-sandbox.sh <ref> --update` rewrites the vendored files, `FILES.lock` and the commit line of
   `VENDOR.md`.
4. Commit `vendor/` **alone** (`chore(vendor): codex-sandbox @ <short-hash>`), then the `codex_derived/` follow-ups in
   a separate commit, so the upstream diff and Agenium's port are reviewed separately.
5. If Codex moves to a newer `mxc-sdk`, bump the pin in `security/sandbox/Cargo.toml` only after the S2 matrix
   passes again.

## 3. Platform notes

| Platform | Backend | Needs | Notes |
|---|---|---|---|
| Windows | MXC process container (`mxc-sdk`) | a Windows build with the process security environment API (seen on 11 build 26300) | no admin, no extra users; shell = PowerShell (MSYS/Git Bash cannot start in the container); build needs `RC_PATH` (`devtools/scripts/rc-windres.cmd`) on the GNU toolchain |
| Linux | bubblewrap (`mxc-sdk` v1, allow-list binds) | `bwrap` + unprivileged user namespaces | verified on WSL2 Ubuntu 24.04; Ubuntu hosts may need `kernel.apparmor_restrict_unprivileged_userns=0`; the host resolver's sockets that mxc-sdk 1.0.1 binds in are masked by `unix.rs` (`resolver_sockets`, containment row `net.resolver_socket`, 2026-10-30; drop it once a release has mxc #1568) |
| macOS | Seatbelt (`mxc-sdk` v1, ADR-55) | `sandbox-exec` | experimental; containment run on a Mac 2026-10-26: file escapes blocked, a daemon outlived the command (killed by its environment token since 2026-10-26, confirmed on the fifth Mac run: 0 escapes), a host unix socket was reachable with network on: narrowed by `seatbelt.rs` (Agenium's edit of mxc-sdk's profile; drop it once mxc #1277 lands), confirmed blocked on the fourth Mac run (301 passed, 0 failed) | Open: Codex (fe8e540) denies `TIOCSTI` to a command that inherits the terminal; Agenium's commands have piped stdio but stay in the session of the `agenium` CLI when it runs in a terminal (the desktop app has none), so a contained command could open `/dev/tty`: adopt the deny (in every profile, network off too) on the next Mac run.

## 4. Design invariants (do not break)

* Every agent command runs with a cleared environment plus an explicit allowlist; secrets never reach it.
* Writable: the ticket worktree and a temp dir outside it. Readable: system/platform folders, toolchains, the
  repository's git directory, the agent's skill folders. Nothing else (in particular not the user profile, `~/.ssh`,
  other worktrees, the Agenium root).
* Network denied unless the policy allows it; enforced by the OS (WFP / network namespace), never by proxy variables.
* Timeouts kill the whole process tree; commands do not survive their run.
