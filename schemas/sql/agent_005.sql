-- agent.db migration 5 (ADR-118): whether a lesson was shown to the run as a rule of its context (its scope matched the
-- work exactly), not as a hint from another context. Only a lesson shown as a rule and found by the work's own words
-- counts a validated context when the run closes cleanly: silence after showing a lesson is not confirmation.
-- Rows written before this migration read 0 (unknown), so they validate nothing.
ALTER TABLE fact_usage ADD COLUMN as_rule INTEGER NOT NULL DEFAULT 0 CHECK (as_rule IN (0, 1));
