-- agent.db migration 6 (ADR-125 follow-up): live lessons by an index. `rules_in` runs on every recall with a scope (every
-- work run) and found them through `fact_live`, i.e. by reading every live fact: 7 ms at 100 000 facts, linear beyond.
CREATE INDEX fact_live_lessons ON fact(kind, untrusted) WHERE superseded_by IS NULL;
