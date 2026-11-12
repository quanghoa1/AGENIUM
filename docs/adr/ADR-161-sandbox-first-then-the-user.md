# ADR-161: Sandbox first, then the user — a blocked command can run outside the sandbox when allowed

Status: Accepted (2026-11-12; owner: "liệu có thể ask approve từ người dùng, thay vì chặn không? cũng như có 1 nút auto
approve cho người dùng, trường hợp họ không muốn tự click approve"; and, on a restricted push tool: "chúng ta tập trung
điều phối, chứ không phải giới hạn agent"). Extends ADR-64, ADR-117, ADR-160.

## Context

- Under the default policy a command runs in the sandbox without asking. One the sandbox blocks failed, and the agent
  only saw the error: docker, a server on 127.0.0.1, `git push` with the user's credentials, a file outside the
  worktree. The macOS tool probe (ADR-160, kits v8–v10) lists them.
- The only way past was turning the sandbox off for every command (ADR-160).
- Codex works the same way: in the sandbox first, and a command that needs more is offered to the user to run outside
  ("on-request"), or run outside at once in full-auto.

## Decision

1. **A command the sandbox blocks goes to the user.** `agenium_sandbox::blocked_by_sandbox` reads a failed command's
   output for the refusal (Seatbelt's `Operation not permitted`, bubblewrap's `Read-only file system`, a nested
   `sandbox_apply`, Windows `Access is denied` / `Access to the path … is denied`). The `shell` tool then asks the gate
   (`CommandGate::escalate`) whether it may run again outside the sandbox, directly on the host with the same clean
   environment (no keys). Allowed: it runs again and the agent gets that output, told it ran outside. Refused: the
   agent gets the sandbox's output and the user's reason. A command that worked is never asked about.
2. **The agent can ask up front.** `shell`'s `outside_sandbox` (the reason) sends a command that cannot work inside
   (credentials, docker, localhost) to the gate before it runs; refused, it does not run. Background commands leave
   the sandbox only this way.
3. **The gate** (`RunGate::escalate`): under "sandbox only, never ask" nothing leaves; an entry `outside:<command>`
   the user gave ("Always for this project") lets it out; `[sandbox] auto_escalate` (Settings: "Approve commands the
   sandbox blocks by myself") lets it out without asking; otherwise the user decides in Needs you, where the card says
   "Outside the sandbox" and why.
4. **Every command that ran outside is in the audit log** (`outside_sandbox`, from the run logs).
5. **Pushing stays orchestration.** Agents are told Agenium pushes after the review and the merge; no special push
   tool. A `git push` an agent runs anyway goes through 1–3 like any command.

What does not change: commands still start in the sandbox; installing software on the computer still asks (ADR-120);
secrets never reach commands; "Turn the sandbox off" (ADR-160) stays for machines where the sandbox gets in the way.

## Consequences

- `agenium_sandbox::blocked_by_sandbox`; `agenium_tools::CommandGate::escalate` (refused by default),
  `jobs::escalate`; `shell`'s `outside_sandbox`; `Approver::approve_escalation`; `SandboxSection.auto_escalate`;
  desktop `set_auto_escalate`, `CommandApproval.outside`; the run context says how a blocked command is handled.
- A command can run twice (blocked partway in the sandbox, then outside). Builds, tests and installs do not mind;
  for one that must not run twice the agent asks up front.
- Tests: `agent/tools/tests/escalate.rs` (real sandbox: blocked then allowed, refused, asked up front, nothing asked
  for a command that works), `orchestrator/tests/runtime/escalate.rs` (the user, always-allow, automatic, never),
  `blocked.rs` unit tests.

## Verified (2026-11-12)

Windows (MXC) and the Ubuntu VM (bubblewrap): the escalation tests ran on the real sandbox, not skipped. macOS
(Seatbelt, Devin's macOS 26.6 M4, kit v11, `docs/reference/bench-raw/macos-2026-11-12-run11`): the 4 tool tests, the
gate test and the detection unit tests pass on real Seatbelt; 750 tests, golden 5/5 with the sandbox on and off, 0
containment escapes, the 44-command probe unchanged from v10.
