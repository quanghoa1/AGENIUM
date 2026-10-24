//! Raw OS keyring access with a hard timeout (spike S5: a locked keyring on a headless session hangs instead of
//! failing) and an in-memory backend for tests.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use crate::SecretError;

/// Minimal keyring operations on `(service, account)` byte blobs.
pub trait KeyringBackend: Send + Sync {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError>;
    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;
    /// Human readable backend name for diagnostics.
    fn name(&self) -> &'static str;
}

/// The platform keyring (Windows Credential Manager, macOS Keychain, Secret Service).
pub struct OsKeyring {
    timeout: Duration,
}

impl OsKeyring {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    /// Checks that the platform store initialised (keyring 4 has no default store otherwise; the real cause
    /// is only visible here, spike S5). Returns a user-facing hint on failure.
    pub fn status() -> Result<(), SecretError> {
        match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(e) => Err(SecretError::Unavailable(format!(
                "OS keyring not available ({e}). Linux: start a Secret Service (gnome-keyring/KWallet) and unlock it; \
                 macOS: unlock the login keychain; Windows: Credential Manager must be enabled."
            ))),
        }
    }

    fn with_timeout<T: Send + 'static>(
        &self,
        what: &'static str,
        f: impl FnOnce() -> Result<T, SecretError> + Send + 'static,
    ) -> Result<T, SecretError> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("agenium-keyring".into())
            .spawn(move || {
                let _ = tx.send(f());
            })
            .map_err(|e| SecretError::Unavailable(format!("cannot start keyring thread: {e}")))?;
        rx.recv_timeout(self.timeout).unwrap_or_else(|_| {
            Err(SecretError::Timeout(format!(
                "keyring {what} did not answer within {:?}; the keyring may be locked (unlock it and retry)",
                self.timeout
            )))
        })
    }
}

impl Default for OsKeyring {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

fn map_err(e: keyring::Error) -> SecretError {
    match e {
        keyring::Error::NoStorageAccess(p) => {
            SecretError::Unavailable(format!("keyring locked or access denied: {p}"))
        }
        keyring::Error::TooLong(attr, limit) => {
            SecretError::TooLarge(format!("{attr} exceeds the platform limit of {limit}"))
        }
        other => SecretError::Backend(other.to_string()),
    }
}

impl KeyringBackend for OsKeyring {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
        let (s, a) = (service.to_owned(), account.to_owned());
        self.with_timeout("read", move || {
            let e = keyring::Entry::new(&s, &a).map_err(map_err)?;
            match e.get_secret() {
                Ok(v) => Ok(Some(v)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(map_err(e)),
            }
        })
    }

    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
        let (s, a, v) = (service.to_owned(), account.to_owned(), value.to_vec());
        self.with_timeout("write", move || {
            let e = keyring::Entry::new(&s, &a).map_err(map_err)?;
            e.set_secret(&v).map_err(map_err)
        })
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        let (s, a) = (service.to_owned(), account.to_owned());
        self.with_timeout("delete", move || {
            let e = keyring::Entry::new(&s, &a).map_err(map_err)?;
            match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(map_err(e)),
            }
        })
    }

    fn name(&self) -> &'static str {
        if cfg!(windows) {
            "Windows Credential Manager"
        } else if cfg!(target_os = "macos") {
            "macOS Keychain"
        } else {
            "Secret Service"
        }
    }
}

/// Process-local keyring for tests and for `--ephemeral` runs. Never persists anything.
#[derive(Default)]
pub struct MemoryKeyring {
    map: Mutex<HashMap<(String, String), Vec<u8>>>,
    /// Simulates a hung/locked keyring.
    pub fail: Mutex<Option<SecretError>>,
}

impl KeyringBackend for MemoryKeyring {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
        if let Some(e) = self.fail.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return Err(e);
        }
        Ok(self
            .map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(service.into(), account.into()))
            .cloned())
    }

    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
        if let Some(e) = self.fail.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            return Err(e);
        }
        self.map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((service.into(), account.into()), value.to_vec());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        self.map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&(service.into(), account.into()));
        Ok(())
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}
