---
name: release-readiness
description: Decides whether a change is ready to ship: tests, docs, migration, rollback and a clean tree. Use before a release, a merge to the main branch or closing a project.
tags: coding
---

## Job
Decide, with evidence, whether a change is ready to release — or say what blocks it.

## When to use
Before merging to the release branch or cutting a release.

## Steps
1. Run `scripts/preflight.py` for the mechanical checks (clean tree, branch).
2. Work through `references/checklist.md`: tests green, docs updated, migration reversible, rollback known, config/flags set.
3. Decide GO or NO-GO; if NO-GO, list exactly what blocks it.

## Tools
`shell` to run the test suite and `python scripts/preflight.py`; `read_file` for the changelog and config.

## Proof
Show the preflight output and the test run, and a one-line GO/NO-GO with the evidence for each checklist item. A GO with unproven items is not a GO.
