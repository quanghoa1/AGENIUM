-- DRAFT / NOT YET APPLIED — design sketch for ADR-51 "Graph memory on SQLite".
-- Not part of the migrations (crates/agenium-store/schema/sqlite/ has only agent_001, agent_002) and not mirrored by
-- `cargo xtask schemas`. When implemented (M1.5/M2, see IMPLEMENTATION-PLAN) it becomes agent_003.sql in that directory.
-- Applies to the per-agent database `agents/<id>.db` (on top of agent_001/002: tables fact, fact_fts, fact_vec, ann_journal).
-- All text is English (ADR-40). The sketch was run against SQLite 3.46 (tables, indexes, the queries in ADR-51).
-- `entity_vec` (sqlite-vec vec0, same dimension as fact_vec) is created at open like fact_vec and is not shown here:
--   CREATE VIRTUAL TABLE entity_vec USING vec0(embedding float[<dim>] distance_metric=cosine);   -- rowid = entity.id

-- ---------------------------------------------------------------- fact extensions
ALTER TABLE fact ADD COLUMN valid_from TEXT;                          -- when the statement became true (NULL = unknown/created_at)
ALTER TABLE fact ADD COLUMN valid_to   TEXT;                          -- when it stopped being true (NULL = still true)
ALTER TABLE fact ADD COLUMN origin     TEXT NOT NULL DEFAULT 'own';   -- own | ticket:<KEY> | pack:<pack-id> | lead
ALTER TABLE fact ADD COLUMN gid        TEXT;                          -- sha256(normalised text + canonical scope): dedup across agents/packs
ALTER TABLE fact ADD COLUMN shareable INTEGER NOT NULL DEFAULT 0 CHECK (shareable IN (0,1));  -- set only by Lead/user (pack promotion), never by the extractor
CREATE UNIQUE INDEX fact_gid ON fact(gid) WHERE gid IS NOT NULL;

-- ---------------------------------------------------------------- vocabulary of relations
CREATE TABLE edge_type (
  name        TEXT PRIMARY KEY,                       -- 'fixes', 'causes', ...
  inverse     TEXT,                                   -- display name when read backwards ('fixed_by'); NOT stored twice
  symmetric   INTEGER NOT NULL DEFAULT 0 CHECK (symmetric IN (0,1)),
  traverse    INTEGER NOT NULL DEFAULT 1 CHECK (traverse IN (0,1)),   -- followed by graph expansion?
  hop_weight  REAL NOT NULL DEFAULT 0.6 CHECK (hop_weight > 0 AND hop_weight <= 1)
) STRICT;
INSERT INTO edge_type(name, inverse, symmetric, traverse, hop_weight) VALUES
  ('is_a',          'has_subtype',    0, 1, 0.8),
  ('part_of',       'has_part',       0, 1, 0.8),
  ('depends_on',    'required_by',    0, 1, 0.7),
  ('uses',          'used_by',        0, 1, 0.6),
  ('configured_with','configures',    0, 1, 0.6),
  ('causes',        'caused_by',      0, 1, 0.8),
  ('fixes',         'fixed_by',       0, 1, 0.9),
  ('workaround_for','has_workaround', 0, 1, 0.9),
  ('conflicts_with', NULL,            1, 1, 0.7),
  ('applies_to',    'has_rule',       0, 1, 0.9),
  ('replaced_by',   'replaces',       0, 1, 0.5),
  ('observed_in',   'observed',       0, 0, 0.4),      -- entity <-> ticket/run provenance; not expanded by default
  ('related_to',    NULL,             1, 1, 0.3);      -- weak catch-all; down-weighted

-- ---------------------------------------------------------------- entities
CREATE TABLE entity (
  id            INTEGER PRIMARY KEY,
  type          TEXT NOT NULL,                        -- open lowercase vocabulary: device, vendor, product, software, library,
                                                      -- language, project, repo, file, service, config, error, concept, command, person
  name          TEXT NOT NULL,                        -- display name, English
  canonical_key TEXT NOT NULL,                        -- normalised name (NFKC, lowercase, punctuation/space collapsed)
  scope         TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),   -- identity scope: {"vendor":"tp-link","project":"..."}
  scope_key     TEXT NOT NULL DEFAULT '',             -- canonical "k=v;k=v" of scope (sorted) for the unique index
  description   TEXT,
  confidence    REAL NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0 AND 1),
  evidence      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence)),  -- ["ticket:AGN-12","fact:41"]
  mentions      INTEGER NOT NULL DEFAULT 1,
  origin        TEXT NOT NULL DEFAULT 'own',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  last_seen_at  TEXT,
  valid_from    TEXT,
  valid_to      TEXT,
  superseded_by INTEGER REFERENCES entity(id),
  merged_into   INTEGER REFERENCES entity(id)         -- entity resolution: tombstone pointing at the survivor
) STRICT;
CREATE UNIQUE INDEX entity_identity ON entity(type, canonical_key, scope_key) WHERE merged_into IS NULL;
CREATE INDEX entity_live ON entity(type) WHERE merged_into IS NULL AND valid_to IS NULL;

