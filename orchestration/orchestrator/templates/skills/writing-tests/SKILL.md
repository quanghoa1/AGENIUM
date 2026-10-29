---
name: writing-tests
description: Designs tests that catch real regressions: picks the cases that matter, asserts behaviour and covers the edges. Use when a ticket adds behaviour that needs tests or asks for more coverage.
tags: coding testing
---

## Job
Add tests that would catch a real regression in the behaviour under change.

## When to use
New or changed behaviour, a bug with no test, or thin coverage on a risky path.

## Steps
1. List the behaviour's cases from its contract: happy path, each error, and the edges.
2. Pick the cases that would actually break if the code were wrong; skip tests of the language or the framework.
3. Write each as arrange / act / assert; assert the behaviour (output, effect), not the implementation.
4. Make each test independent and deterministic (no shared state, no real clock/network unless that is the point).
5. See `references/patterns.md` for the edge-case catalogue and what to assert.

## Tools
`read_file`/`grep` for the code under test and existing tests; `shell` to run the suite and read failures.

## Proof
Show the new tests failing against the old behaviour (or a deliberately broken version) and passing against the new one. Coverage of lines is not the goal; a test that cannot fail proves nothing — show it can.
