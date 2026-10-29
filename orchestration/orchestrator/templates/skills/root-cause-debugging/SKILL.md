---
name: root-cause-debugging
description: Finds and fixes the cause of a bug, not the symptom: reproduce, isolate, fix, and prove it with a regression test. Use when a ticket reports a bug, a crash or a failing test.
tags: coding debugging
---

## Job
Fix the cause of a bug, not its symptom.

## When to use
A ticket reports a defect, a test fails, or behaviour is wrong.

## Steps
1. Reproduce first: find the smallest input or steps that trigger it, and capture the exact wrong output.
2. Isolate: narrow to the function or line responsible (bisect, add a failing assertion, read the call path). Do not guess.
3. Explain the cause in one sentence before changing code.
4. Fix the cause. Avoid sleeps, broad try/except, or disabling the test — those hide the bug.
5. Add a regression test that fails before the fix and passes after.

## Tools
`grep` and `read_file` to trace the path; `shell` to run the reproducer and the test; read the real output each time.

## Proof
Show the reproducer, the one-sentence cause, and the regression test passing after and failing before the fix (paste both runs). "Could not reproduce" is a valid, honest outcome — say so instead of changing unrelated code.
