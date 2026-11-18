//! How agents name what is on the map: `kind:name`. Code is `file:<path>` and `symbol:<path>#<name>`; anything else
//! (a vendor, a product, a domain, an endpoint, a service) is any lowercase kind and its name.

use crate::{OrchError, Result};

/// A node's name as an agent wrote it, with its key (what identifies it on the map).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub kind: String,
    pub name: String,
    /// The file of a `file` or `symbol` node.
    pub file: Option<String>,
    /// The declaration of a `symbol` node.
    pub symbol: Option<String>,
}

impl Ref {
    pub fn parse(raw: &str) -> Result<Ref> {
        let raw = raw.trim();
        let (kind, name) = raw.split_once(':').ok_or_else(|| {
            bad(format!(
                "{raw:?} is not a map name: write kind:name (file:src/app.py, symbol:src/app.py#load, domain:api.example.com)"
            ))
        })?;
        let kind = kind.trim().to_ascii_lowercase();
        let name = name.trim();
        if !(2..=24).contains(&kind.len())
            || !kind.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        {
            return Err(bad(format!(
                "{kind:?} is not a kind: a short lowercase word (file, symbol, service, domain, vendor, product…)"
            )));
        }
        if name.is_empty() || name.chars().count() > 200 {
            return Err(bad(format!(
                "{raw:?} has no name or a name over 200 characters"
            )));
        }
        match kind.as_str() {
            "file" => Ok(Ref {
                file: Some(path(name)?),
                name: path(name)?,
                kind,
                symbol: None,
            }),
            "symbol" => {
                let (f, s) = name.split_once('#').ok_or_else(|| {
                    bad(format!(
                        "{raw:?}: a symbol is symbol:<file>#<name> (symbol:src/app.py#load_config)"
                    ))
                })?;
                let (f, s) = (path(f)?, s.trim().to_owned());
                if s.is_empty() {
                    return Err(bad(format!("{raw:?}: the symbol has no name after #")));
                }
                Ok(Ref {
                    name: format!("{f}#{s}"),
                    file: Some(f),
                    symbol: Some(s),
                    kind,
                })
            }
            _ => Ok(Ref {
                name: name.split_whitespace().collect::<Vec<_>>().join(" "),
                kind,
                file: None,
                symbol: None,
            }),
        }
    }

    /// The node's key: paths keep their case, other names do not.
    pub fn key(&self) -> String {
        if self.is_code() {
            format!("{}:{}", self.kind, self.name)
        } else {
            format!("{}:{}", self.kind, self.name.to_lowercase())
        }
    }

    pub fn is_code(&self) -> bool {
        matches!(self.kind.as_str(), "file" | "symbol")
    }

    /// What evidence of a link to this node must contain: a symbol's own name (its last part), a file's name
    /// without its folder, anything else its name.
    pub fn term(&self) -> String {
        match (&self.symbol, &self.file) {
            (Some(s), _) => s
                .rsplit([':', '.'])
                .find(|p| !p.is_empty())
                .unwrap_or(s)
                .to_owned(),
            (None, Some(f)) => {
                let base = f.rsplit('/').next().unwrap_or(f);
                base.split('.').next().unwrap_or(base).to_owned()
            }
            _ => self.name.clone(),
        }
    }
}

/// A worktree-relative path: forward slashes, no `..`, not absolute.
fn path(p: &str) -> Result<String> {
    let p = p.trim().replace('\\', "/");
    let p = p.trim_start_matches("./").to_owned();
    if p.is_empty() || p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s == "..") {
        return Err(bad(format!(
            "{p:?}: a file is a path inside the project, relative to its root"
        )));
    }
    Ok(p)
}

/// A link kind: a lowercase verb or phrase (calls, imports, reads, requests, hosted_at, part_of…).
pub fn link_kind(k: &str) -> Result<String> {
    let k = k.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    if (2..=30).contains(&k.len()) && k.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        Ok(k)
    } else {
        Err(bad(format!(
            "{k:?} is not a link type: a lowercase verb (calls, imports, reads, writes, requests, hosted_at, part_of…)"
        )))
    }
}

pub(super) fn bad(m: String) -> OrchError {
    OrchError::Invalid(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_into_keys_and_terms() {
        let s = Ref::parse("symbol:./src\\app.py#Config.load").unwrap();
        assert_eq!(s.key(), "symbol:src/app.py#Config.load");
        assert_eq!(s.term(), "load");
        let f = Ref::parse("file:src/net/client.rs").unwrap();
        assert_eq!(
            (f.key(), f.term()),
            ("file:src/net/client.rs".into(), "client".into())
        );
        let d = Ref::parse("Domain: API.Anthropic.com").unwrap();
        assert_eq!(d.key(), "domain:api.anthropic.com");
        assert_eq!(d.term(), "API.Anthropic.com");
        let p = Ref::parse("product:Claude   Desktop").unwrap();
        assert_eq!(p.name, "Claude Desktop");
        for bad in [
            "nokind",
            "file:../x",
            "file:/etc/passwd",
            "symbol:a.py",
            "K:x",
            "file:C:/x",
        ] {
            assert!(Ref::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(link_kind("Hosted at").unwrap(), "hosted_at");
        assert!(link_kind("x").is_err());
    }
}
