# ADR-80: Push the merged work on Closed when the project has a remote

Status: Accepted (2026-10-25, M5; owner: "when the user adds a repo, push; otherwise keep it local").

## Decision

When a code ticket is closed and its work is merged, Agenium pushes the project's main branch to `origin` — but only
when the project repository has an `origin` remote (the user cloned or added a repo). A project without a remote stays
entirely local; nothing is pushed. The push is a journaled `Op::Push` enqueued on close and is best-effort and
outward-facing: on failure it records a System comment ("the work is merged and saved locally") and completes without
retrying or blocking the close. It uses git's own credentials (the remote the user set up), with no token handling in
Agenium.

## Consequences

- `Repo::remote_url(name)`; `Op::Push { branch }` in `effects`; `close_ticket` enqueues it when `origin` exists and
  the ticket merged.
- Tests: `vcs.rs::remote_url_detects_origin_and_push_lands`, `lifecycle.rs::a_closed_ticket_pushes_when_the_project_has_a_remote`.
