# ADR-122: In a team the user answers the Lead, never a worker

Status: Accepted (2026-10-28; owner, on the worker's feed showing the user's answer: "không được, phải thông qua lead,
lead sẽ comment lên ticket và ticket đó sẽ được hệ thống gửi tới worker, chứ không có chuyện lead hay user nói chuyện
trực tiếp với worker").

## Context

A worker's question goes to the Lead; the Lead answers it or escalates it to the user (ADR-120). The user's answer to an
escalated question was written on the ticket as the user's own Answer, so the worker read the user directly — against
the rule that the user talks to the Lead (or a Solo agent) only, and that workers get work through tickets and the
Lead's plan.

## Decision

1. **A Solo agent** is the one the user talks to: the answer goes on its ticket, as before.
2. **In a team** the answer becomes a message to the Lead, with the question it answers and the question's id
   (`route_user_answer` → `UserAnswer::ToLead`). The Lead's turn runs in the background (desktop) or in the command
   (CLI); the Lead answers on the ticket, and the worker hears it from the Lead. Nothing of the user's lands on the
   ticket. The window says "Sent to the Lead".
3. **Decisions stay shared** (ADR-120): the Lead's answer to a question it escalated to the user counts as the user's
   decision for the next tickets.
4. Also from the same review:
   - The Lead's delegations in the Live tab are a one-line bar over the chat (tickets, in progress, closed); opened, the
     list covers the chat until folded or Esc.
   - A long paste in a chat input shows as one label ("[Pasted #1 · 60 lines · 2.3 KB]") and is put back on send;
     Backspace after a label removes it; a long message of the user's in the history is folded with "Show all".
   - Projects show their run state in colour like tickets (running glows green, waiting for the user amber, paused
     amber, done blue), on the Projects page, the Overview and the team cards.
   - A team works one project at a time (it moves only when its project has no open ticket, decision 25): the team card
     shows the project it works on with its state, the projects it worked on before (from `TeamProjectChanged`), and the
     project selector is locked while tickets are open, saying why.
   - Agent cards show how many skills and MCP servers each agent uses, with the MCP servers named.
   - Dialogs and the palette open Apple-style (grow from 94 % with a soft spring, the page behind dims and blurs) and
     are rendered on `<body>`: a route's animated filter had made `fixed` cover only that route's box.
5. **Sandbox (Windows MXC): parents are listable.** Node resolves every `require` with `fs.realpathSync`, which reads
   the attributes of each folder from the drive root down (`EPERM: lstat 'C:\'` stopped `node --test` in a team run).
   The parents of the folders a command may use are given as `enumerate_paths` where Windows supports it: listable,
   never readable. A test runs Node in the sandbox and checks a file beside the worktree stays unreadable.

## Verified

- `route_user_answer` in a team returns a message to the Lead naming the ticket, the question id and the answer, and
  adds no comment to the ticket; the Lead's answer resolves the question and appears among the user's decisions.
- Node in the sandbox: `require('./m.js')` works; reading a file next to the worktree is denied.
- Workspace tests 444/444, desktop tests, clippy, UI smoke all pass; the screens checked on a root with six teams.

## Consequences

- `engine.rs` (`route_user_answer`, `UserAnswer`), `comments::user_answers`, desktop `answer`, CLI `answer`.
- `security/sandbox/src/mxc.rs` (`enumerate_paths`).
- UI: `ChatPane.tsx`, `Live.tsx`, `Organize.tsx`, `Overview.tsx`, `Shell.tsx`, `ui.tsx`, `motion.tsx`, `Needs.tsx`,
  `Ticket.tsx`; desktop `teams` returns `past_projects`.

## Amendment (2026-11-04, no comment box on a ticket)

The ticket popup still had a "Comment to steer the work" box, and `agenium ticket comment` the same: the user writing
on a ticket a worker reads, which is telling the worker directly. Both are gone (desktop `comment`, `api.comment`, the
CLI subcommand). A new request or a correction goes to the Lead or Solo agent in the chat; it writes to the ticket, in
English. The user's decisions on a ticket stay (close, reopen with feedback per criterion, waive, cancel), and
`App::user_comment` stays for the engine's own tests.
