# ADR-164: Long commands go on in API runs, a report waits for the checklist, done is proven

Status: Accepted (2026-11-18). Owner: chose ideas 1, 2 and 5 of the jcode study (a harness that moves a timed-out
command to the background, keeps a self-assessed todo list and nudges at the end of a turn).

## Context

1. **An API run killed its long commands.** `shell` ended a command at its timeout (120 s by default, the agent may ask
   for up to 600 s), process tree and all. A test suite or build that took longer never gave the passing check that a
   report cites (ADR-147, ADR-148): the agent ran it again with a longer timeout, cited a partial run, or listed it as
   not run. A vendor CLI's run already had the answer (ADR-133): a command still going after 45 s goes on as a job
   the run waits for, and its end reaches the agent as a message.
2. **A report could leave the agent's own checklist behind.** `todo` keeps the agent's plan for the run; nothing looked
   at it when the agent reported, so a step it had planned (often the tests) could be dropped without a word.
3. **A Lead could round a requirement up to done.** The review rules ask for evidence per ticket (ADR-162); what the
   Lead tells the user at the end had no such rule.

## Decision

1. **In an API run a command outlives its timeout as a job.** `ToolCtx::awaits_commands` (set for every API run; a
   CLI's run keeps `foreground_limit`) makes `shell` start the command as an awaited job (ADR-133's path): the call
   answers at the agent's timeout as before if the command ended; otherwise it answers that the command goes on as job
   N, with its time limit. The command then runs up to `shell_timeout_max` (10 minutes), not further. The loop hands
   the end of such a job to the agent as a message before its next step; an agent that ends its turn while one goes on
   waits for it (its end is the next message, not a nudge). The user's stop ends the command and the wait at once;
   the run's end stops every job (`Drop for Jobs`). A command that hangs therefore costs at most its 10 minutes, and
   the agent can end it sooner with `shell_stop`. Background commands (`background: true`, up to an hour) are as
   before.
2. **A report waits for the run's commands and the agent's checklist**, in both kinds of run, through one check
   (`agenium_tools::submit_refusal`, used by the API loop and the CLI bridge). A command still going refuses the
   report (as ADR-133 did for a CLI). A checklist with items not done refuses it with the list of those items, at most
   twice in a run (`TODO_REFUSALS`); then the report goes through as it is, the refusals in the run's log. The agent
   finishes the items or updates the checklist (drops what is no longer needed and says why in the report). The same
   report sent a third time with nothing in between stops as a loop (decision 17), as after ADR-162's refusals.
3. **Done is proven to the user.** A Lead/Solo rule: telling the user the work or a part of it is done names for each
   requirement what shows it is met; a requirement whose evidence is uncertain, indirect, from before the last change,
   or missing is not done, and the Lead says what is missing.

## Not taken from jcode

Self-scored memories, a shared checkout for parallel agents, self-modifying code reloaded without a gate, uploading
transcripts, and a gate that classifies commands as destructive (the sandbox bounds the damage; ADR-161). Measured
criteria for optimisation tickets and the classification of prompt-cache misses by cause wait until a case needs them.

## Consequences

- A slow test suite in an API run gives its result instead of a kill; the check-to-tree tie (ADR-148) applies when it
  ends.
- An API run that waits for a command shows `waiting` in its log; the step count does not grow while it waits.
- A run that parks (a blocking question) while a job goes on stops the job with the run (its tool context ends).
- Tests: `agent/loop/tests/loop/commands.rs` (a command past its timeout goes on, a report meanwhile is refused, the
  end replaces the nudge; a checklist refuses twice then lets the report through), the prompt rule in
  `agent/loop/src/prompt.rs`, and `quality/eval/examples/long_command_real.rs`: a Solo ticket whose check outlasts
  the timeout, through the orchestrator, the sandbox and a real command. `--scripted` (the model scripted) passed on
  Windows 11 and Ubuntu 25.10 on 2026-11-18; the run on a real API model waits for a working key (both OpenCode keys
  answered 401 that day).
- Known and unchanged: a run outside the sandbox on Linux ends its command's process group, so a daemon that left it
  with `setsid` outlives the run (inside bubblewrap the PID namespace ends it; macOS looks for strays by token).
