-- Per-agent private memory database `db/agents/<agent-id>.db`, migration 1 (decision 2, ADR-49/50).
-- Facts are English (ADR-40). The vector table `fact_vec` (sqlite-vec vec0, cosine, dimension fixed per database
-- in agent_meta.embedding_dim) is created on open because its dimension is a runtime choice. Only live facts
-- (not superseded) have a vector; the text in `fact` is the source of truth (re-embedding rebuilds vectors).

CREATE TABLE agent_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;   -- embedding_dim, embedder, agent_id

CREATE TABLE fact (
  id             INTEGER PRIMARY KEY,
  text           TEXT NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('fact','preference','lesson')),
  project        TEXT,                                                 -- project the fact was learned in (decision 25)
  scope          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),  -- vendor/product/version/language/project/domain
  cue            TEXT,
  evidence       TEXT,
  source         TEXT NOT NULL,                                        -- ticket:<key> | chat | review
  untrusted      INTEGER NOT NULL DEFAULT 0 CHECK (untrusted IN (0,1)),
  confidence     REAL NOT NULL DEFAULT 0.5,
  weight         REAL NOT NULL DEFAULT 1.0,                            -- decay; hidden below 0.2 (decision 35)
  model_id       TEXT,                                                 -- embedder that produced the vector
  superseded_by  INTEGER REFERENCES fact(id),
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  used_at        TEXT
) STRICT;
CREATE INDEX fact_live ON fact(superseded_by);
CREATE INDEX fact_updated ON fact(updated_at);

CREATE VIRTUAL TABLE fact_fts USING fts5(text, content='fact', content_rowid='id', tokenize='porter unicode61');
CREATE TRIGGER fact_ai AFTER INSERT ON fact BEGIN
  INSERT INTO fact_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER fact_ad AFTER DELETE ON fact BEGIN
  INSERT INTO fact_fts(fact_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER fact_au AFTER UPDATE OF text ON fact BEGIN
  INSERT INTO fact_fts(fact_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO fact_fts(rowid, text) VALUES (new.id, new.text);
END;

-- Every fact injected into a work order is logged (M1-11, ADR-51 reuse signal; `outcome` filled in M4.5).
CREATE TABLE fact_usage (
  fact     INTEGER NOT NULL REFERENCES fact(id),
  ticket   TEXT,
  run      TEXT NOT NULL,
  rank     INTEGER NOT NULL,
  via      TEXT NOT NULL CHECK (via IN ('vector','keyword','hybrid','graph','recent')),
  outcome  TEXT CHECK (outcome IN ('helped','hurt','neutral')),
  at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (run, fact)
) STRICT;
CREATE INDEX fact_usage_fact ON fact_usage(fact);

-- Resumable run context: normalized messages, append-only per session (ConversationStore).
CREATE TABLE conversation (
  session  TEXT NOT NULL,
  n        INTEGER NOT NULL,
  message  TEXT NOT NULL CHECK (json_valid(message)),
  at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (session, n)
) STRICT;

CREATE TABLE skill_overlay (
  skill        TEXT PRIMARY KEY,
  base_version TEXT NOT NULL,
  patch        TEXT NOT NULL,
  changelog    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(changelog))
) STRICT;

-- Change journal for the optional ANN sidecar (ADR-50): written by triggers in the same transaction as the
-- vector change, replayed into the sidecar on open.
CREATE TABLE ann_journal (
  seq      INTEGER PRIMARY KEY AUTOINCREMENT,
  op       INTEGER NOT NULL CHECK (op IN (1, 2)),                      -- 1 = add, 2 = delete
  fact_id  INTEGER NOT NULL
) STRICT;
