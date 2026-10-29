---
name: secure-coding
description: Reviews code defensively for input validation, authorization, secrets and injection-safe patterns. Use when a change handles user input, sign-in, secrets, files or database queries.
tags: coding security
---

## Job
Check a change against defensive secure-coding basics and fix what is unsafe. (Defensive only — not an attack tool.)

## When to use
A change handles input from outside, authentication/authorization, secrets, files, or builds a query/command.

## Steps
Work through `references/checklist.md`: validate input, parameterize, least privilege, handle secrets, fail safely.

## Tools
`grep` to find the sinks (query builders, shell calls, file paths, template rendering); `read_file` to trace the data from its source; `shell` for any security linter the project has.

## Proof
For each risky sink, show the input is validated or the call is parameterized, and name the source the data came from. State explicitly which inputs are trusted and why. Do not claim "safe" without showing the guard.
