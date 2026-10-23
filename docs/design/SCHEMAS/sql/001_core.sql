-- Agenium core database (SQLite, WAL). Primary store engine (ADR-49); SurrealQL twin: ../surreal/db-core.surql.
-- Rules (decision 22): the event is written first and the state second, in ONE transaction (BEGIN IMMEDIATE).
-- Schema version = PRAGMA user_version (set by the migrator in the same transaction as the DDL).
-- JSON columns (acceptance, git, payload, ...) are validated with json_valid; record links are TEXT ids with
-- foreign keys, graph-like relations are join tables (ticket_dep, ticket_link, assigned_to).
-- Dates are UTC ISO-8601 text.

CREATE TABLE project (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  path        TEXT NOT NULL UNIQUE,                         -- folder containing agenium.toml
  key_prefix  TEXT NOT NULL DEFAULT 'AGN',
  next_seq    INTEGER NOT NULL DEFAULT 1,
  status      TEXT NOT NULL CHECK (status IN ('Active','Completed','Cancelled')),
  git         TEXT NOT NULL CHECK (json_valid(git)),        -- { main_branch, lfs, remote: {url, ssh_key_ref, last_test}? }
  allowlist   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowlist)),  -- only the user adds "always allow"
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;

CREATE TABLE team (
  id        TEXT PRIMARY KEY,
  name      TEXT NOT NULL,
  template  TEXT NOT NULL,                                  -- "coding@1"
  project   TEXT REFERENCES project(id),                    -- one project at a time
  lead      TEXT,                                           -- agent id
  solo      INTEGER NOT NULL DEFAULT 0 CHECK (solo IN (0,1))
) STRICT;

CREATE TABLE agent (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  team       TEXT NOT NULL REFERENCES team(id),
  role       TEXT NOT NULL,
  tags       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags)),
  status     TEXT NOT NULL CHECK (status IN ('Idle','Working','Waiting','QuotaExhausted','Offline')),
  backend    TEXT NOT NULL CHECK (backend IN ('Api','Cli')),
  model      TEXT NOT NULL CHECK (json_valid(model)),       -- { provider, model, effort }
  memory_db  TEXT NOT NULL,                                 -- agents/<id>.db
  mcp        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(mcp)),
  skills     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(skills))
) STRICT;

CREATE TABLE spec (
  id           TEXT PRIMARY KEY,
  project      TEXT NOT NULL REFERENCES project(id),
  version      INTEGER NOT NULL,
  body         TEXT NOT NULL CHECK (json_valid(body)),
  approved_by  TEXT,
  locked_at    TEXT,
  UNIQUE (project, version)
) STRICT;

CREATE TABLE plan (
  id            TEXT PRIMARY KEY,
  project       TEXT NOT NULL REFERENCES project(id),
  version       INTEGER NOT NULL,
  spec_version  INTEGER NOT NULL,
  reason        TEXT,
  diff          TEXT,
  body          TEXT NOT NULL CHECK (json_valid(body)),
  approved_by   TEXT,
  UNIQUE (project, version)
) STRICT;

CREATE TABLE ticket (
  id              TEXT PRIMARY KEY,
  key             TEXT NOT NULL,                            -- AGN-n
  project         TEXT NOT NULL REFERENCES project(id),
  parent          TEXT REFERENCES ticket(id),
  kind            TEXT NOT NULL CHECK (kind IN ('Epic','Story','Task','Bug','Requirement')),
  title           TEXT NOT NULL,
  description     TEXT NOT NULL,
  acceptance      TEXT NOT NULL CHECK (json_valid(acceptance) AND json_array_length(acceptance) > 0),
  status          TEXT NOT NULL CHECK (status IN ('New','Assigned','InProgress','Pending','Resolved','Reopened','Closed','Cancelled')),
  assignee        TEXT,                                     -- agent id (agent rows arrive with the registry in M1.5: no FK yet)
  plan_item       TEXT NOT NULL,
  priority        INTEGER NOT NULL CHECK (priority BETWEEN 0 AND 3),
  required_role   TEXT,
  required_tags   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(required_tags)),
  scope_paths     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scope_paths)),
  report_template TEXT NOT NULL,
  approved        INTEGER NOT NULL DEFAULT 0 CHECK (approved IN (0,1)),
  merged          INTEGER NOT NULL DEFAULT 0 CHECK (merged IN (0,1)),
  has_code        INTEGER NOT NULL DEFAULT 0 CHECK (has_code IN (0,1)),
  reopen_count    INTEGER NOT NULL DEFAULT 0,
  escalated       INTEGER NOT NULL DEFAULT 0 CHECK (escalated IN (0,1)),
  close_reason    TEXT,
  usage           TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),   -- tokens + estimated cost, display only
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (project, key)
) STRICT;
CREATE INDEX ticket_status   ON ticket(project, status);
CREATE INDEX ticket_assignee ON ticket(assignee, status);
CREATE INDEX ticket_plan     ON ticket(plan_item);

