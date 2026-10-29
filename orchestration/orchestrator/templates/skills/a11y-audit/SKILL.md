---
name: a11y-audit
description: Checks a UI change against accessibility basics (keyboard, labels, contrast, roles) and fixes what fails. Use when a ticket adds or changes screens, forms or controls, or asks for an accessibility pass.
tags: coding frontend
---

## Job
Check a UI change against accessibility basics and fix what fails.

## When to use
A ticket adds or changes UI (components, forms, navigation, colour).

## Steps
Work through `references/checklist.md`: keyboard, names, roles/structure, colour/contrast, and motion.

## Tools
`read_file` for the markup/styles; `shell` for any a11y linter or contrast checker the project has; read its output.

## Proof
List each check as pass/fail with the element and the fix applied. For contrast, give the measured ratio. Do not mark a check passed you did not actually test.
