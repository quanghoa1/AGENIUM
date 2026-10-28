//! Consistent copies of a database and what they hold: snapshots in steps, logical tables, content hashes, files.

use super::*;

pub(super) fn sha256_file(path: &Path) -> Result<String, StoreError> {
    let mut f = File::open(path).map_err(|e| StoreError::io(path, e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| StoreError::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The bytes of a hex string; `None` when its length is odd or a character is not a hex digit. Checked byte by byte:
/// slicing the text two bytes at a time panicked on a malformed import file (odd length, or a non-ASCII character
/// split in two).
pub(crate) fn unhex(h: &str) -> Option<Vec<u8>> {
    let digit = |c: u8| (c as char).to_digit(16);
    let b = h.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    b.chunks(2)
        .map(|p| Some(((digit(p[0])? << 4) | digit(p[1])?) as u8))
        .collect()
}

pub(super) fn compress(src: &Path, dst: &Path) -> Result<(), StoreError> {
    let input = File::open(src).map_err(|e| StoreError::io(src, e))?;
    let out = File::create(dst).map_err(|e| StoreError::io(dst, e))?;
    let mut enc =
        zstd::Encoder::new(BufWriter::new(out), ZSTD_LEVEL).map_err(|e| StoreError::io(dst, e))?;
    std::io::copy(&mut BufReader::new(input), &mut enc).map_err(|e| StoreError::io(dst, e))?;
    let w = enc.finish().map_err(|e| StoreError::io(dst, e))?;
    w.into_inner()
        .map_err(|e| StoreError::io(dst, e.into_error()))?
        .sync_all()
        .map_err(|e| StoreError::io(dst, e))
}

pub(super) fn decompress(src: &Path, dst: &Path) -> Result<(), StoreError> {
    let input = File::open(src).map_err(|e| StoreError::io(src, e))?;
    let mut dec = zstd::Decoder::new(input).map_err(|e| StoreError::io(src, e))?;
    let mut out = BufWriter::new(File::create(dst).map_err(|e| StoreError::io(dst, e))?);
    std::io::copy(&mut dec, &mut out).map_err(|e| StoreError::io(dst, e))?;
    out.into_inner()
        .map_err(|e| StoreError::io(dst, e.into_error()))?
        .sync_all()
        .map_err(|e| StoreError::io(dst, e))
}

/// Runs `f` in one read transaction on `c` (every statement sees the same snapshot); a transaction the caller already
/// holds is used as it is.
pub(super) fn in_read_tx<T>(
    c: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    if !c.is_autocommit() {
        return f(c);
    }
    // `BEGIN` is deferred: the snapshot is taken by the first read.
    c.execute_batch("BEGIN")?;
    let r = c
        .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(StoreError::from)
        .and_then(|()| f(c));
    let end = c.execute_batch("COMMIT");
    let v = r?;
    end?;
    Ok(v)
}

/// Copies a consistent image of `src` into a new database file `dst` (rollback journal, not WAL). The copy runs in
/// one read transaction on `src`: without one, the stepped online backup restarts whenever another connection
/// commits between two steps, so under steady WAL writes it never finished.
pub fn snapshot(src: &Connection, dst: &Path) -> Result<(), StoreError> {
    snapshot_in_steps(src, dst, 1000, None)
}

pub(super) fn snapshot_in_steps(
    src: &Connection,
    dst: &Path,
    pages: i32,
    progress: Option<fn(rusqlite::backup::Progress)>,
) -> Result<(), StoreError> {
    register_sqlite_vec();
    if dst.exists() {
        fs::remove_file(dst).map_err(|e| StoreError::io(dst, e))?;
    }
    in_read_tx(src, |src| {
        let mut out = Connection::open(dst)?;
        {
            let b = Backup::new(src, &mut out)?;
            b.run_to_completion(pages, std::time::Duration::ZERO, progress)?;
        }
        out.pragma_update(None, "journal_mode", "DELETE")?;
        Ok(())
    })
}

/// Names of virtual tables and their shadow tables (excluded from logical hashes; rebuilt by SQLite/extensions).
pub(super) fn derived_tables(c: &Connection) -> Result<BTreeSet<String>, StoreError> {
    let mut st = c.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'",
    )?;
    let vt: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(all
        .into_iter()
        .filter(|t| vt.iter().any(|v| t == v || t.starts_with(&format!("{v}_"))))
        .collect())
}

/// Logical tables (user tables that are not derived and not internal).
pub fn logical_tables(c: &Connection) -> Result<Vec<String>, StoreError> {
    let derived = derived_tables(c)?;
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|t| !derived.contains(t)).collect())
}

/// Stable row order of a table: `rowid`, or the primary key columns of a `WITHOUT ROWID` table.
pub(crate) fn row_order(c: &Connection, table: &str) -> Result<String, StoreError> {
    let sql: String = c.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get(0),
    )?;
    if !sql.to_ascii_uppercase().contains("WITHOUT ROWID") {
        return Ok("rowid".into());
    }
    let mut st = c.prepare(&format!(
        "SELECT name FROM pragma_table_info({}) WHERE pk > 0 ORDER BY pk",
        quote_literal(table)
    ))?;
    let cols: Vec<String> = st
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    Ok(cols
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", "))
}

/// `true` when the table has a single-column `INTEGER PRIMARY KEY` (an alias of rowid).
pub(super) fn has_rowid_alias(c: &Connection, table: &str) -> Result<bool, StoreError> {
    let mut st = c.prepare(&format!(
        "SELECT type, pk FROM pragma_table_info({})",
        quote_literal(table)
    ))?;
    let pks: Vec<(String, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .filter(|(_, pk)| *pk > 0)
        .collect();
    Ok(pks.len() == 1 && pks[0].0.eq_ignore_ascii_case("INTEGER"))
}

pub(super) fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub(super) fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// sha256 per logical table over every row (ordered by rowid or primary key), type-tagged.
pub fn content_hash(c: &Connection) -> Result<BTreeMap<String, String>, StoreError> {
    let mut out = BTreeMap::new();
    for t in logical_tables(c)? {
        let mut h = Sha256::new();
        let order = row_order(c, &t)?;
        let mut st = c.prepare(&format!(
            "SELECT * FROM {} ORDER BY {order}",
            quote_ident(&t)
        ))?;
        let n = st.column_count();
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            for i in 0..n {
                match r.get_ref(i)? {
                    ValueRef::Null => h.update([0u8]),
                    ValueRef::Integer(v) => {
                        h.update([1u8]);
                        h.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        h.update([2u8]);
                        h.update(v.to_le_bytes());
                    }
                    ValueRef::Text(b) => {
                        h.update([3u8]);
                        h.update((b.len() as u64).to_le_bytes());
                        h.update(b);
                    }
                    ValueRef::Blob(b) => {
                        h.update([4u8]);
                        h.update((b.len() as u64).to_le_bytes());
                        h.update(b);
                    }
                }
            }
            h.update([0xffu8]);
        }
        out.insert(t, hex(&h.finalize()));
    }
    Ok(out)
}

pub(super) fn schema_version(c: &Connection) -> Result<u32, StoreError> {
    Ok(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
}
