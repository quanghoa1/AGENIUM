# Migration safety checklist

## Backward compatible (expand / contract)
- Add first: new columns nullable or with a default; new tables unused yet.
- Deploy code that writes both old and new, reads new with a fallback.
- Backfill in batches.
- Only after everything uses the new shape, contract (drop old) in a later migration.

## Reversible
- Every migration has a tested down step, or a written reason it cannot be reversed.

## Idempotent
- Re-running the migration does not error or double-apply (guard with existence checks).

## Safe to run
- No long table lock on a hot table; batch large updates; add indexes concurrently where supported.
- Wrap in a transaction only where the engine allows DDL in one.

## Data
- Back up (or snapshot) before a destructive change.
- Validate row counts and a sample before and after.
