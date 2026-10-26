# ADR-55: macOS sandbox backend = Seatbelt through `mxc-sdk` (experimental)

- **Status:** Accepted; **first run on a Mac 2026-10-25** (virtual M4 Pro, macOS 26.6.2): `agenium doctor` detects
  Seatbelt, and the sandbox tests that need a backend pass inside it (live stdin/stdout round trip, clean environment,
  timeout kills the process tree). Raw: `docs/reference/bench-raw/macos-2026-10-25/`. **Containment matrix run
  2026-10-26** (same machine, commit bc3bb60): every file escape blocked; three open items, see "Containment on a
  Mac". Raw: `docs/reference/bench-raw/macos-2026-10-26/`. **Third run 2026-10-26** (commit 9f1ce7a, mxc-sdk
  1.0.1): both fixes confirmed, file escapes still all blocked; two items open (the unix socket one mitigated since, see below). Raw:
  `docs/reference/bench-raw/macos-2026-10-26-run3/`. **Fourth run 2026-10-26** (commit 52f1c12): 301 passed, 0
  failed; with the narrowed profile `net.unix_host_socket` is blocked in both sandboxed modes while `net.dns` and
  `net.tcp_internet` still pass with the network on; only `daemon.outlives_*` remains. Raw:
  `docs/reference/bench-raw/macos-2026-10-26-run4/`.
- **Date:** 2026-10-24 (Asia/Saigon)
- **Amends:** ADR-43 (macOS Seatbelt v2, experimental) — implementation choice only.

## Decision

macOS shares the Linux module (`security/sandbox/src/unix.rs`, `mxc_sdk::v1::spawn`) with
`Containment::Seatbelt(SeatbeltConfig::default())`: no GUI access, no keychain, no nested PTYs. Same allow-list policy as
Linux: worktree + temp writable; `/usr`, `/bin`, `/sbin`, `/System`, the Command Line Tools, Xcode, Homebrew, `/etc`
and the timezone database, PATH toolchains, the repository's git directory and skill folders read-only; network
denied; cleared environment. `detect()` reports `Seatbelt` only when `mxc_sdk::v1::available_backends()` lists it.
Codex's Seatbelt profiles (vendored, `vendor/codex-sandbox/upstream/codex-rs/sandboxing/src/*.sbpl`) allow reading the
whole disk except denied paths; they stay a reference for deny rules.

## Verification plan

Run `cargo test --workspace` on a macOS host (the owner proposed Devin cloud with the macOS platform); until then the
backend is labelled experimental in `doctor` and the docs.

## Containment on a Mac (2026-10-26)

The S3 containment harness (`security/sandbox/examples/containment.rs`) on the virtual M4 Pro, macOS 26.6.2, against
Seatbelt with network off and on, plus an unsandboxed baseline (every probe escaped there, as it must). Tests: 295
passed, 1 failed, 2 ignored.

Blocked in both sandboxed modes: writes outside the worktree (direct, `..`, symlink, hardlink, rename out, truncate,
delete, chmod/utime), the main repository's `.git/hooks` and `.git/config`, `$HOME`, `/tmp`, reads of the real home,
`~/.ssh`, a secret file outside and `/etc/shadow`, the environment canary, and, with network off, DNS, TCP to the
internet and to a host listener, and a host unix socket. With network on, only outbound internet passes, as designed.

Found and fixed:
- **The failing test** (`stream_run`, the same as on the first run) was the CLI agents' permission hook. It still
  compared paths lexically. The fix in ce7f597 went into the API tools' path check (`agenium_tools::paths`), but the hook
  has its own check (`cli-backend` `driver::inside`), so the agent's write to `/private/var/…/wt/a.txt` was still
  refused for a worktree given as `/var/…/wt`. The hook now uses the same rule (`paths::confined`: either name of the
  root, links resolved). The reproduction on Windows is the `\\?\` canonical name.
- **`git status` failed inside the sandbox** (exit 128, both modes; on Linux it passes). The harness did not keep git's
  stderr. The likely cause is that HOME stays the user's, and `~/.gitconfig` is outside the sandbox. Seatbelt answers
  EPERM there, not ENOENT as bubblewrap does, and git dies on an unreadable config. Sandboxed commands on Linux and macOS
  now get a git config of their own (`GIT_CONFIG_GLOBAL` in the run's temp dir, `GIT_CONFIG_NOSYSTEM`), as on Windows.
  The probe now reports git's stderr. Confirmed by the third run: `ok.git_status` is ok in every column.

**Third run** (commit 9f1ce7a, mxc-sdk 1.0.1; a virtual M4, 8 cores, so timings do not compare with runs 1–2). Tests:
298 passed, 1 failed, 2 ignored. `stream_run` passes. `doctor` still reports Seatbelt, and the containment matrix is
the same as in run 2 except `ok.git_status`, now ok. The SDK update changed nothing in containment.

The failing test was new: `opencode` `permission_requests_follow_agenium_rules` refused `src/a.rs` for a worktree under
the temp dir that did not exist yet. `confined` canonicalized the root only if it existed, so the root kept its `/var`
name while the path resolved through `/private/var`. The root is now resolved like the path, through its deepest
existing ancestor. A unix test with a root not created yet under a link reproduces it (fails before the fix, on Linux).
In a real run the worktree exists, so the hook was not affected.

Open:
- **A daemon outlives the command** (double fork + `setsid`, both modes). macOS has no PID namespace or job object.
  Seatbelt still confines the daemon (it cannot write outside the worktree or read the home). But it
  keeps running after the run and after a timeout. Bubblewrap's PID namespace kills it on Linux, and MXC's job object
  does on Windows.
- **With network on, a host unix socket is reachable** (runs 2 and 3). `mxc-sdk` 1.0.0 and 1.0.1 emit an unfiltered
  `(allow network-outbound)`, which in Seatbelt also covers AF_UNIX `connect()` to any path. A fixed-path socket such as
  Docker's `/var/run/docker.sock` is therefore reachable from a sandboxed command when the network is on. The SDK has no
  rule for scoping unix sockets: `profile_override` replaces the whole profile, the typed request's conversion is
  crate-private, and the proposed `deniedUnixSocketPaths` (issue microsoft/mxc#1277, PR #1278 closed unmerged on
  2026-10-24) would be a deny list. **Mitigation (2026-10-26, confirmed by the fourth Mac run):**
  `security/sandbox/src/seatbelt.rs` asks mxc-sdk for the profile it would build, through its published 1.0.0 request
  contract (`load_mxc_request_from_json`) and its own builder (`build_profile_with_proxy`), replaces that one rule with
  `(allow network-outbound (remote ip "*:*"))` plus the resolver's socket `/private/var/run/mDNSResponder`, and passes
  the result as the profile override. Everything else stays mxc-sdk's. A test on every OS fails if an SDK update no
  longer emits exactly one such rule; if the narrowing cannot be built at run time, commands run without network
  (`doctor` says why). Once upstream scopes unix sockets, the module goes. Tracked by
  `devtools/scripts/upstream-watch.sh`.
