---
name: code-review
description: Review a change for correctness, safety and clarity before it is approved; report issues with evidence.
tags: coding
---

## Job
Review one change and decide whether it is correct, safe and clear enough to approve.

## When to use
Before approving a ticket's work, or when asked to review a diff or a branch.

## Steps
1. Read the ticket goal and acceptance criteria; review against them, not your taste.
2. Read the diff in full. For each hunk: does it do what the ticket asked, and nothing it should not?
3. Check the risky edges with `references/checklist.md`.
4. Confirm tests exist for the new behaviour and would fail without the change.
5. Prefer the smallest correct change; note reuse or simplification only when it clearly helps.

## Tools
`read_file` and `grep` for the changed files and their callers; run the project's tests and linter through `shell` and read the output — do not assume it passed.

## Proof
List each issue as `file:line` with a one-line reason and, where you can, the concrete input that breaks it. State which acceptance criteria you verified and how (which test, which command output). A review with no evidence is not a review.
