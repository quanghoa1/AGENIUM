# Windows guide (UNSUPPORTED in v1 — build/test probe only)

**Status.** The first release targets Linux + macOS (ADR-39). Windows is *deferred*: there is no CI, no bundle, no sandbox
backend and nothing in this repo has ever been compiled or run on Windows (**NOT RUN**). This guide exists so you (or your AI
assistant, see `WINDOWS-PROMPT.md`) can find out what breaks and report it in a fixed format. Expect some failures; they
are information, not regressions.

## 1. Install the toolchain (PowerShell, as a normal user unless stated)

1. **Visual Studio Build Tools 2022** (C++ workload: "Desktop development with C++", Windows 10/11 SDK, MSVC v143, CMake optional).
   `winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`
2. **Rust** (MSVC toolchain): `winget install Rustlang.Rustup` then `rustup default stable-x86_64-pc-windows-msvc`
   (the repo pins the channel in `rust-toolchain.toml`; rustup installs it on first use).
3. **Git** (>= 2.30): `winget install Git.Git`. **Node 20 + pnpm 10:** `winget install OpenJS.NodeJS.LTS` then `corepack enable; corepack prepare pnpm@10 --activate`.
4. **WebView2 runtime** (already present on Windows 11; needed only to *run* the Tauri shell).
5. Open a new **"x64 Native Tools Command Prompt"/PowerShell** so `cl.exe` is on PATH. Do not run inside WSL (that is the Linux path).
6. Long paths: `git config --global core.longpaths true`; enable Windows long paths if the build complains.

## 2. One-command check

```powershell
git clone <repo> agenium-app; cd agenium-app
.\scripts\ci-local.ps1 -NoTauri      # Rust-only first; add the Tauri crate once that passes
.\scripts\ci-local.ps1               # full set (needs WebView2 + the Tauri prerequisites)
```

`ci-local.ps1` runs: fmt, clippy (default + `--features surrealkv` on `agenium-store`), `cargo test` for the workspace, the
surreal-feature tests, `cargo xtask schemas --check`, `pnpm install/vitest/build`; it **skips** `sandbox-attack-tests`
(no Windows backend) and prints a PASS/FAIL/SKIP summary + `target\ci-local\<step>.log`. Flags: `-Quick` (fmt, clippy, test), `-SkipUi`,
`-SkipSurreal`, `-NoTauri`, `-FailFast`, `-Heavy` (store scale benchmarks, ~3 GB disk, uses a synthetic corpus).
Or step by step: `cargo test --workspace`, `pnpm --dir app/ui install --frozen-lockfile; pnpm --dir app/ui build`.

## 3. What to expect on Windows (cfg-gated / deferred)

| Area | State on Windows | Expected result |
|---|---|---|
| `agenium-sandbox` | no backend (Linux: bwrap/Landlock; macOS: Seatbelt). `select()` returns an error → **fail closed**, agents refuse to run. `libc` dep is `cfg(unix)`; the helper binary is Linux-only | lib should compile; attack suite **SKIP**; if the crate does not compile, report the first error |
| `agenium-secrets` | OS keyring = Windows Credential Manager via `keyring`; file vault falls back without `0600` mode (`cfg(not(unix))`) | tests that need a keyring service may fail on a headless box |
| `agenium-store` (SQLite, default) | `rusqlite` bundled + `sqlite-vec` + `usearch` 2.26.5 (C++17) | Builds and passes with the GNU toolchain and MSYS2 gcc (2026-10-28, ADR-125); MSVC not tried. libstdc++ is linked statically (`CXXSTDLIB_x86_64_pc_windows_gnu` in `.cargo/config.toml`): a clean Windows has no `libstdc++-6.dll`. `flock` locking uses an OS-specific implementation — report failures |
| `agenium-store` `--features surrealkv` | optional reference backend | long build; skip with `-SkipSurreal` if it fails |
| `agenium-vcs` | drives the `git` CLI | needs `git` on PATH; worktree/long-path tests may be sensitive to CRLF/path length |
| Tauri shell (`agenium-app`) | bundle targets in `tauri.conf.json` list `deb, appimage, app, dmg` — no `msi/nsis`; `.app`/`.dmg` are not built on Windows | `cargo build -p agenium-app` needs WebView2 SDK; `tauri build` produces nothing useful until a Windows target is added |
| UI (`app/ui`) | pure TS/React | `pnpm install/test/build` should pass unchanged |
| Paths | root default `%USERPROFILE%\.agenium` (`AGENIUM_HOME` overrides) | |
| CI / release | no Windows matrix, no signing | — |

Future path for real Windows support (not started): WSL2 or a container for the sandbox, or an experimental restricted-token
backend (see the architecture doc, section Sandbox, "Windows — deferred").

## 4. Reporting

Use the fixed format in `WINDOWS-PROMPT.md` ("REPORT FORMAT"). Send the whole report plus `target\ci-local\*.log` for any FAIL.