CREATE TABLE entity_alias (
  entity_id  INTEGER NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  alias_norm TEXT NOT NULL,                           -- normalised like canonical_key
  alias      TEXT NOT NULL,
  source     TEXT NOT NULL DEFAULT 'extract',         -- extract | merge | user
  PRIMARY KEY (alias_norm, entity_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX entity_alias_entity ON entity_alias(entity_id);

CREATE VIRTUAL TABLE entity_fts USING fts5(name, aliases, description, tokenize='porter unicode61');  -- maintained by the app

CREATE TABLE entity_merge_log (                       -- audit + undo information for entity resolution
  id INTEGER PRIMARY KEY, from_id INTEGER NOT NULL, into_id INTEGER NOT NULL,
  similarity REAL, decided_by TEXT NOT NULL,          -- auto | llm | user
  at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  moved TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(moved))   -- ids of edges / fact_entity rows rewired
) STRICT;

-- ---------------------------------------------------------------- edges
CREATE TABLE edge (
  id            INTEGER PRIMARY KEY,
  src           INTEGER NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  dst           INTEGER NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  type          TEXT NOT NULL REFERENCES edge_type(name),
  scope         TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(scope)),   -- e.g. a "fixes" edge valid only for {"vendor":"tp-link"}
  scope_key     TEXT NOT NULL DEFAULT '',
  weight        REAL NOT NULL DEFAULT 1.0,            -- strength; bumped on re-observation
  confidence    REAL NOT NULL DEFAULT 0.5 CHECK (confidence BETWEEN 0 AND 1),
  evidence_fact INTEGER REFERENCES fact(id) ON DELETE SET NULL,         -- the fact that justifies the edge (provenance)
  source        TEXT NOT NULL DEFAULT 'extract',
  origin        TEXT NOT NULL DEFAULT 'own',
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  valid_from    TEXT,
  valid_to      TEXT,                                 -- NULL = live; set (never delete) when contradicted
  superseded_by INTEGER REFERENCES edge(id),
  CHECK (src <> dst)
) STRICT;
CREATE UNIQUE INDEX edge_identity ON edge(src, dst, type, scope_key) WHERE valid_to IS NULL;
CREATE INDEX edge_out ON edge(src, type, dst) WHERE valid_to IS NULL;      -- forward traversal (covering for the CTE)
CREATE INDEX edge_in  ON edge(dst, type, src) WHERE valid_to IS NULL;      -- backward traversal
CREATE INDEX edge_fact ON edge(evidence_fact) WHERE evidence_fact IS NOT NULL;

-- ---------------------------------------------------------------- fact <-> entity
CREATE TABLE fact_entity (
  fact_id    INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
  entity_id  INTEGER NOT NULL REFERENCES entity(id) ON DELETE CASCADE,
  role       TEXT NOT NULL DEFAULT 'mention' CHECK (role IN ('subject','object','mention','scope')),
  confidence REAL NOT NULL DEFAULT 1.0,
  PRIMARY KEY (fact_id, entity_id, role)
) STRICT, WITHOUT ROWID;
CREATE INDEX fact_entity_by_entity ON fact_entity(entity_id, fact_id);

-- ---------------------------------------------------------------- reuse signals
CREATE TABLE fact_usage (                             -- which facts were shown to which run; outcome is filled in later
  id           INTEGER PRIMARY KEY,
  fact_id      INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
  ticket       TEXT NOT NULL,                         -- AGN-n
  run          TEXT,
  rank         INTEGER,
  score        REAL,
  via          TEXT NOT NULL DEFAULT 'vec' CHECK (via IN ('vec','bm25','graph','pack')),
  retrieved_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  outcome      TEXT CHECK (outcome IN ('helped','neutral','hurt')),    -- NULL until the ticket settles
  signal       TEXT,                                  -- closed | reopened | lead_correction | review_change | contradicted
  outcome_at   TEXT
) STRICT;
CREATE INDEX fact_usage_fact   ON fact_usage(fact_id, retrieved_at);
CREATE INDEX fact_usage_ticket ON fact_usage(ticket);
