//! Secret stores. Default = hybrid vault (spike S5 decision): one 256-bit master key in the OS keyring, every
//! secret in an XChaCha20-Poly1305 encrypted file in the root. This removes per-platform blob limits (Windows
//! Credential Manager ~2.5 KB) and keyring prompts per secret. Alternative = each secret directly in the keyring.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::keyring_backend::KeyringBackend;
use crate::SecretError;

pub const SERVICE: &str = "agenium";
const MAGIC: &[u8; 6] = b"AGNV1\n";

/// A secret value. Its `Debug`/`Display` never print the content.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(v: impl Into<Vec<u8>>) -> Self {
        Self(v.into())
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
    pub fn expose_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(&self.0).map_err(|_| SecretError::Backend("secret is not UTF-8".into()))
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret([{} bytes])", self.0.len())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // Best effort wipe; volatile so the compiler keeps it.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

/// Names look like `provider/anthropic`, `mcp/github/token`, `ssh/ed25519`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'))
}

pub trait SecretStore: Send + Sync {
    fn set(&self, name: &str, value: &[u8]) -> Result<(), SecretError>;
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;
    fn delete(&self, name: &str) -> Result<(), SecretError>;
    fn list(&self) -> Result<Vec<String>, SecretError>;
    fn describe(&self) -> String;
}

fn check_name(name: &str) -> Result<(), SecretError> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(SecretError::InvalidName(name.to_owned()))
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

// ---------------------------------------------------------------- vault

pub struct VaultStore {
    path: PathBuf,
    keyring: Arc<dyn KeyringBackend>,
    account: String,
    lock: Mutex<()>,
}

impl VaultStore {
    /// `account` identifies the master key in the keyring (one per root, e.g. `vault-master/<root id>`).
    pub fn new(path: &Path, keyring: Arc<dyn KeyringBackend>, account: &str) -> Self {
        Self {
            path: path.to_path_buf(),
            keyring,
            account: account.to_owned(),
            lock: Mutex::new(()),
        }
    }

    fn cipher(&self, create: bool) -> Result<Option<XChaCha20Poly1305>, SecretError> {
        let stored = self.keyring.get(SERVICE, &self.account)?;
        let key = match stored {
            Some(k) => unhex(std::str::from_utf8(&k).unwrap_or_default())
                .filter(|k| k.len() == 32)
                .ok_or_else(|| {
                    SecretError::Corrupt("vault master key in the keyring is malformed".into())
                })?,
            None if !create => {
                if self.path.exists() {
                    return Err(SecretError::Corrupt(format!(
                        "{} exists but its master key is missing from the keyring ({})",
                        self.path.display(),
                        self.keyring.name()
                    )));
                }
                return Ok(None);
            }
            None => {
                if self.path.exists() {
                    return Err(SecretError::Corrupt(format!(
                        "{} exists but its master key is missing from the keyring; refusing to overwrite it",
                        self.path.display()
                    )));
                }
                let k = XChaCha20Poly1305::generate_key(&mut OsRng);
                self.keyring
                    .set(SERVICE, &self.account, hex(&k).as_bytes())?;
                k.to_vec()
            }
        };
        XChaCha20Poly1305::new_from_slice(&key)
            .map(Some)
            .map_err(|_| SecretError::Corrupt("bad key length".into()))
    }

    fn load(&self, c: &XChaCha20Poly1305) -> Result<BTreeMap<String, String>, SecretError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(SecretError::Io(format!("{}: {e}", self.path.display()))),
        };
        if bytes.len() < MAGIC.len() + 24 || &bytes[..MAGIC.len()] != MAGIC {
            return Err(SecretError::Corrupt(format!(
                "{} is not an Agenium vault",
                self.path.display()
            )));
        }
        let nonce = XNonce::from_slice(&bytes[MAGIC.len()..MAGIC.len() + 24]);
        let plain = c.decrypt(nonce, &bytes[MAGIC.len() + 24..]).map_err(|_| {
            SecretError::Corrupt("vault authentication failed (wrong key or tampered file)".into())
        })?;
        let map: BTreeMap<String, String> = serde_json_like::parse(&plain)
            .ok_or_else(|| SecretError::Corrupt("vault content is malformed".into()))?;
        Ok(map)
    }

    fn save(
        &self,
        c: &XChaCha20Poly1305,
        map: &BTreeMap<String, String>,
    ) -> Result<(), SecretError> {
        let plain = serde_json_like::render(map);
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = c
            .encrypt(&nonce, plain.as_bytes())
            .map_err(|_| SecretError::Backend("encryption failed".into()))?;
        let mut out = Vec::with_capacity(MAGIC.len() + 24 + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| SecretError::Io(e.to_string()))?;
        }
        let tmp = self.path.with_extension("tmp");
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| SecretError::Io(format!("{}: {e}", tmp.display())))?;
        f.write_all(&out)
            .and_then(|_| f.sync_all())
            .map_err(|e| SecretError::Io(e.to_string()))?;
        drop(f);
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| SecretError::Io(format!("{}: {e}", self.path.display())))
    }
}

