# ADR-54: Linux sandbox backend = bubblewrap through `mxc-sdk`

- **Status:** Accepted for implementation; **containment matrix (spike S3) NOT RUN yet**
- **Date:** 2026-10-24 (Asia/Saigon)
- **Amends:** ADR-42 (Linux: bubblewrap preferred, Landlock fallback, fail-closed) — implementation choice only.

## Context

The owner asked to reuse Codex's sandbox (ADR-53 did so on Windows through Microsoft's `mxc-sdk`). On Linux, Codex's
own helper (`codex-rs/linux-sandbox`, ~300 KB, Codex-internal crates) runs bubblewrap with `--ro-bind / /`: the whole
disk is readable, which Agenium's policy (spike S2 threshold: no reads outside the worktree) does not allow. `mxc-sdk`
(MIT, already a dependency) has a bubblewrap backend with an allow-list: explicit read-only and read-write binds,
`--unshare-net`, `--clearenv`, `--die-with-parent`, fresh `/dev`, `/proc`, `/tmp`.

## Decision

`agenium-sandbox` uses `mxc_sdk::v1::spawn` with `Containment::Bubblewrap` on Linux (`src/unix.rs`, shared with macOS). `detect()`
reports `Bubblewrap` only when `mxc_sdk::v1::available_backends()` lists it (bwrap installed, user namespaces work);
otherwise `None` and the fail-closed rules apply. Policy: worktree + per-worktree temp dir writable; `/usr`, `/bin`,
`/sbin`, `/lib*`, `/etc`, `/opt`, PATH toolchains (Cargo: `bin`, `registry`, `git` only; `RUSTUP_HOME`), the
repository's git directory and skill folders read-only; network denied unless the policy allows it; cleared
environment plus `TMPDIR`. Commands run as `bash -c` inside the SDK's `sh -c`. Landlock is not used.

## Evidence so far

WSL2 Ubuntu 24.04 (kernel 6.18, bubblewrap 0.9.0): `agenium doctor` reports the backend; the sandbox crate's tests
(clean environment, output capture, exit code, timeout tree kill) pass with commands inside bubblewrap; `cargo test
--workspace` passed on this distro before the backend was added.

## Not done

The S2-style containment matrix on Linux (S3), runs on a non-WSL kernel, the macOS Seatbelt backend (to be run on
Devin cloud, macOS, per the owner).
