//! The user's own OpenCode account, read without starting opencode: whether they signed in, the key a run of one of
//! its providers is given, and the models the account offers (ADR-68, amended 2026-10-31).

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The API key the user's own opencode holds for `provider` (`opencode auth login`: a row of `credential` in its
/// database), read-only; `None` when there is none or it is not an API key.
pub(super) fn user_login(provider: &str) -> Option<String> {
    login_in(&user_data()?, provider)
}

/// The user's opencode data folder (`XDG_DATA_HOME`, else `~/.local/share`), where `opencode/opencode.db` lives.
fn user_data() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|h| PathBuf::from(h).join(".local").join("share"))
        })
}

/// Whether the user signed in to their own opencode: an active stored credential or an account in its database, read
/// read-only. `None` when there is none: OpenCode's free models need no login, so that is not "logged out". It
/// replaces running `opencode auth list`, which starts opencode's own server even with `--standalone` (a `serve
/// --stdio` child, 2026-10-31) and without it the background service that outlives the check.
pub(crate) fn user_signed_in() -> Option<bool> {
    signed_in(&user_data()?)
}

/// [`user_signed_in`] from the data folder `data`.
fn signed_in(data: &Path) -> Option<bool> {
    let db = data.join("opencode").join("opencode.db");
    let c = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    let n = count("SELECT count(*) FROM credential WHERE active = 1")
        + count("SELECT count(*) FROM account");
    (n > 0).then_some(true)
}

/// [`user_login`] from the data folder `data` (opencode keeps `opencode/opencode.db` there).
fn login_in(data: &Path, provider: &str) -> Option<String> {
    let db = data.join("opencode").join("opencode.db");
    if !db.is_file() {
        return None;
    }
    let c = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let value: String = c
        .query_row(
            "SELECT value FROM credential WHERE integration_id = ?1 AND active = 1 \
             ORDER BY time_updated DESC LIMIT 1",
            [provider],
            |r| r.get(0),
        )
        .ok()?;
    let v: Value = serde_json::from_str(&value).ok()?;
    v["key"]
        .as_str()
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
}

/// The OpenCode providers an account can sign in to, as the opencode CLI names them, with their model list's endpoint:
/// the Go subscription and Zen (pay as you go). One account key works on both; the endpoint decides the bill.
const ACCOUNT_PROVIDERS: [(&str, &str); 2] = [
    ("opencode-go", "https://opencode.ai/zen/go/v1/models"),
    ("opencode", "https://opencode.ai/zen/v1/models"),
];

/// The models the user's own OpenCode account offers, as the opencode CLI names them (`opencode-go/beam-free`,
/// `opencode/glm-5.3-flash-free`): each provider the user signed in to in their opencode lists its models with the
/// user's key (read-only, as a run gets it), and no opencode process is started. Empty when not signed in or offline:
/// the caller then offers OpenCode's free models from the price catalogue. Before, only that catalogue was offered, so
/// the models of a Go subscription (its free ones too) could not be picked (2026-10-31).
pub async fn account_models() -> Vec<agenium_llm::ModelInfo> {
    let mut out = Vec::new();
    for (provider, url) in ACCOUNT_PROVIDERS {
        let Some(key) = user_login(provider) else {
            continue;
        };
        match list_account(url, &key).await {
            Ok(ids) => out.extend(prefixed(provider, ids)),
            Err(e) => tracing::warn!(provider, error = %e, "OpenCode models not listed"),
        }
    }
    out
}

/// One endpoint's model ids (`{"data": [{"id": …}]}`).
async fn list_account(url: &str, key: &str) -> Result<Vec<String>, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let r = http
        .get(url)
        .bearer_auth(key)
        // Its CDN refuses requests without a user agent (403 "error code: 1010").
        .header(reqwest::header::USER_AGENT, "agenium")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("HTTP {}", r.status()));
    }
    let v: Value = r.json().await.map_err(|e| e.to_string())?;
    Ok(v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect())
}

/// `ids` of `provider`, named as the opencode CLI takes them.
fn prefixed(provider: &str, ids: Vec<String>) -> impl Iterator<Item = agenium_llm::ModelInfo> + '_ {
    ids.into_iter().map(move |id| agenium_llm::ModelInfo {
        id: format!("{provider}/{id}"),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_users_login_for_a_provider_is_found() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join("opencode")).unwrap();
        let c =
            rusqlite::Connection::open(data.path().join("opencode").join("opencode.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE credential (id TEXT, integration_id TEXT, label TEXT, value TEXT, connector_id TEXT, \
             method_id TEXT, active INTEGER, time_created INTEGER, time_updated INTEGER);
             INSERT INTO credential VALUES ('c1', 'opencode-go', 'Go', '{\"type\":\"api\",\"key\":\"sk-old\"}', NULL, NULL, 0, 1, 1);
             INSERT INTO credential VALUES ('c2', 'opencode-go', 'Go', '{\"type\":\"api\",\"key\":\"sk-go\"}', NULL, NULL, 1, 2, 2);",
        )
        .unwrap();
        drop(c);
        assert_eq!(
            login_in(data.path(), "opencode-go").as_deref(),
            Some("sk-go")
        );
        assert_eq!(login_in(data.path(), "anthropic"), None);
        assert_eq!(login_in(&data.path().join("none"), "opencode-go"), None);
        // Signed in, read from the database without starting opencode: an active credential says so; none active, or
        // no database, is unknown (free models need no login).
        assert_eq!(signed_in(data.path()), Some(true));
        assert_eq!(signed_in(&data.path().join("none")), None);
        rusqlite::Connection::open(data.path().join("opencode").join("opencode.db"))
            .unwrap()
            .execute_batch("UPDATE credential SET active = 0;")
            .unwrap();
        assert_eq!(signed_in(data.path()), None);
    }

    /// An account's models are named as the opencode CLI takes them, under the provider that listed them; that name
    /// picks the key a run is given.
    #[test]
    fn account_models_are_named_by_their_provider() {
        let ids: Vec<String> = prefixed(
            "opencode-go",
            vec!["beam-free".into(), "deepseek-v4.1-flash".into()],
        )
        .map(|m| m.id)
        .collect();
        assert_eq!(
            ids,
            ["opencode-go/beam-free", "opencode-go/deepseek-v4.1-flash"]
        );
        assert_eq!(
            super::super::model_ref(&ids[0]),
            json!({"providerID": "opencode-go", "id": "beam-free"})
        );
        assert_eq!(
            ACCOUNT_PROVIDERS.map(|(p, _)| p),
            ["opencode-go", "opencode"]
        );
    }
}
