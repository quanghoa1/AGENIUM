-- core.db migration 6 (ADR-151/152): the model calls of the app's own helpers (the team designer, the skill writer).
-- They belong to no agent, so they are not runs; the usage views count them beside the runs.
CREATE TABLE helper_call (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL,                                         -- design | skill
  provider    TEXT NOT NULL,                                         -- an API provider's id, or a CLI program
  model       TEXT NOT NULL,
  api         INTEGER NOT NULL,                                      -- 1: an API provider, 0: a vendor CLI
  session     TEXT,                                                  -- the helper session it belongs to
  started_at  TEXT NOT NULL,
  ended_at    TEXT NOT NULL,
  usage       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(usage))
) STRICT;
CREATE INDEX helper_call_started ON helper_call(started_at);

-- A new skill an agent distilled from a lesson that held in two contexts (ADR-91, amended by ADR-152): it waits for
-- the user, who saves it into the library (every agent may enable it; the proposing agent gets it switched on) or
-- declines it. Agents never add a skill on their own; they change only their own copy of a library skill.
CREATE TABLE skill_proposal (
  id          TEXT PRIMARY KEY,
  project     TEXT REFERENCES project(id),                           -- the proposing agent's project then
  agent       TEXT NOT NULL REFERENCES agent(id),
  name        TEXT NOT NULL,
  text        TEXT NOT NULL,                                         -- the SKILL.md it proposes
  lesson      INTEGER NOT NULL,                                      -- in the agent's memory
  lesson_text TEXT NOT NULL,
  contexts    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(contexts)), -- the closed tickets it held in
  similar     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(similar)),  -- library skills close to it
  notes       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(notes)),    -- contradictions with library skills
  reason      TEXT NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('pending','approved','declined')),
  skill       TEXT,                                                  -- the library skill it was saved as
  note        TEXT,                                                  -- why it was declined
  created_at  TEXT NOT NULL,
  decided_at  TEXT
) STRICT;
CREATE INDEX skill_proposal_status ON skill_proposal(status, project);
CREATE INDEX skill_proposal_agent ON skill_proposal(agent, status);

-- What the user chose last, kept in the root (ADR-152): the model of each place a model is picked (the agent form, the
-- team form, the skill writer, the template designer), so a form opens with it instead of empty.
CREATE TABLE preference (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL CHECK (json_valid(value)),
  updated_at TEXT NOT NULL
) STRICT;

-- A helper chat (ADR-152): the team designer's or the skill writer's conversation, kept so it can be reopened and
-- continued. `turns` is what the model is sent back (the helper's whole answers), `transcript` what the user saw;
-- `attachments` the documents already read as Markdown; `source` what it started from (an agent's skill proposal, a
-- skill the designer proposed); `result` what was saved from it.
CREATE TABLE helper_session (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('design','skill')),
  title       TEXT NOT NULL,
  model       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(model)),
  turns       TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(turns)),
  transcript  TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(transcript)),
  attachments TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(attachments)),
  draft       TEXT CHECK (draft IS NULL OR json_valid(draft)),
  source      TEXT CHECK (source IS NULL OR json_valid(source)),
  status      TEXT NOT NULL CHECK (status IN ('open','saved','closed')),
  result      TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
) STRICT;
CREATE INDEX helper_session_kind ON helper_session(kind, updated_at);
