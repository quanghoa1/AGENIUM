---
name: commit-and-pr
description: Writes a clear commit and pull request that explain what changed and why, scoped to one logical change. Use when committing work or opening a pull request.
tags: coding
---

## Job
Turn a finished change into a clear commit (and PR description) a reviewer can follow.

## When to use
Before committing work or opening a pull request.

## Steps
1. Keep one logical change per commit; split unrelated changes.
2. Subject: imperative mood, <= 72 characters, says what the change does (not "fixed stuff").
3. Body: why the change is needed and anything non-obvious; wrap at ~72.
4. PR: what and why, how you verified it, and any risk or follow-up.
5. Check the message with `scripts/check_commit.py` before committing.

## Tools
`shell` to run `git diff --staged` and `python scripts/check_commit.py <file>`; `read_file` for the diff.

## Proof
Show the final message and the output of `check_commit.py` with no warnings (or a stated reason for each you keep). Match the repo's existing commit style.
