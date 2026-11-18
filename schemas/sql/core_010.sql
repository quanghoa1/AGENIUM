-- core.db migration 10: events read by kind (the latest plan draft, the Teams page) are found by an index, not by
-- scanning the whole retained log.
CREATE INDEX event_kind ON event(kind, seq);
