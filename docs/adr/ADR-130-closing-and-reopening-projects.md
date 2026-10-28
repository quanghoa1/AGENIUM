# ADR-130: The user closes a done project, its team goes free; reopening picks the team, or waits for it

Status: Accepted (2026-10-28). Owner: "chỗ để người dùng duyệt dự án và close nó, để assign AI solo hay team AI hiện
tại qua dự án khác là chỗ nào nhỉ, vì dự án cần close thì team hay AI solo hiện mới trở lại pool được"; "close project
chỉ có người dùng được đóng, chứ agent không được đóng"; "project đã close thì không hiện nút add a team trên thẻ của
nó, trừ khi người dùng reopen lại"; "nhưng nếu team cũ đang bận làm project khác thì phải đợi"; "button close project
chỉ hiện khi tất cả các ticket đã closed". Amends ADR-63 (the team works from the agreed spec to the end: a done
project stayed done until new work came, with its team bound to it).

## Context

A project whose plan was done showed "Project done" and nothing more. Its team stayed bound to it (one team per
project, decision 25): moving the team to another project meant finding the project selector on the team's card in
Teams, and the old project never ended; it could flip back to running whenever someone asked its Lead for more. Nothing
told the user that the work was done, and the Lead's chat did not lead to where the team worked.

## Decision

1. **Only the user closes a project, and only a done one** (every plan item done, no ticket open, no spec or plan
   waiting) **while none of its team's runs is going** (the Lead answering in the chat marks no one Working, so the
   runs are what is checked): from the header next to "Project done", from the project's card, or from the Lead's chat.
   A confirmation shows what was done (tickets, plan items) first. No agent tool closes or reopens a project. Asked
   while work is open or a run is going (a race, the CLI), it is refused and nothing changes.
2. **Closing ends the project and sets its team free, whole** (members and Lead stay together): nothing runs on a
   closed project, its card and the header say "Project closed", and the Teams page opens to give the team another
   project. The mark is a project setting (`closed_at`; the stored status stays Completed, so no table is rebuilt), and
   the team that worked on it is remembered (`closed_team`).
3. **Reopening is the user's own step**: a closed project offers Reopen, not "Add a team", and the Teams page does not
   offer it to a team. Reopening asks who takes it:
   - the team that worked on it, preselected when it is free: its agents' memory knows the project (its files,
     conventions, how its tests run), where a new team starts fresh;
   - **the same team once it is free**, when it is busy on another project (`waits_for_team`): the project shows
     "Waiting for <team>" and gets the team as soon as it is set free (its project closed, or freed in Teams);
   - any other free team now (work that needs other skills), or nobody yet.
4. **The work is told and linked**: the team starting (after the spec is agreed, or new work on a done project) and a
   project getting done are toasts with the way there ("Watch the team", "Look it over"); a project getting done is
   also a desktop notification. The Lead's chat shows where the work is: while the team works, buttons to the live
   office, the tickets and the runs; once done, the tickets and Close project. Ticket keys in the Lead's messages open
   the ticket. A closed project's chat still shows its last conversation (with its last team's Lead).

## Consequences

- One project at a time per team stays the rule; a team leaves a project only when nothing is open there.
- A team set free while a project waits for it is taken at once; the user is told where it went.
- A team dissolved while a project waits for it leaves the project waiting for nobody: it shows no wait and offers a
  team as usual.

## Verified (2026-10-28)

- Edge cases in the running app: closing confirmed while the Lead answered in the chat was **accepted at first** (the
  check looked only at agents marked Working, which a chat turn does not set): the team left mid-turn. Fixed: any run
  of the team still going refuses the close ("<agent> is at work on the project; wait until its run ends"). A double
  close closes once and refuses the other; a closed project's chat refuses a message; a closed project takes a team only
  by reopening; reopening with a team busy elsewhere is refused; a ticket of another project opens as that project's
  ticket and its follow-up goes to that project's Lead (it went to the selected project's before); a project from a
  repository that does not exist is a clear error and leaves nothing behind.

- Tests (`tests/lifecycle/close.rs`): refused while a ticket runs and while one is resolved but not closed; closing
  frees the team whole; a closed project takes no team until reopened; reopening offers and restores the team; a
  project reopened to wait for its busy team gets it when that team is freed.
- In the desktop app on a real root (DeepSeek Solo agent): a project cloned from an empty GitHub repository was given
  the freed team, built a two-ticket request and pushed `main` to GitHub on close (a fresh clone's tests pass); it was
  closed from the header, the team went back to the first project which had been reopened to wait for it ("went on to
  uidemo, which was waiting for it"); a request in the chat raised "The team started" after 11 s and "is done" after
  72 s; the full UI check passes (57/57).
