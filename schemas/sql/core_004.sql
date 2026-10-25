-- core.db migration 4 (M4, propose_role): a Lead proposes a role its team lacks; agents are created only after the
-- user approves (decision 3l). One row per proposal; the decision and the agent it created are kept.
CREATE TABLE role_proposal (
  id          TEXT PRIMARY KEY,
  project     TEXT NOT NULL REFERENCES project(id),
  team        TEXT NOT NULL REFERENCES team(id),
  proposed_by TEXT NOT NULL REFERENCES agent(id),
  body        TEXT NOT NULL CHECK (json_valid(body)),                -- ProposeRole
  status      TEXT NOT NULL CHECK (status IN ('pending','approved','declined')),
  agent       TEXT REFERENCES agent(id),                             -- the agent created on approval
  note        TEXT,                                                  -- the user's reason when declining
  created_at  TEXT NOT NULL,
  decided_at  TEXT
) STRICT;
CREATE INDEX role_proposal_project ON role_proposal(project, status);
