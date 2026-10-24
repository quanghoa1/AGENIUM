-- Per-agent memory database, migration 2: graph memory (ADR-51, M15-7). Adapted from the ADR draft
-- `docs/design/SCHEMAS/sql/drafts/agent_003_graph_memory.sql` (numbered 002 here: this implementation has one earlier
-- agent migration). Differences from the draft: `fact_usage` already exists (migration 1) and only gains the outcome
-- signal columns; `entity.degree` caches live edge counts for the hub cap. `entity_vec` (vec0, same dimension as
-- `fact_vec`) is created on open like `fact_vec`. Nothing is deleted when it becomes wrong: `valid_to`,
-- `superseded_by` and `merged_into` mark it.

-- ---------------------------------------------------------------- fact extensions
ALTER TABLE fact ADD COLUMN valid_from TEXT;
ALTER TABLE fact ADD COLUMN valid_to   TEXT;
ALTER TABLE fact ADD COLUMN origin     TEXT NOT NULL DEFAULT 'own';     -- own | ticket:<KEY> | pack:<id> | lead
ALTER TABLE fact ADD COLUMN gid        TEXT;                            -- sha256(normalised text + scope key)
ALTER TABLE fact ADD COLUMN shareable  INTEGER NOT NULL DEFAULT 0 CHECK (shareable IN (0,1));
CREATE UNIQUE INDEX fact_gid ON fact(gid) WHERE gid IS NOT NULL;

ALTER TABLE fact_usage ADD COLUMN signal TEXT;                          -- closed | reopened | lead_correction | contradicted
ALTER TABLE fact_usage ADD COLUMN outcome_at TEXT;

-- ---------------------------------------------------------------- relation vocabulary
CREATE TABLE edge_type (
  name        TEXT PRIMARY KEY,
  inverse     TEXT,                                   -- display name read backwards; an edge is stored once
  symmetric   INTEGER NOT NULL DEFAULT 0 CHECK (symmetric IN (0,1)),
  traverse    INTEGER NOT NULL DEFAULT 1 CHECK (traverse IN (0,1)),
  hop_weight  REAL NOT NULL DEFAULT 0.6 CHECK (hop_weight > 0 AND hop_weight <= 1)
) STRICT;
INSERT INTO edge_type(name, inverse, symmetric, traverse, hop_weight) VALUES
  ('is_a',           'has_subtype',    0, 1, 0.8),
  ('part_of',        'has_part',       0, 1, 0.8),
  ('depends_on',     'required_by',    0, 1, 0.7),
  ('uses',           'used_by',        0, 1, 0.6),
  ('configured_with','configures',     0, 1, 0.6),
  ('causes',         'caused_by',      0, 1, 0.8),
  ('fixes',          'fixed_by',       0, 1, 0.9),
  ('workaround_for', 'has_workaround', 0, 1, 0.9),
  ('conflicts_with', NULL,             1, 1, 0.7),
  ('applies_to',     'has_rule',       0, 1, 0.9),
  ('replaced_by',    'replaces',       0, 1, 0.5),
  ('observed_in',    'observed',       0, 0, 0.4),
  ('related_to',     NULL,             1, 1, 0.3);

-- ---------------------------------------------------------------- entities
CREATE TABLE entity (
  id            INTEGER PRIMARY KEY,
  type          TEXT NOT NULL,                        -- open lowercase vocabulary (device, vendor, product, software, ...)
  name          TEXT NOT NULL,
  canonical_key TEXT NOT NULL,
  scope         TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),   -- identity scope
  scope_key     TEXT NOT NULL DEFAULT '',
  description   TEXT,
  confidence    REAL NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0 AND 1),
  evidence      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence)),
  mentions      INTEGER NOT NULL DEFAULT 1,
  degree        INTEGER NOT NULL DEFAULT 0,           -- live edges touching it (hub cap)
  origin        TEXT NOT NULL DEFAULT 'own',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  last_seen_at  TEXT,
  valid_from    TEXT,
  valid_to      TEXT,
  superseded_by INTEGER REFERENCES entity(id),
  merged_into   INTEGER REFERENCES entity(id)
) STRICT;
CREATE UNIQUE INDEX entity_identity ON entity(type, canonical_key, scope_key) WHERE merged_into IS NULL;
CREATE INDEX entity_live ON entity(type) WHERE merged_into IS NULL AND valid_to IS NULL;

CREATE TABLE entity_alias (
  entity_id  INTEGER NOT NULL REFERENCES entity(id),
  alias_norm TEXT NOT NULL,
  alias      TEXT NOT NULL,
  source     TEXT NOT NULL DEFAULT 'extract',         -- extract | merge | user
  PRIMARY KEY (alias_norm, entity_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX entity_alias_entity ON entity_alias(entity_id);

-- Maintained by the application (rowid = entity.id).
CREATE VIRTUAL TABLE entity_fts USING fts5(name, aliases, description, tokenize='porter unicode61');

CREATE TABLE entity_merge_log (
  id         INTEGER PRIMARY KEY,
  from_id    INTEGER NOT NULL,
  into_id    INTEGER NOT NULL,
  similarity REAL,
  decided_by TEXT NOT NULL,                           -- auto | llm | user
  at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  moved      TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(moved))
) STRICT;

-- ---------------------------------------------------------------- edges
CREATE TABLE edge (
  id            INTEGER PRIMARY KEY,
  src           INTEGER NOT NULL REFERENCES entity(id),
  dst           INTEGER NOT NULL REFERENCES entity(id),
  type          TEXT NOT NULL REFERENCES edge_type(name),
  scope         TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),   -- applicability scope
  scope_key     TEXT NOT NULL DEFAULT '',
  weight        REAL NOT NULL DEFAULT 1.0,
  confidence    REAL NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0 AND 1),
  evidence_fact INTEGER REFERENCES fact(id),
  source        TEXT NOT NULL DEFAULT 'extract',
  origin        TEXT NOT NULL DEFAULT 'own',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  valid_from    TEXT,
  valid_to      TEXT,
  superseded_by INTEGER REFERENCES edge(id),
  CHECK (src <> dst)
) STRICT;
CREATE UNIQUE INDEX edge_identity ON edge(src, dst, type, scope_key) WHERE valid_to IS NULL;
CREATE INDEX edge_out ON edge(src, type, dst) WHERE valid_to IS NULL;
CREATE INDEX edge_in  ON edge(dst, type, src) WHERE valid_to IS NULL;
CREATE INDEX edge_fact ON edge(evidence_fact) WHERE evidence_fact IS NOT NULL;

-- ---------------------------------------------------------------- fact <-> entity
CREATE TABLE fact_entity (
  fact_id    INTEGER NOT NULL REFERENCES fact(id),
  entity_id  INTEGER NOT NULL REFERENCES entity(id),
  role       TEXT NOT NULL DEFAULT 'mention' CHECK (role IN ('subject','object','mention','scope')),
  confidence REAL NOT NULL DEFAULT 1.0,
  PRIMARY KEY (fact_id, entity_id, role)
) STRICT, WITHOUT ROWID;
CREATE INDEX fact_entity_by_entity ON fact_entity(entity_id, fact_id);
