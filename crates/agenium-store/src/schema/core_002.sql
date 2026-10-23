-- core.db migration 2 (M15-5, decision 32): deleting an agent removes its memory database, skills, SOUL.md and
-- workspace for good, but the row stays as a tombstone so runs, tickets and events keep valid references. A deleted
-- agent is renamed `<name>~<id>` (the name can be reused), set Offline and hidden from listings.
ALTER TABLE agent ADD COLUMN deleted_at TEXT;
