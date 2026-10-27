---
name: doc-writing
description: Write or update user-facing docs for a change so a reader can use it without the code.
tags: coding docs
---

## Job
Write or update the user-facing docs for a change so a reader can use it without reading the code.

## When to use
A change adds or alters behaviour a user or developer relies on (a command, flag, API, config, UI).

## Steps
1. Find who reads this and what they need to do; write for that task, not the implementation.
2. Update the right place (README, reference, changelog) rather than adding a stray file.
3. Lead with what it does and a working example; then the details and edge cases.
4. Keep it accurate: every command, flag and path must be real and current.

## Tools
`read_file` and `grep` to find existing docs and confirm names/flags; `shell` to run any example you include and confirm it works.

## Proof
Show that the example you wrote actually runs (paste the command and its output), and name the file(s) you updated. Do not document a flag or path you have not verified exists.
