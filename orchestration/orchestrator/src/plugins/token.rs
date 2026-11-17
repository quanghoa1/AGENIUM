//! The GitHub token of a private plugin repository, kept in Agenium's secret store (the OS keyring or the encrypted
//! vault, as the API keys) per repository: an update reads the repository with it and asks for a token only when
//! none is kept or GitHub refuses the kept one. A token pasted again replaces it; removing the repository's last
//! plugin forgets it.

use super::source::Source;
use crate::App;

/// The secret's name for the repository of `source` (`None` for a folder, which needs no token).
fn key(source: &Source) -> Option<String> {
    match source {
        Source::GitHub { owner, repo, .. } => Some(format!(
            "plugin-github/{}/{}",
            owner.to_ascii_lowercase(),
            repo.to_ascii_lowercase()
        )),
        Source::Folder(_) => None,
    }
}

impl App {
    /// The token to read `source` with: the one pasted, else the one kept for its repository.
    pub(super) fn plugin_token(&self, source: &Source, pasted: Option<&str>) -> Option<String> {
        let key = key(source)?;
        if let Some(t) = pasted.map(str::trim).filter(|t| !t.is_empty()) {
            return Some(t.to_owned());
        }
        let kept = self.secret(&key).ok().flatten()?;
        kept.expose_str().ok().map(str::to_owned)
    }

    /// Whether a token is kept for the repository of `source`.
    pub(super) fn plugin_token_kept(&self, source: &Source) -> bool {
        key(source).is_some_and(|k| matches!(self.secrets().get(&k), Ok(Some(_))))
    }

    /// Keeps the token an install read the repository with (a failure to keep it is logged: the install stands).
    pub(super) fn keep_plugin_token(&self, source: &Source, token: &str) {
        let Some(k) = key(source) else { return };
        if let Err(e) = self.secrets().set(&k, token.as_bytes()) {
            tracing::warn!(error = %e, "the plugin repository's token was not kept");
        }
    }

    /// Forgets the token of the repository of `source` when no installed plugin comes from it any more.
    pub(super) fn forget_plugin_token(&self, source: &Source) {
        let Some(k) = key(source) else { return };
        let still_used = self
            .plugins()
            .unwrap_or_default()
            .iter()
            .filter_map(|p| Source::parse(&p.source).ok())
            .any(|s| key(&s).as_deref() == Some(k.as_str()));
        if !still_used {
            let _ = self.secrets().delete(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_repositorys_token_is_kept_used_replaced_and_forgotten() {
        let tmp = tempfile::tempdir().unwrap();
        let (app, _) = App::open(
            &tmp.path().join("root"),
            crate::AppOptions {
                keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
                checks: Arc::new(crate::NoCheckFactory),
            },
        )
        .unwrap();
        let src = Source::parse("Acme/Pentest").unwrap();
        let same = Source::parse("acme/pentest@v2").unwrap();
        assert_eq!(app.plugin_token(&src, None), None, "nothing kept yet");
        app.keep_plugin_token(&src, "tok-one");
        assert!(app.plugin_token_kept(&same));
        assert_eq!(
            app.plugin_token(&same, None).as_deref(),
            Some("tok-one"),
            "an update needs no paste"
        );
        assert_eq!(
            app.plugin_token(&same, Some(" tok-two ")).as_deref(),
            Some("tok-two"),
            "a pasted one wins"
        );
        app.keep_plugin_token(&same, "tok-two");
        assert_eq!(
            app.plugin_token(&src, Some("")).as_deref(),
            Some("tok-two"),
            "replaced"
        );
        let folder = Source::parse(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(
            app.plugin_token(&folder, Some("x")),
            None,
            "a folder needs no token"
        );
        // No plugin of the repository is installed: removing forgets it.
        app.forget_plugin_token(&src);
        assert!(!app.plugin_token_kept(&src));
        assert_eq!(app.plugin_token(&src, None), None);
    }
}
