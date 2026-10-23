-- Agenium core database, migration 1 (SQLite, WAL, STRICT tables). Canonical copy: this file; mirrored to
-- schemas/sql/ by `cargo xtask schemas`. Design reference: docs/design/SCHEMAS/sql/001_core.sql (ADR-49).
-- Rules: an intent's events and its state changes commit in ONE `BEGIN IMMEDIATE` transaction (decision 22);
-- the schema version is `PRAGMA user_version`; ids are ULID text, timestamps UTC ISO-8601 text; relations are
-- join tables with foreign keys.

CREATE TABLE project (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  path        TEXT NOT NULL UNIQUE,                                   -- folder containing agenium.toml
  key_prefix  TEXT NOT NULL DEFAULT 'AGN',
  next_seq    INTEGER NOT NULL DEFAULT 1,
  status      TEXT NOT NULL CHECK (status IN ('Active','Paused','Completed','Cancelled')),
  main_branch TEXT NOT NULL DEFAULT 'main',
  git         TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(git)),     -- { lfs, remote: {url, ssh_key_ref, last_test}? }
  allowlist   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowlist)),  -- "always allow": only the user adds entries
  settings    TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(settings)),   -- e.g. { review_before_merge }
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;

CREATE TABLE team (
  id        TEXT PRIMARY KEY,
  name      TEXT NOT NULL,
  template  TEXT NOT NULL,                                           -- "coding@1"
  project   TEXT REFERENCES project(id),                             -- one project at a time (decision 25)
  lead      TEXT,                                                    -- agent id of the Lead slot
  solo      INTEGER NOT NULL DEFAULT 0 CHECK (solo IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;

CREATE TABLE agent (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  team       TEXT NOT NULL REFERENCES team(id),                      -- exactly one team (decision 3)
  role       TEXT NOT NULL,
  role_kind  TEXT NOT NULL CHECK (role_kind IN ('lead','worker')),
  tags       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags)),
  status     TEXT NOT NULL CHECK (status IN ('Idle','Working','Waiting','QuotaExhausted','Offline')),
  backend    TEXT NOT NULL CHECK (backend IN ('Api','Cli')),
  model      TEXT NOT NULL CHECK (json_valid(model)),                -- ModelBinding
  mcp        TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(mcp)),
  skills     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(skills)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX agent_team ON agent(team);

CREATE TABLE spec (
  id           TEXT PRIMARY KEY,
  project      TEXT NOT NULL REFERENCES project(id),
  version      INTEGER NOT NULL,
  body         TEXT NOT NULL CHECK (json_valid(body)),
  proposed_by  TEXT NOT NULL,
  approved_by  TEXT,
  locked_at    TEXT,
  created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
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
  proposed_by   TEXT NOT NULL,
  approved_by   TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (project, version)
) STRICT;

CREATE TABLE ticket (
  id              TEXT PRIMARY KEY,
  key             TEXT NOT NULL,                                     -- AGN-n
  project         TEXT NOT NULL REFERENCES project(id),
  parent          TEXT REFERENCES ticket(id),
  kind            TEXT NOT NULL CHECK (kind IN ('Epic','Story','Task','Bug','Requirement')),
  title           TEXT NOT NULL,
  description     TEXT NOT NULL,
  acceptance      TEXT NOT NULL CHECK (json_valid(acceptance) AND json_array_length(acceptance) > 0),
  status          TEXT NOT NULL CHECK (status IN ('New','Assigned','InProgress','Pending','Resolved','Reopened','Closed','Cancelled')),
  assignee        TEXT REFERENCES agent(id),
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
  usage           TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),  -- display only (decision 3k)
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE (project, key)
) STRICT;
CREATE INDEX ticket_status   ON ticket(project, status);
CREATE INDEX ticket_assignee ON ticket(assignee, status);
CREATE INDEX ticket_plan     ON ticket(project, plan_item);

-- depends_on: DAG edges (cycle check before insert). `pos` keeps the author's order.
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
  branch      TEXT NOT NULL,                                         -- agenium/<ticket-key>
  base_sha    TEXT NOT NULL,
  head_sha    TEXT,
  merged_sha  TEXT,
  removed     INTEGER NOT NULL DEFAULT 0 CHECK (removed IN (0,1))
) STRICT;

-- Journal of git side effects (decision 22, ADR-44): a row is inserted with its `VcsOp` event in the transaction
-- that decides the operation, executed after commit, and marked done/failed with its `VcsOpDone` event. Reconcile
-- re-runs every `pending` row on startup (operations are idempotent per ticket + sha).
CREATE TABLE vcs_op (
  seq         INTEGER PRIMARY KEY,                                   -- seq of the VcsOp event
  ticket      TEXT NOT NULL REFERENCES ticket(id),
  op          TEXT NOT NULL,
  detail      TEXT NOT NULL CHECK (json_valid(detail)),
  state       TEXT NOT NULL CHECK (state IN ('pending','done','failed')),
  result      TEXT CHECK (result IS NULL OR json_valid(result)),
  updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX vcs_op_pending ON vcs_op(state);

CREATE TABLE ticket_link (
  src          TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  dst          TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  kind         TEXT NOT NULL CHECK (kind IN ('related_to','blocks','duplicates','follow_up_of','aggregates')),
  proposed_by  TEXT NOT NULL,
  confirmed_by TEXT,
  note         TEXT,
  PRIMARY KEY (src, dst, kind)
) STRICT;
CREATE INDEX ticket_link_dst ON ticket_link(dst);

CREATE TABLE comment (
  id          TEXT PRIMARY KEY,
  ticket      TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL CHECK (kind IN ('Question','Answer','Progress','Report','Review','Permission','Handoff','System','User')),
  author      TEXT NOT NULL,                                         -- agent:<id> | user | system
  reply_to    TEXT REFERENCES comment(id),
  body        TEXT NOT NULL CHECK (json_valid(body)),
  blocking    INTEGER NOT NULL DEFAULT 0 CHECK (blocking IN (0,1)),
  resolved    INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0,1)),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX comment_ticket ON comment(ticket, created_at);