-- depends_on: DAG edges (cycle check in the store layer). Ordered by pos to keep the author's order.
CREATE TABLE ticket_dep (
  ticket      TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  depends_on  TEXT NOT NULL REFERENCES ticket(id),
  pos         INTEGER NOT NULL,
  PRIMARY KEY (ticket, depends_on)
) STRICT;
CREATE INDEX ticket_dep_rev ON ticket_dep(depends_on);

CREATE TABLE ticket_workspace (
  ticket      TEXT PRIMARY KEY REFERENCES ticket(id) ON DELETE CASCADE,
  worktree    TEXT NOT NULL,
  branch      TEXT NOT NULL,                                -- agenium/<ticket-id>
  base_sha    TEXT NOT NULL,
  head_sha    TEXT,
  merged_sha  TEXT
) STRICT;

CREATE TABLE ticket_link (
  src        TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  dst        TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  kind       TEXT NOT NULL CHECK (kind IN ('related_to','blocks','duplicates','follow_up_of','aggregates')),
  confirmed  INTEGER NOT NULL DEFAULT 0 CHECK (confirmed IN (0,1)),
  PRIMARY KEY (src, dst, kind)
) STRICT;
CREATE INDEX ticket_link_dst ON ticket_link(dst);

-- agent -> ticket relation ("tickets of agent" traversal)
CREATE TABLE assigned_to (
  agent   TEXT NOT NULL,
  ticket  TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  PRIMARY KEY (agent, ticket)
) STRICT;
CREATE INDEX assigned_to_ticket ON assigned_to(ticket);

CREATE TABLE comment (
  id          TEXT PRIMARY KEY,
  ticket      TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('Question','Answer','Progress','Report','Review','Permission','Handoff','System','User')),
  author      TEXT NOT NULL,                                -- agent:<id> | user | system
  reply_to    TEXT REFERENCES comment(id),
  body        TEXT NOT NULL CHECK (json_valid(body)),
  blocking    INTEGER NOT NULL DEFAULT 0 CHECK (blocking IN (0,1)),
  resolved    INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0,1)),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX comment_ticket ON comment(ticket, created_at);

CREATE TABLE permission_grant (
  id          TEXT PRIMARY KEY,
  ticket      TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  run         TEXT,                                         -- Lead grants are per run
  scope       TEXT NOT NULL CHECK (scope IN ('run','project')),
  granted_by  TEXT NOT NULL,
  request     TEXT NOT NULL CHECK (json_valid(request))
) STRICT;
-- "always allow" (scope = project) can only be granted by the user.
CREATE TRIGGER lead_only_run BEFORE INSERT ON permission_grant
WHEN NEW.scope = 'project' AND NEW.granted_by != 'user'
BEGIN SELECT RAISE(ABORT, 'always-allow (scope=project) can only be granted by the user'); END;

CREATE TABLE run (
  id          TEXT PRIMARY KEY,
  ticket      TEXT REFERENCES ticket(id),
  agent       TEXT NOT NULL,
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  outcome     TEXT,                                         -- Completed | Stopped(reason) | Error
  steps       INTEGER NOT NULL DEFAULT 0,
  usage       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),
  provider    TEXT NOT NULL
) STRICT;

-- Append-only source of truth for UI resync (seq is gap-free, allocated from counter inside the commit transaction).
CREATE TABLE event (
  seq        INTEGER PRIMARY KEY,
  intent_id  TEXT UNIQUE,                                   -- idempotency
  kind       TEXT NOT NULL,
  ticket     TEXT,                                          -- no FK: the log outlives deleted tickets
  actor      TEXT NOT NULL,
  payload    TEXT NOT NULL CHECK (json_valid(payload)),
  at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX event_ticket ON event(ticket, seq);
CREATE TRIGGER event_immutable BEFORE UPDATE ON event
BEGIN SELECT RAISE(ABORT, 'event log is append-only'); END;

CREATE TABLE counter (name TEXT PRIMARY KEY, n INTEGER NOT NULL) STRICT;
INSERT INTO counter(name, n) VALUES ('event', 0);

CREATE TABLE model_cache (
  provider    TEXT PRIMARY KEY,
  models      TEXT NOT NULL CHECK (json_valid(models)),
  fetched_at  TEXT NOT NULL
) STRICT;

-- Retention (ADR-49): old events are moved to zstd JSONL files under <core dir>/event-archive/ and deleted from
-- `event`. `counter.event` is never touched, so seq stays gap-free and monotonic; readers fall through to the
-- archive when asked for a cursor older than the oldest live event. One row per archive file.
CREATE TABLE event_archive (
  from_seq    INTEGER PRIMARY KEY,
  to_seq      INTEGER NOT NULL CHECK (to_seq >= from_seq),
  rows        INTEGER NOT NULL,
  file        TEXT NOT NULL,                                -- relative to the archive directory
  sha256      TEXT NOT NULL,                                -- of the compressed file
  bytes       INTEGER NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
