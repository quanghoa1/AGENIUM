-- v2: change journal that keeps the optional ANN sidecar index (`agents/<id>.ann`) consistent with the facts.
-- The journal rows are written by triggers in the SAME transaction as the fact change, so after a crash the
-- sidecar is brought up to date by replaying every row with seq > the sidecar's recorded `applied_seq`
-- (or rebuilt from `fact_vec` when the sidecar is missing or fails its checks). Rows <= applied_seq are trimmed
-- after each sidecar save. The journal never affects query results: the exact index is always `fact_vec`.
CREATE TABLE ann_journal (
  seq      INTEGER PRIMARY KEY AUTOINCREMENT,
  op       INTEGER NOT NULL CHECK (op IN (1, 2)),   -- 1 = add, 2 = delete
  fact_id  INTEGER NOT NULL
) STRICT;
CREATE TRIGGER fact_ann_ai AFTER INSERT ON fact BEGIN
  INSERT INTO ann_journal(op, fact_id) VALUES (1, new.id);
END;
CREATE TRIGGER fact_ann_ad AFTER DELETE ON fact BEGIN
  INSERT INTO ann_journal(op, fact_id) VALUES (2, old.id);
END;