impl SecretStore for VaultStore {
    fn set(&self, name: &str, value: &[u8]) -> Result<(), SecretError> {
        check_name(name)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let c = self
            .cipher(true)?
            .ok_or_else(|| SecretError::Backend("no vault key".into()))?;
        let mut map = self.load(&c)?;
        map.insert(name.to_owned(), hex(value));
        self.save(&c, &map)
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        check_name(name)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let Some(c) = self.cipher(false)? else {
            return Ok(None);
        };
        let map = self.load(&c)?;
        map.get(name)
            .map(|h| {
                unhex(h)
                    .map(Secret)
                    .ok_or_else(|| SecretError::Corrupt(format!("secret {name} is malformed")))
            })
            .transpose()
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        check_name(name)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let Some(c) = self.cipher(false)? else {
            return Ok(());
        };
        let mut map = self.load(&c)?;
        if map.remove(name).is_some() {
            self.save(&c, &map)?;
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, SecretError> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let Some(c) = self.cipher(false)? else {
            return Ok(vec![]);
        };
        Ok(self.load(&c)?.into_keys().collect())
    }

    fn describe(&self) -> String {
        format!(
            "encrypted vault {} (master key in {})",
            self.path.display(),
            self.keyring.name()
        )
    }
}

// ---------------------------------------------------------------- direct keyring

/// Every secret directly in the OS keyring (account = secret name). The list of names is kept in an index entry.
pub struct KeyringStore {
    keyring: Arc<dyn KeyringBackend>,
    lock: Mutex<()>,
}

const INDEX: &str = "__index__";

impl KeyringStore {
    pub fn new(keyring: Arc<dyn KeyringBackend>) -> Self {
        Self {
            keyring,
            lock: Mutex::new(()),
        }
    }

    fn index(&self) -> Result<Vec<String>, SecretError> {
        Ok(self
            .keyring
            .get(SERVICE, INDEX)?
            .map(|b| {
                String::from_utf8_lossy(&b)
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn set_index(&self, names: &[String]) -> Result<(), SecretError> {
        self.keyring
            .set(SERVICE, INDEX, names.join("\n").as_bytes())
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, name: &str, value: &[u8]) -> Result<(), SecretError> {
        check_name(name)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.keyring.set(SERVICE, name, value)?;
        let mut idx = self.index()?;
        if !idx.iter().any(|n| n == name) {
            idx.push(name.to_owned());
            self.set_index(&idx)?;
        }
        Ok(())
    }

    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        check_name(name)?;
        Ok(self.keyring.get(SERVICE, name)?.map(Secret))
    }

    fn delete(&self, name: &str) -> Result<(), SecretError> {
        check_name(name)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.keyring.delete(SERVICE, name)?;
        let idx: Vec<String> = self.index()?.into_iter().filter(|n| n != name).collect();
        self.set_index(&idx)
    }

    fn list(&self) -> Result<Vec<String>, SecretError> {
        let mut v = self.index()?;
        v.sort();
        Ok(v)
    }

    fn describe(&self) -> String {
        format!("OS keyring ({})", self.keyring.name())
    }
}

/// Tiny, dependency-free `{"name":"hex",...}` codec for the vault plaintext (names are restricted to a safe
/// charset by [`valid_name`], values are hex, so no escaping is needed).
mod serde_json_like {
    use std::collections::BTreeMap;

    pub fn render(m: &BTreeMap<String, String>) -> String {
        let body: Vec<String> = m.iter().map(|(k, v)| format!("\"{k}\":\"{v}\"")).collect();
        format!("{{{}}}", body.join(","))
    }

    pub fn parse(b: &[u8]) -> Option<BTreeMap<String, String>> {
        let s = std::str::from_utf8(b).ok()?.trim();
        let inner = s.strip_prefix('{')?.strip_suffix('}')?;
        let mut m = BTreeMap::new();
        if inner.is_empty() {
            return Some(m);
        }
        for pair in inner.split(',') {
            let (k, v) = pair.split_once(':')?;
            let k = k.strip_prefix('"')?.strip_suffix('"')?;
            let v = v.strip_prefix('"')?.strip_suffix('"')?;
            if !super::valid_name(k) || !v.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            m.insert(k.to_owned(), v.to_owned());
        }
        Some(m)
    }
}