CREATE INDEX comment_open ON comment(ticket, resolved, blocking);

CREATE TABLE permission_grant (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL REFERENCES project(id),
  ticket      TEXT REFERENCES ticket(id) ON DELETE CASCADE,
  run         TEXT,                                                  -- Lead grants are per run
  scope       TEXT NOT NULL CHECK (scope IN ('run','project')),
  granted_by  TEXT NOT NULL,
  request     TEXT NOT NULL CHECK (json_valid(request)),
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  revoked_at  TEXT                                                   -- run grants end when the ticket is Resolved/Closed/Cancelled
) STRICT;
CREATE INDEX grant_ticket ON permission_grant(ticket, revoked_at);
-- "always allow" (scope = project) can only be granted by the user (decision 4).
CREATE TRIGGER grant_project_scope_user_only BEFORE INSERT ON permission_grant
WHEN NEW.scope = 'project' AND NEW.granted_by != 'user'
BEGIN SELECT RAISE(ABORT, 'always-allow (scope=project) can only be granted by the user'); END;
CREATE TRIGGER grant_run_scope_needs_run BEFORE INSERT ON permission_grant
WHEN NEW.scope = 'run' AND NEW.run IS NULL
BEGIN SELECT RAISE(ABORT, 'a run-scoped grant needs a run id'); END;

CREATE TABLE run (
  id          TEXT PRIMARY KEY,
  ticket      TEXT REFERENCES ticket(id),
  agent       TEXT NOT NULL REFERENCES agent(id),
  provider    TEXT NOT NULL,
  model       TEXT NOT NULL,
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  outcome     TEXT,                                                  -- Completed | Parked | Stopped:<reason> | Error:<msg>
  steps       INTEGER NOT NULL DEFAULT 0,
  usage       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage)),
  heartbeat   TEXT
) STRICT;
CREATE INDEX run_ticket ON run(ticket, started_at);
CREATE INDEX run_agent ON run(agent, started_at);

-- Structured run log (diagnostics, replay): LLM calls, tool calls, errors. Full tool output lives in git
-- artifacts, not here (decision 23).
CREATE TABLE run_log (
  run    TEXT NOT NULL REFERENCES run(id) ON DELETE CASCADE,
  n      INTEGER NOT NULL,
  kind   TEXT NOT NULL,
  data   TEXT NOT NULL CHECK (json_valid(data)),
  at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (run, n)
) STRICT;

-- Append-only event log. `seq` is gap-free: allocated from `counter` inside the committing transaction.
CREATE TABLE event (
  seq        INTEGER PRIMARY KEY,
  intent_id  TEXT,
  kind       TEXT NOT NULL,
  ticket     TEXT,                                                   -- no FK: the log outlives tickets
  actor      TEXT NOT NULL,
  payload    TEXT NOT NULL CHECK (json_valid(payload)),
  at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE INDEX event_ticket ON event(ticket, seq);
CREATE INDEX event_intent ON event(intent_id);
CREATE TRIGGER event_immutable BEFORE UPDATE ON event
BEGIN SELECT RAISE(ABORT, 'event log is append-only'); END;

-- Idempotency: one row per committed intent with its result; a retried intent returns the stored result and
-- changes nothing (one intent may produce several events).
CREATE TABLE intent (
  id         TEXT PRIMARY KEY,
  actor      TEXT NOT NULL,
  first_seq  INTEGER,
  result     TEXT NOT NULL CHECK (json_valid(result)),
  at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;

CREATE TABLE counter (name TEXT PRIMARY KEY, n INTEGER NOT NULL) STRICT;
INSERT INTO counter(name, n) VALUES ('event', 0);

CREATE TABLE model_cache (
  provider    TEXT PRIMARY KEY,
  models      TEXT NOT NULL CHECK (json_valid(models)),
  fetched_at  TEXT NOT NULL
) STRICT;

-- Retention (ADR-50): old events move to zstd JSONL files; `counter.event` never resets.
CREATE TABLE event_archive (
  from_seq    INTEGER PRIMARY KEY,
  to_seq      INTEGER NOT NULL CHECK (to_seq >= from_seq),
  rows        INTEGER NOT NULL,
  file        TEXT NOT NULL,
  sha256      TEXT NOT NULL,
  bytes       INTEGER NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
