//! Reading the vectors and the journal out of the database.

use super::*;

/// `(count, sum(id))` of `fact_vec`, from its rowid shadow table (an index scan, not a vector scan).
pub(super) fn vec_stats(c: &Connection) -> Result<(u64, i64), StoreError> {
    Ok(c.query_row(
        "SELECT count(*), coalesce(sum(rowid), 0) FROM fact_vec_rowids",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
    )?)
}

/// The journal's high-water mark: the last `seq` ever handed out (trimming does not lower it).
pub(super) fn journal_hi(c: &Connection) -> Result<i64, StoreError> {
    Ok(c.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = 'ann_journal'",
        [],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}

pub(super) fn vector_of(c: &Connection, id: i64) -> Result<Option<Vec<f32>>, StoreError> {
    let blob: Option<Vec<u8>> = c
        .prepare_cached("SELECT embedding FROM fact_vec WHERE rowid = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    Ok(blob.map(|b| decode(&b)))
}

pub(super) fn decode(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|x| f32::from_le_bytes(*x))
        .collect()
}

/// Up to `limit` fact vectors with id > `after`, ascending, read straight from vec0's shadow tables (sqlite-vec 0.1.9,
/// pinned: `<t>_rowids(rowid, id, chunk_id, chunk_offset)`, `<t>_vector_chunks00(rowid = chunk_id, vectors)`). A range
/// read: vec0 itself can only scan everything or look rows up one by one, each loading a whole 1 024-row chunk.
pub(super) fn read_batch(
    c: &Connection,
    dim: usize,
    after: i64,
    limit: i64,
) -> Result<Vec<(i64, Vec<f32>)>, StoreError> {
    let rows: Vec<(i64, Option<i64>, Option<i64>)> = c
        .prepare_cached(
            "SELECT rowid, chunk_id, chunk_offset FROM fact_vec_rowids WHERE rowid > ?1 ORDER BY rowid LIMIT ?2",
        )?
        .query_map(params![after, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut chunks: HashMap<i64, Vec<u8>> = HashMap::new();
    let mut st =
        c.prepare_cached("SELECT vectors FROM fact_vec_vector_chunks00 WHERE rowid = ?1")?;
    let width = dim * 4;
    let mut out = Vec::with_capacity(rows.len());
    for (id, chunk, offset) in rows {
        let (Some(chunk), Some(offset)) = (chunk, offset) else {
            return Err(StoreError::Corrupt(format!(
                "fact_vec row {id} has no chunk position"
            )));
        };
        let blob = match chunks.entry(chunk) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(st.query_row([chunk], |r| r.get(0))?)
            }
        };
        let at = offset as usize * width;
        let Some(bytes) = blob.get(at..at + width) else {
            return Err(StoreError::Corrupt(format!(
                "fact_vec row {id} is outside its chunk"
            )));
        };
        out.push((id, decode(bytes)));
    }
    Ok(out)
}
