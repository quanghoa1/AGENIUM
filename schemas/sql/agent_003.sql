-- agent.db migration 3 (M4.5, decision 35): the contexts (`project:<id>`, `ticket:<key>`) in which a lesson was used
-- and the work held; a lesson validated in two or more contexts may update the agent's own skill overlay.
ALTER TABLE fact ADD COLUMN validated TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(validated));
