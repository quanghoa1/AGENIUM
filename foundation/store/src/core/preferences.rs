//! The user's last choices, kept in the root (ADR-152): one JSON value per key.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::StoreError;

pub fn get(c: &Connection, key: &str) -> Result<Option<Value>, StoreError> {
    let v: Option<String> = c
        .query_row(
            "SELECT value FROM preference WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
}

pub fn set(c: &Connection, key: &str, value: &Value, at: &str) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO preference (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value.to_string(), at],
    )?;
    Ok(())
}
