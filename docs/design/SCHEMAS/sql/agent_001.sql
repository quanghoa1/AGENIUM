-- Per-agent private memory database `agents/<agent-id>.db` (decision 2). Embeddings (default 384-d) live in the
-- sqlite-vec virtual table `fact_vec` (created at open with the configured dimension, exact cosine KNN),
-- keyword search in the FTS5 table `fact_fts` (BM25, porter stemming). All facts are English (ADR-40).
CREATE TABLE fact (
  id             INTEGER PRIMARY KEY,
  text           TEXT NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('fact','preference','lesson')),
  scope          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),   -- vendor/product/version/language/project/domain
  cue            TEXT,
  evidence       TEXT,
  source         TEXT NOT NULL,                                          -- ticket:<key> | chat | review
  untrusted      INTEGER NOT NULL DEFAULT 0 CHECK (untrusted IN (0,1)),
  confidence     REAL NOT NULL DEFAULT 0.5,
  weight         REAL NOT NULL DEFAULT 1.0,                              -- decay half-life 90 days, hidden < 0.2
  superseded_by  INTEGER REFERENCES fact(id),
  created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  used_at        TEXT
) STRICT;
CREATE INDEX fact_live ON fact(superseded_by);

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

CREATE TABLE conversation (                                  -- Solo/Lead chat with the user
  id         TEXT PRIMARY KEY,
  started_at TEXT NOT NULL,
  closed_at  TEXT,
  messages   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(messages))
) STRICT;

CREATE TABLE skill_overlay (
  skill        TEXT PRIMARY KEY,
  base_version TEXT NOT NULL,
  patch        TEXT NOT NULL,
  changelog    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(changelog))
) STRICT;

CREATE TABLE agent_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;   -- embedding_dim, embedder, ...
