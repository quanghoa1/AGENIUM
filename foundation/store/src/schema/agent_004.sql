-- agent.db migration 4: a lesson injected as a rule of the work's context (`Via::Rule`, decision 35) is logged as used.
-- Its `via = 'rule'` failed the CHECK of migration 1 and `INSERT OR IGNORE` dropped the row without a word, so such
-- lessons were never recorded as used. SQLite cannot change a CHECK, so the table is rebuilt with its rows and rowids
-- (the runner turns foreign keys off around migrations and checks them before commit).
CREATE TABLE fact_usage_new (
  fact       INTEGER NOT NULL REFERENCES fact(id),
  ticket     TEXT,
  run        TEXT NOT NULL,
  rank       INTEGER NOT NULL,
  via        TEXT NOT NULL CHECK (via IN ('vector','keyword','hybrid','graph','recent','rule')),
  outcome    TEXT CHECK (outcome IN ('helped','hurt','neutral')),
  at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  signal     TEXT,                                                       -- closed | reopened | lead_correction | contradicted
  outcome_at TEXT,
  PRIMARY KEY (run, fact)
) STRICT;
INSERT INTO fact_usage_new (rowid, fact, ticket, run, rank, via, outcome, at, signal, outcome_at)
  SELECT rowid, fact, ticket, run, rank, via, outcome, at, signal, outcome_at FROM fact_usage ORDER BY rowid;
DROP TABLE fact_usage;
ALTER TABLE fact_usage_new RENAME TO fact_usage;
CREATE INDEX fact_usage_fact ON fact_usage(fact);
