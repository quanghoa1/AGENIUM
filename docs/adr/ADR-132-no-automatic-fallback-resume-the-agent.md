# ADR-132: No automatic move to another model; the user resumes the agent

Status: Accepted (2026-10-29). Owner: "agenium không tự chuyển sang model api, cái đó người dùng tự chuyển và resume lại,
đã có nút resume cho từng agent chưa, tôi nghĩ vậy ổn hơn thay vì nhắn lead"; "chúng ta chỉ thay đổi bộ nào chứ không
thay agent mà nhỉ, tức thay vendor AI thôi". Amends ADR-56 (6: API fallback), ADR-131 (3: the Lead hands a set-aside
worker's tickets on) and decision 3j.

## Context

- **A CLI that could not run moved the work to an API model by itself.** With `[cli] fallback_provider/fallback_model`
  set, a CLI agent whose CLI was missing, too old or logged out ran on that API model instead, on another vendor's
  quota and price, without the user choosing it.
- **A worker out of quota handed its ticket to another agent** (ADR-131, 3): the Lead was told of its tickets and
  assigned them to another member, who started without the worker's conversation (only the comments and the
  worktree). The owner wants the agent kept: only its model (its vendor) changes, and the agent goes on.
- **Set back to work by hand, from one place only.** An agent set aside (Offline, QuotaExhausted) went back to work
  with a new model, or by "Set to Idle" in its own page. Needs you said to change the model, with no step there.

## Decision

1. **Agenium never moves an agent's work to another model.** A CLI that cannot run (missing, too old, logged out, no
   sandbox for Codex) sets the agent aside (Offline) for the user: the ticket gets a System comment, Needs you says
   "`<agent>` cannot use `<CLI>`: `<why>`. Fix it, or give `<agent>` another model, then resume it: it goes on from
   where it stopped." A Lead or planner on such a CLI is set aside the same way. No API request is made.
2. **`[cli] fallback_provider/fallback_model` is only the API model that draws a CLI agent's lessons** (memory
   extraction is an API call). Settings says so: "API model for CLI agents' memory (lessons are drawn with it; their
   work never moves to it)".
3. **Quota used up mid-ticket keeps the ticket with its agent**, in a team too: the Lead is not asked to hand it on.
   ADR-131 (3) is withdrawn: `assign_ticket` takes New or Reopened tickets again, and the Lead's inbox no longer lists
   Assigned or InProgress tickets of a worker set aside. The user gives the agent another model (another vendor's API
   model or CLI, or the same one once its quota is back), which frees it (ADR-131, 2), and it goes on from its
   conversation: the earlier runs' messages go to the new model (an API model gets them as messages, a vendor CLI as
   its first prompt), the worktree holds its changes.
4. **A Resume button wherever an agent set aside is shown**: on the agent's card in Agents, on its page (with what
   resuming does), and on the Needs you item that says why it stopped (with a link to the agent's page to change its
   model). Resume sets the agent back to Idle; its tickets go on at the next round.
5. A Needs you item names the agent's model: "ran out of quota or credit on `<model>`: add credit or give it another
   model, then resume it", or "cannot use `<model>` (its provider unreachable, or its CLI missing or logged out)".

## Consequences

- The user decides every change of vendor and its cost. A CLI agent whose CLI is gone stops; it is one click to go on
  once fixed, or one model change.
- A ticket's work is never split between two agents by Agenium; its conversation stays whole.
- Not done: a vendor's subscription window that resets does not resume the agent by itself (Resume is the user's
  step).

## Verified (2026-10-29)

Tests:
- `a_cli_agent_whose_cli_cannot_run_waits_for_the_user` (a CLI agent on `devin`, which cannot be driven: Offline, Needs
  you says why, no API request; a new backend frees it);
- `a_lead_or_planner_whose_cli_cannot_run_waits_for_the_user`, `a_run_that_cannot_go_on_leaves_nothing_open`;
- `quota_used_up_mid_work_goes_on_with_another_model`.

Real models: `agenium-eval --quota same|team` (DeepSeek through OpenCode Go, and a vendor CLI as the new model with
`--worker-cli`): the ticket stays with the worker whose quota ran out, the worker goes on with its new model and the
golden task's check passes after the merge. See [VERIFICATION](../VERIFICATION.md).
