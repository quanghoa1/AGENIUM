# ADR-55: macOS sandbox backend = Seatbelt through `mxc-sdk` (experimental)

- **Status:** Accepted; **first run on a Mac 2026-10-25** (virtual M4 Pro, macOS 26.6.2): `agenium doctor` detects
  Seatbelt, and the sandbox tests that need a backend pass inside it (live stdin/stdout round trip, clean environment,
  timeout kills the process tree). Raw: `docs/reference/bench-raw/macos-2026-10-25/`. **Containment matrix run
  2026-10-26** (same machine, commit bc3bb60): every file escape blocked; three open items, see "Containment on a
  Mac". Raw: `docs/reference/bench-raw/macos-2026-10-26/`.
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
  The probe now reports git's stderr. To be confirmed on the next Mac run.

Open:
- **A daemon outlives the command** (double fork + `setsid`, both modes). macOS has no PID namespace or job object.
  Seatbelt still confines the daemon (it cannot write outside the worktree or read the home). But it
  keeps running after the run and after a timeout. Bubblewrap's PID namespace kills it on Linux, and MXC's job object
  does on Windows.
- **With network on, a host unix socket is reachable.** `mxc-sdk` 1.0.0 emits an unfiltered `(allow network-outbound)`,
  which in Seatbelt also covers AF_UNIX `connect()` to any path. A fixed-path socket such as Docker's
  `/var/run/docker.sock` is therefore reachable from a sandboxed command when the network is on. Listing directories
  outside the read roots is still denied, so sockets at random paths (the ssh-agent's) are hard to find. The SDK has no
  rule for scoping unix sockets. A `profile_override` replaces the whole profile, and the request-to-profile conversion is
  crate-private. To raise upstream with mxc-sdk, together with DNS in the Linux private network (ADR-95).
