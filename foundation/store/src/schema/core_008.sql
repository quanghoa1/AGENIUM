-- core.db migration 8 (ADR-158): the runs still going, found without reading every run (the scheduler asks each round
-- whether a Lead has one; reconcile and the merge checks read them too).
CREATE INDEX run_open ON run(agent) WHERE ended_at IS NULL;
