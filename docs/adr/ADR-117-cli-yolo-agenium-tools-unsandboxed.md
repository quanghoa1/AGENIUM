# ADR-117: Vendor CLIs work through Agenium's tools; a machine without a sandbox may run unsandboxed by choice

Status: Accepted (2026-10-27; owner: "hướng 2, và các cli phải chạy với option yolo để không phải hỏi quyền và hướng
chúng vào dùng tool call của chúng ta thay vì dùng tool của chúng"). Amends ADR-64 (command gate), ADR-56 (CLI
backend) and ADR-116 §1.

## Context

- A clean Windows 11 23H2 machine cannot create MXC process containers (release check, 2026-10-27): there every agent
  command waited for the user, and Codex, which runs only inside Agenium's sandbox, could not run at all.
- A CLI checked through its hook (Claude Code, Grok, agy, OpenCode) ran its own shell on the host; each command went to
  the user. ADR-116 added Agenium's `shell` beside it where a sandbox works, but the CLI's own shell still asked.
- The owner wants the CLIs in YOLO mode — nothing asks the user — and working through Agenium's tools.

## Decision

1. **Hooked CLIs work through Agenium's tools.** They run as before in their own YOLO mode with Agenium's hook, and get
   the run's tool set over MCP — in a work run `read_file`, `list_dir`, `glob`, `grep`, `edit_file`, `multi_edit`,
   `apply_patch`, `write_file`, `shell`, `todo`, `fetch`, `view_image`; in a review or chat the read-only ones plus
   `shell` where a sandbox makes that run read-only. Their instructions say to work through these: they keep the
   ticket's write scope and run commands under Agenium's rules.
2. **A hooked CLI's own shell is off.** The hook refuses it at once, naming Agenium's `shell`; it is never put to the
   user. A command on the project allowlist still runs. Its own file tools keep working inside the worktree (they are
   checked by the hook as before), but the instructions prefer Agenium's.
3. **Running without a sandbox is the user's choice** (`[sandbox] unsandboxed`, Settings: shown only where no sandbox
   works, with a warning to confirm). On: commands run directly in the ticket's worktree without asking (Codex's
   full-access level), and a CLI that runs contained elsewhere (Codex) runs on the host in its YOLO mode, told that
   no sandbox protects it. Off (the default): each command waits for approval, as before. Where a sandbox works the
   setting has no effect.

What does not change: secrets never reach agents; write scope is checked at `submit_report`; protocol tools, the
review and the merge queue are the same; YOLO (`never`) without a sandbox stays refused unless the user opted in.

## Consequences

- `SandboxSection.unsandboxed`; `RunGate` allows contained commands without a sandbox when it is on and refuses a
  hooked CLI's own commands with the redirect; `CliRun.uncontained`; `agenium_cli_backend::run::Commands` (what the CLI
  is told); runtime offers the run's tools to hooked CLIs and `shell` in their read-only runs.
- Desktop: `set_unsandboxed`, the Settings checkbox with a confirmation.
- Tests: `runtime.rs::sandboxed_commands_run_without_asking`, `run.rs::each_cli_is_told_how_its_commands_run`.
- Risk accepted by the owner: on an opted-in machine an agent's command can touch anything the user can.

## Verified on real models, and what that run fixed (2026-10-27)

A second request to the mixed team (Claude Opus Lead; Grok, agy, Codex, DeepSeek) went through spec v2, plan v2 and 3
tickets, all merged; the app's tests pass. Grok and agy worked only through Agenium's tools (`read_file`,
`edit_file`, `shell`: 0 refusals, nobody asked); Claude's own shell was refused 3 times and it moved to `shell`.
It also showed:

- **Tool outputs were committed.** `shell` keeps long outputs in `artifacts/tool-output/`; the auto-commit took them
  and the Lead reopened the ticket. Now they are in `LOCAL_EXCLUDES`, a commit untracks any committed earlier, and the
  merge queue moves them aside while it rebases (a replayed old commit would otherwise fail over the copy on disk).
- **A Lead retyping a long plan.** `revise_plan` refused a done item whose text the Lead retyped, without saying what
  differed; its run gave up after four tries. A done item that differs only in case, spacing or punctuation now gets
  its stored text back; one with other words is refused with the stored title and acceptance to copy.
- **Deleting inside MXC.** PowerShell's `Remove-Item` and cmd's `del` fail with "Access is denied" inside an MXC
  container while `[IO.File]::Delete`, `[IO.Directory]::Delete` and Python's `os.remove` work (reproduced with
  `cargo run -p agenium-sandbox --example contained`); the shell line of the work order says so.

Tests: `vcs.rs::generated_files_stay_out_of_commits` (tool outputs, untracking, the rebase),
`runtime.rs::solo_agent_takes_a_request_to_a_closed_merged_ticket` (retyped and changed done items).
