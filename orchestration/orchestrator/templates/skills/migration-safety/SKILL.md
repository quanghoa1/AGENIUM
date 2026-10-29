---
name: migration-safety
description: Changes a schema or stored data safely: reversible, idempotent, and deployable without downtime. Use when a ticket alters a database schema, migrates data or changes a stored format.
tags: coding data
---

## Job
Make a schema or data migration that is safe to run and safe to roll back.

## When to use
A change alters a database schema, a stored format, or does a bulk data change.

## Steps
Work through `references/checklist.md`: backward compatibility, reversibility, idempotency, batching, and the deploy order.

## Tools
`read_file` for the migration and the code that reads/writes the data; `shell` to run the migration up and down against a copy and read the result.

## Proof
Show the migration run forward and then rolled back on a copy with data, and that the app works against both the old and new shape during the transition. A migration you cannot roll back is a risk you must call out explicitly.
