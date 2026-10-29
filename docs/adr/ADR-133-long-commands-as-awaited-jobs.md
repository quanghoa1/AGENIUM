# ADR-133: A vendor CLI's long command goes on as a job the run waits for

Status: Accepted (2026-10-29). Owner: "không cho nó chạy qua tool của agenium được sao, tức nó chỉ gửi lệnh và idle, còn
agenium đảm nhiệm việc xử lí timeout và gửi phản hồi kết quả tool cho nó". Amends ADR-129 (commands through Agenium's
`shell`) and ADR-56 ("Reviewed again": tool waits).

## Context

- A vendor CLI waits only so long for one MCP tool call. Claude Code gave up before 80 s; Codex has
  `tool_timeout_sec` (60 s by default); Grok `tool_timeout_sec` (6000 s). Agenium sets one hour where it can
  (`MCP_CALL_TIMEOUT`), but agy and OpenCode have no setting: they waited for an 80-second command, and nothing says
  they wait for a 5-minute build or for the user's approval of a command (up to 30 minutes in the app).
- A CLI that gave up called the command failed: the model started it again (the check ran twice) or asked the user.

## Decision

1. **In a vendor CLI's run, `shell` waits at most `FOREGROUND_LIMIT` (45 s, under every CLI's own wait).** The command
   runs as a job the run waits for (`Jobs::start_awaited`): ended within the limit, the call answers as a plain
   `shell` call. Still running, or still waiting for the user's approval, the call answers that it goes on as job `N`
   (with its time limit), that its result comes as a message when the agent's turn is over, and that it must not be
   started again; `shell_output` shows what it printed so far and `shell_stop` ends it. An API run is not affected:
   it waits for the command as before.
2. **The run delivers the end.** When the CLI's turn ends, the run waits for its awaited jobs (the run's deadline and
   the user's stop still hold) and sends the next turn "Your command `…` (job N) has ended: exit code …, its output",
   or that it was not run (refused, with how to ask), or stopped. A job whose end the agent already read with
   `shell_output` is not told again.
3. **The report waits for the commands.** `submit_report` is refused (`commands_running`) while an awaited job goes on:
   a report cannot rest on a check that has not ended.
4. The command's approval (the run's command gate) is asked inside the job: a wait for the user past the limit does not
   hold the CLI's tool call either. The user's stop ends the job and the wait (ADR-131).
5. Awaited jobs do not count against the background-command limit (they are the agent's foreground work).

## Consequences

- A command of any length up to its time limit (10 minutes at most) and an approval of any length reach the agent,
  whatever the CLI's own tool wait.
- The model sees the end as a message, the way a person is told a build finished; one that polls with
  `shell_output` gets it there instead (Claude Haiku and Codex did in the real runs).

## Verified (2026-10-29)

Tests:
- `a_cli_runs_long_commands_as_jobs_it_is_told_the_end_of` (tools: the limit, the job, the message once, an approval
  past the limit, a refused one);
- `a_long_command_goes_on_as_a_job_and_its_end_comes_as_a_message` (a CLI run: the fake CLI ends its turn, the run
  waits, the next turn carries the end; `submit_report` refused meanwhile);
- `the_users_stop_ends_a_long_command_and_a_wait_for_approval`.

Real CLIs: `agenium-eval --task slow-check` (an 80-second check that must run once) passed on Claude Code (Haiku), Codex
(gpt-6-luna), Grok (grok-4.7), agy (Gemini 3.8 Flash) and OpenCode (DeepSeek v4.1 Flash): the `shell` call answered
after 45 s that the check goes on as job 1, the check ran once and its number was written. See
[VERIFICATION](../VERIFICATION.md).
