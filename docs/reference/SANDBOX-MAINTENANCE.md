# Sandbox maintenance guide

Audience: whoever keeps Agenium's process sandbox safe and up to date. Decisions: ADR-39 (platforms), ADR-41 (layering and
vendoring), ADR-42 (Linux), ADR-43 (macOS).

## 1. Layering (what lives where)

```
vendor/codex-sandbox/            UNMODIFIED OpenAI Codex files (Apache-2.0) + VENDOR.md + FILES.txt
vendor/patches/                  numbered patches, only if wrapping upstream is impossible (none today)
scripts/sync-codex-sandbox.sh    diff / update vendored files against an upstream commit
crates/agenium-sandbox/          ALL Agenium-specific code, behind `trait Sandbox`
  src/policy.rs                  SandboxPolicy / ExecRequest (roots, network, env, limits)
  src/exec.rs                    real-path resolution of the executable, env allowlist, `Prepared` command
  src/detect.rs                  capability probing by actually running things (bwrap, Landlock ABI, seatbelt)
  src/bwrap.rs                   Linux backend 1 (preferred)
  src/landlock.rs + src/bin/agenium-sandbox-helper.rs   Linux backend 2 (Landlock ABI >= 3 + seccomp), fail-closed
  src/seatbelt.rs                macOS backend (Seatbelt profile v2) — EXPERIMENTAL
  src/orphans.rs                 `AGENIUM_RUN_ID` marker based orphan cleanup
  tests/attack.rs, tests/payload/ the attack suite — CI test target `sandbox-attack-tests`
```

Rules:

1. **Never edit a file under `vendor/codex-sandbox/upstream/`.** They are byte-for-byte upstream. `scripts/sync-codex-sandbox.sh
   <ref> --summary` shows `same / changed / new / removed-upstream` per file; any local drift is a bug.
2. Agenium additions go into `crates/agenium-sandbox` as a wrapper, never into vendored files. If upstream truly cannot be
   wrapped, add `vendor/patches/NNNN-topic.patch` (with a header explaining why), apply with
   `scripts/sync-codex-sandbox.sh --apply-patches`, and re-check the patch after every sync.
3. Callers (tools, vcs, orchestrator) only see `agenium_sandbox::{select, Sandbox, SandboxPolicy, ExecRequest}`. Selection is
   fail-closed: no usable backend => `SandboxError::Unavailable`, the agent does not run (only an explicit user opt-out
   `allow_unsandboxed` bypasses this).

### What is compiled vs reference-only

| Vendored file(s) | Status |
|---|---|
| `seatbelt_base_policy.sbpl`, `seatbelt_network_policy.sbpl`, `seatbelt_preferences_policy.sbpl`, `seatbelt_read_only_platform_defaults.sbpl` | compiled into `agenium-sandbox` with `include_str!` (macOS profile v2 starts from these) |
| Codex `seatbelt.rs`, `landlock.rs`, `bwrap.rs`, `seccomp.rs`, `linux_run_main.rs` | **reference only** — they depend on Codex-internal crates (`codex-protocol`, `codex-sandboxing`, `codex-utils-*`) that we do not vendor. We read the diff on each sync and port hardening by hand. |
| Windows sandbox crates | not vendored (Windows is out of the first release) |

## 2. Updating the vendored Codex code

1. `scripts/sync-codex-sandbox.sh <upstream-ref> --summary`, then without `--summary` for the full diff.
2. Triage each changed file:
   * `.sbpl`: the change takes effect automatically on the next build. **Run the macOS attack suite** (CI job
     `sandbox-pentest / macos`) before accepting.
   * reference `.rs`: look for new deny rules, new syscalls in the seccomp filter, new bwrap flags, new Landlock access rights;
     port them into `agenium-sandbox` and add an attack-test case for each.
3. `scripts/sync-codex-sandbox.sh <ref> --update` rewrites the files and the hash/date in `VENDOR.md`.
4. Commit `vendor/` **alone** (`chore(vendor): codex-sandbox @ <short-hash>`), then the `agenium-sandbox` follow-ups in a
   separate commit.
5. Run `cargo xtask sandbox-attack-tests` on Linux (and let CI run macOS). Keep `NOTICE` attributing Codex.

## 3. The attack suite (`sandbox-attack-tests`)

* Run: `cargo xtask sandbox-attack-tests` (= `AGENIUM_REQUIRE_SANDBOX=1 cargo test -p agenium-sandbox --test sandbox-attack-tests`).
* `AGENIUM_REQUIRE_SANDBOX=1` makes "no backend on this runner" a **failure** (CI sets it). Without it the suite skips with a message
  (convenient on a laptop with a locked-down userns).
* A Python payload (`tests/payload/payload.py`) runs *inside* the sandbox and attempts the attacks; the Rust harness additionally checks the
  *host* for side effects (a write that lands in bwrap's private tmpfs is not an escape; a changed host file is).
* Linux (verified on the author's box, kernel 6.12): **36 attacks + 8 legitimate actions** pass on bwrap, and on the Landlock helper (ABI >= 3).
  Known, documented gap of the Landlock backend: `chmod`/`utime` on files outside the writable roots succeed (bwrap blocks them).
  Prefer bwrap whenever it works (that is what `Preference::Auto` does).
* macOS: `macos_seatbelt_attack_suite` runs the same payload under Seatbelt profile v2. **It has not been executed by the author** (no Mac was
  available); the owner's separate macOS spike reported 30 attack cases, which were not imported into this repo. Treat macOS as EXPERIMENTAL until the
  CI macOS job is green and those 30 cases are ported. The macOS CI jobs are `continue-on-error` until then.
* Also covered: fail-closed selection, old kernel without bwrap is refused, Landlock helper refuses weak ABI, ABI < 3 leaves the truncate gap
  (simulated, not a real old kernel), environment scrub to the allowlist, orphan cleanup, `git` inside the sandbox (`agenium-vcs` test `sandboxed_git`).

### Adding an attack
1. Add a function to `tests/payload/payload.py` that tries the attack and records `BLOCKED`/`ALLOWED` under a unique name.
2. Add the name to the expected list in `tests/attack.rs` (`check_common` or the backend-specific list), plus a host-side assertion if the attack
   could touch the host.
3. If a backend cannot block it, document it in the backend-specific list (and in this file) instead of weakening the test.

## 4. Environment requirements

| Platform | Needs | CI note |
|---|---|---|
| Linux | `bwrap` (bubblewrap) working with unprivileged user namespaces, **or** kernel >= 6.2 (Landlock ABI >= 3) + the `agenium-sandbox-helper` binary next to the app | Ubuntu 24.04 runners: `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` |
| macOS | `/usr/bin/sandbox-exec` (Seatbelt; deprecated by Apple but functional) | experimental |
| Windows | unsupported in the first release (code stays cfg-gated) | not in CI |

## 5. Design invariants (do not break)

* Fail closed. Probe by running, not by version strings.
* Resolve the real path of the executable *before* sandboxed exec (no replaceable symlink between check and exec).
* Run every sandboxed child with a cleared environment + explicit allowlist (`PATH`, `LANG`, `TERM`, `LC_ALL` + explicit extras). The launcher process itself
  is also env-cleared (a canary leaked via PID 1's environ in an early bwrap version).
* macOS: broad `file-read*` with explicit deny exceptions (must deny `~/.ssh`, keychains, browser profiles), `env -i`, and an `ulimit` wrapper
  (Seatbelt cannot limit fork/RAM). Paths are passed as `-D` parameters, never interpolated into the profile text.
* Every sandboxed process carries `AGENIUM_RUN_ID` so orphans can be found and killed after a crash.
