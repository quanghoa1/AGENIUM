-- agent.db migration 7 (ADR-145): what the settling pass found of a pair of live facts that mean nearly the same, so a
-- pair is judged once. `same` (folded), `distinct` (both kept), `conflict` (both kept until the user decides:
-- `resolved` is set then). `a` < `b`.
CREATE TABLE fact_pair (
    a INTEGER NOT NULL,
    b INTEGER NOT NULL,
    verdict TEXT NOT NULL CHECK (verdict IN ('same', 'distinct', 'conflict')),
    similarity REAL NOT NULL,
    by TEXT NOT NULL,
    at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved TEXT,
    PRIMARY KEY (a, b),
    CHECK (a < b)
) WITHOUT ROWID;
CREATE INDEX fact_pair_open ON fact_pair(verdict) WHERE verdict = 'conflict' AND resolved IS NULL;
