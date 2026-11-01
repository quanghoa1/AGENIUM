-- core.db migration 7 (ADR-152): a helper session the user named keeps that name; otherwise its title follows its draft
-- (the skill's or the team's name) once it has one, else the first message.
ALTER TABLE helper_session ADD COLUMN named INTEGER NOT NULL DEFAULT 0;
