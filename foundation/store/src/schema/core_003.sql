-- core.db migration 3 (ADR-61, agent pool): an agent exists before and between teams. `team` becomes nullable (an
-- agent in no team is in the pool; still at most one team at a time, decision 3). SQLite cannot drop NOT NULL, so
-- the table is rebuilt (the runner turns foreign keys off around migrations and checks them before commit).
CREATE TABLE agent_new (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  team       TEXT REFERENCES team(id),                               -- NULL = in the pool (no team)
  role       TEXT NOT NULL,
  role_kind  TEXT NOT NULL CHECK (role_kind IN ('lead','worker')),
  tags       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags)),
  status     TEXT NOT NULL CHECK (status IN ('Idle','Working','Waiting','QuotaExhausted','Offline')),
  backend    TEXT NOT NULL CHECK (backend IN ('Api','Cli')),
  model      TEXT NOT NULL CHECK (json_valid(model)),                -- ModelBinding
  mcp        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(mcp)),
  skills     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(skills)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  deleted_at TEXT
) STRICT;
INSERT INTO agent_new (id, name, team, role, role_kind, tags, status, backend, model, mcp, skills, created_at, deleted_at)
  SELECT id, name, team, role, role_kind, tags, status, backend, model, mcp, skills, created_at, deleted_at FROM agent ORDER BY rowid;
DROP TABLE agent;
ALTER TABLE agent_new RENAME TO agent;
CREATE INDEX agent_team ON agent(team);
