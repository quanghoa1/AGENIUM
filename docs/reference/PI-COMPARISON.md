# Agenium and the pi coding agent — what we learned (2026-10-26)

pi (`earendil-works/pi`, TypeScript, widely used) is a single-agent coding harness: a lean core with an
extension system, ~40 provider presets, JSONL session trees, steering queues and careful context compaction.
Agenium is a multi-agent system (Lead, workers, tickets, review, merge) with an OS sandbox, memory and its own
protocol. We read pi's design (`packages/coding-agent/docs`, `packages/agent`, `packages/ai`) and about 2 200
"Fixed" entries of its changelogs, to find what Agenium lacks and which bugs to avoid. The reference clone is
outside the repository (`D:\Work\references\pi`); nothing was copied, rules were learned and written anew.

## Where Agenium already goes further

| Area | pi | Agenium |
|---|---|---|
| Containment | "intentionally no sandbox"; container docs and opt-in examples | OS sandbox on Windows (MXC), Linux (bubblewrap), macOS (Seatbelt): 0 escapes on the containment matrix of each (ADR-53/55/95) |
| Multi-agent | none in core ("skips sub-agents"); a subagent example extension | Lead, planner, workers, reviews, merges, inbox, SLA (M4) |
| Permissions | none in core; example extensions | command gate, write scope, approvals, `request_permission` |
| Memory | session only | per-agent memory, lessons, shared pool, chat learning (ADR-51/87/98) |
| Work tracking | sessions | tickets with acceptance criteria, worktrees, pre-merge checks |

## Fixed today from pi's bug history

| pi bug class (how often it recurred) | Agenium before | Now |
|---|---|---|
| Poisoned sessions: dangling tool calls, results without calls, empty blocks, foreign thinking signatures, provider-incompatible tool-call ids (~30 + ~10 + ~150) | sent the stored history as it was | a pass over every request (ADR-101) |
| Tool calls in an answer cut at the output limit were run with truncated arguments | ran them | not run, the model is told to resend (ADR-101) |
| Thinking replayed to another model of the same vendor ("Invalid signature") | replayed to any model of the vendor | stamped with its model, replayed to it only (ADR-101) |
| Edit tool: CRLF, BOM, typographic quotes made "text not found"; a fuzzy match rewrote a whole file (~12) | exact bytes only | CRLF/BOM kept, loose match spliced into the original bytes (ADR-102) |
| Context overflow wording differs per vendor; overflow vs rate limit confused (~20) | an overflow failed the run | recognised per vendor, one recovery: clear or compact, send again (ADR-103) |
| Compaction: summaries cut at the output limit stored; big tool results sent before the threshold check; summary requests larger than the context (~45) | — (compaction was new, ADR-100) | notes must be whole and fit; new results count before the next request (ADR-103) |
| BOM written by PowerShell 5.1 broke config files (~15) | config.toml, templates, MCP JSON failed to parse | stripped on read |

Already right in Agenium, checked against pi's classes: a stream that ends without its terminal event is an error;
quota and billing errors are not retried; a command ended by a signal is reported as such; truncated command output is
saved whole to a file; the process tree is killed (job object, PID namespace, and now the stray-process sweep on
macOS); MCP tool names that collide after sanitising get a hash suffix; endpoints for OAuth must be HTTPS.

## Still missing — proposed, by value

1. **Steering a running agent** — done for the Lead and Solo agents (ADR-104; never workers, the owner's rule). pi lets the user send a message that the running agent reads at its next turn (and
   a follow-up queue for after it stops). In Agenium a user comment on a ticket reaches the agent only at its next
   run. Proposal: comments by the user (and the Lead) on a ticket whose run is active are injected before the next
   model request, as `[user, during your run]: …`.
2. **Lenient tool arguments** — done (ADR-105). pi coerces harmless shapes (a number sent as a string, one object where an array was
   expected, an array sent as a JSON string) and reports malformed JSON with what the model sent. Agenium's
   `deny_unknown_fields` + strict types turn these into `invalid_arguments`, which costs a step and counts towards
   the three-errors stop.
3. **Parallel read-only tools** — done (ADR-105). pi runs a turn's tool calls in parallel by default (results in call order; writes
   to the same file serialised). Agenium runs them one by one; read-only calls (read, grep, list, glob, fetch) could
   run together.
4. **Retry-After cap.** pi caps a provider-requested delay (60 s). Agenium waits the full `retry-after` (bounded only
   by the total retry budget).
5. **Per-provider fixtures for usage accounting** (pi: reasoning or cached tokens counted twice, cache writes counted
   as input, ~25 entries): a golden test per adapter with real usage payloads.
6. **Compaction quality:** pi's summary format (Goal / Constraints / Progress / Decisions / Next steps / Critical
   context, plus the files read and modified, carried across compactions) and an update of the previous summary on
   a second compaction instead of a fresh one.
7. **OAuth issuer check (RFC 9207)** on the authorization response, and treating an empty `scope` as absent.
8. **A very long Lead chat** reloads its whole history each turn; compaction of the chat session itself.

Not proposed: session trees and `/fork` (Agenium's unit of work is the ticket and its git worktree), extensions and
codemode (out of scope now), prompt-cache warming (cost model differs: runs are short and scheduled).
