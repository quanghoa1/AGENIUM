//! Secrets never live in the root, the databases, git or logs in clear text (decision 10). Default backend:
//! encrypted vault whose master key is in the OS keyring (spike S5); every keyring call has a timeout.

pub mod keyring_backend;
pub mod scan;
pub mod store;

use std::path::Path;
use std::sync::Arc;

pub use keyring_backend::{KeyringBackend, MemoryKeyring, OsKeyring};
pub use scan::{scan_text, Finding, Redactor};
pub use store::{valid_name, KeyringStore, Secret, SecretStore, VaultStore};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("secret store unavailable: {0}")]
    Unavailable(String),
    #[error("keyring timeout: {0}")]
    Timeout(String),
    #[error("secret too large for the keyring: {0}")]
    TooLarge(String),
    #[error("invalid secret name {0:?} (letters, digits, / - _ . only)")]
    InvalidName(String),
    #[error("secret store corrupt: {0}")]
    Corrupt(String),
    #[error("io: {0}")]
    Io(String),
    #[error("keyring: {0}")]
    Backend(String),
}

/// Which backend `open_store` builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Vault,
    Keyring,
}

/// Builds the configured store. `root_id` distinguishes vault master keys of different roots on one machine.
pub fn open_store(
    backend: Backend,
    keyring: Arc<dyn KeyringBackend>,
    vault_path: &Path,
    root_id: &str,
) -> Arc<dyn SecretStore> {
    match backend {
        Backend::Vault => Arc::new(VaultStore::new(
            vault_path,
            keyring,
            &format!("vault-master/{root_id}"),
        )),
        Backend::Keyring => Arc::new(KeyringStore::new(keyring)),
    }
}
