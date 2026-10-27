---
name: refactoring
description: Restructure code without changing its behaviour: small steps, green tests after each.
tags: coding
---

## Job
Improve the structure of code while keeping its behaviour exactly the same.

## When to use
Code is hard to change or read and you need to change it; never mix a refactor with a behaviour change in one step.

## Steps
1. Make sure there are tests that pin the current behaviour; add characterization tests if not.
2. Refactor in small steps (see `references/catalog.md`); run the tests green after each step.
3. Keep behaviour identical — same inputs, same outputs and effects.
4. Commit the refactor separately from any feature or fix.

## Tools
`grep` to find all callers before you move or rename; `read_file` to understand; `shell` to run tests after each step.

## Proof
Show the tests green before and after, and state that no behaviour changed (no new or changed test assertions). If a test had to change, it was not a pure refactor — say so.
