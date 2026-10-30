-- core.db migration 5 (ADR-147): what must survive a crash between a decision and its effect, and what a plan item's
-- cancelled tickets count for.

-- A message to a team's Lead (the user's chat, an answer to a worker's question routed through the Lead) is stored
-- before it is acknowledged, and marked delivered once a run of the Lead had it in its conversation. Undelivered ones
-- go to the Lead again after a restart.
CREATE TABLE lead_message (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  project      TEXT NOT NULL REFERENCES project(id),
  text         TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  delivered_at TEXT
) STRICT;
CREATE INDEX lead_message_pending ON lead_message(project, delivered_at);

-- A ticket that became Closed or Reopened, for its assignee to learn from, whoever made it so (the Lead's review, the
-- user on the board). One row per ticket state, written with the transition; removed once the learning ran.
CREATE TABLE learn_pending (
  ticket     TEXT NOT NULL REFERENCES ticket(id) ON DELETE CASCADE,
  state      TEXT NOT NULL,                                          -- `<status>:<reopen_count>`
  created_at TEXT NOT NULL,
  PRIMARY KEY (ticket, state)
) STRICT;

-- How a cancelled ticket's work is accounted for in its plan item: `replaced_by` the ticket that carries it on, or
-- `waived_by` the user, who agreed it is not needed. A Lead's cancel with neither waits for the user: the item is not
-- done meanwhile. Tickets cancelled before this migration count as waived.
CREATE TABLE ticket_cancel (
  ticket       TEXT PRIMARY KEY REFERENCES ticket(id) ON DELETE CASCADE,
  cancelled_by TEXT NOT NULL,                                        -- user | lead | legacy
  replaced_by  TEXT REFERENCES ticket(id),
  waived_by    TEXT,                                                 -- user | legacy
  note         TEXT,
  at           TEXT NOT NULL
) STRICT;
INSERT INTO ticket_cancel (ticket, cancelled_by, waived_by, at)
  SELECT id, 'legacy', 'legacy', updated_at FROM ticket WHERE status = 'Cancelled';
