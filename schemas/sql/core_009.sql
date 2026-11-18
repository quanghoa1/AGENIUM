-- core.db migration 9 (ADR-166): the project map. Agents add what they traced (code) or read (the web) with its
-- evidence; Agenium checks it before it is kept. A node is one thing (a file, a function, a vendor, a domain), named by
-- its key `kind:name` (a symbol `symbol:<file>#<name>`); a link joins two nodes with a typed relation and its evidence.
-- `status`: tentative (added in a ticket still open), confirmed, stale (its evidence stopped matching).
CREATE TABLE map_node (
  project    TEXT NOT NULL,
  key        TEXT NOT NULL,
  kind       TEXT NOT NULL,
  name       TEXT NOT NULL,
  file       TEXT,
  line_start INTEGER,
  line_end   INTEGER,
  summary    TEXT NOT NULL DEFAULT '',
  evidence   TEXT NOT NULL DEFAULT '{}',
  status     TEXT NOT NULL,
  ticket     TEXT,
  agent      TEXT NOT NULL,
  run        TEXT NOT NULL,
  created_at TEXT NOT NULL,
  checked_at TEXT NOT NULL,
  PRIMARY KEY (project, key)
);
CREATE INDEX map_node_kind ON map_node(project, kind);

CREATE TABLE map_link (
  id         INTEGER PRIMARY KEY,
  project    TEXT NOT NULL,
  src        TEXT NOT NULL,
  kind       TEXT NOT NULL,
  dst        TEXT NOT NULL,
  evidence   TEXT NOT NULL,
  status     TEXT NOT NULL,
  ticket     TEXT,
  agent      TEXT NOT NULL,
  run        TEXT NOT NULL,
  created_at TEXT NOT NULL,
  checked_at TEXT NOT NULL,
  UNIQUE (project, src, kind, dst)
);
CREATE INDEX map_link_out ON map_link(project, src);
CREATE INDEX map_link_in ON map_link(project, dst);
