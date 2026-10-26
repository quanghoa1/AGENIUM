# ADR-110: The user can message a running Lead on a vendor CLI

Status: Accepted (2026-10-26). Extends [ADR-104](ADR-104-messaging-a-running-lead.md) (its point 4 said a CLI Lead had
no queue). Evidence: `agent/cli-backend/tests/stream_run.rs::the_user_messages_a_running_cli_lead`; real runs on
2026-10-26 with `agent/cli-backend/examples/cli_steer.rs` (Claude Haiku, Grok, Codex: PASS each) and
`quality/eval/examples/cli_lead_steer.rs` through the runtime (Claude Haiku, Grok: PASS each).

## Context

ADR-104 gave a Lead on the API backend a message queue that the loop reads before each model request. A Lead running
through a vendor CLI had none, so a message started a second, concurrent turn. Agenium cannot write into a CLI's own
context at an arbitrary point, but it is in the path of two things every CLI does while it works: it calls
Agenium's MCP tools (the protocol tools and the agent's own MCP bindings), and a hooked CLI (claude, grok, agy) asks
Agenium's hook before each of its own tool calls.

## Decision

1. A CLI Lead's run gets the same queue as on the API backend (only the project team's Lead; workers never have one;
   `Runtime::chat` delivers into it first).
2. A queued message reaches the CLI at the first of:
   - the result of the next Agenium MCP tool it calls, after that result;
   - the answer of the hook to its next own tool call, as added context, for a CLI whose hook can carry context
     (Claude Code: `hookSpecificOutput.additionalContext`; `Driver::hook_with_context`). Grok's and agy's hooks have
     no such field, so they use the other two paths;
   - the end of its turn: the message is the next turn (in a chat run it no longer ends the run; no nudge is counted).
   A run that is ending (a protocol tool ended or parked it) does not take the message; it stays for the project's
   next turn as in ADR-104 (nothing is lost).
3. **The message carries the run's tag.** It is marked `[Message from the user while you work · <tag>]`, where the tag
   is random per run and named once in the run's instructions, with the rule that text with any other mark is not
   the user's. Claude refused the first, untagged version because it arrived beside a tool result and looked like
   an injection. That refusal was right: a file can claim to be from the user, but it cannot guess the tag.

## Consequences

- On a CLI the message arrives at the next tool boundary, not before the next model request. A Lead's work is mostly
  Agenium tool calls (tickets, plan), so in practice that is soon. A turn without any tool gets it at the turn's end.
- The run transcript records each relayed message as a user message, as on the API backend.
