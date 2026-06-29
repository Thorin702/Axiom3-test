//! Secret storage for Axiom3.
//!
//! Secrets — the SQLCipher passphrase for each project's `durable.db` and the
//! user's BYO-API keys — live in the OS keychain (macOS Keychain / Windows
//! Credential Manager), never in plaintext on disk and never in the database the
//! key protects. The DB file on disk is useless without the keychain entry.
//!
//! [`SecretStore`] is the interface; [`KeyringSecretStore`] is the runtime backend
//! and [`MemorySecretStore`] is a deterministic in-memory backend for tests.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("random source failed: {0}")]
    Random(String),
}

pub type Result<T> = std::result::Result<T, SecretError>;

/// A named secret store. `get` returns `None` for a missing name (not an error).
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// OS-keychain-backed store. All entries share one service name; `name` is the
/// per-secret account (e.g. `db-key:<project>` or `api-key:anthropic`).
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self { service: service.into() }
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match keyring::Entry::new(&self.service, name)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        keyring::Entry::new(&self.service, name)?.set_password(value)?;
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        match keyring::Entry::new(&self.service, name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// In-memory store for tests (no keychain access).
#[derive(Default)]
pub struct MemorySecretStore {
    map: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.map.lock().unwrap().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.map.lock().unwrap().remove(name);
        Ok(())
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// The keychain account name for a project's SQLCipher passphrase.
pub fn db_key_name(project_id: &str) -> String {
    format!("db-key:{project_id}")
}

/// Fetch a project's SQLCipher passphrase, generating + persisting a fresh
/// 256-bit random key (64 hex chars) on first use. Idempotent: subsequent calls
/// return the same key, so the DB stays openable across runs.
pub fn get_or_create_db_key(store: &dyn SecretStore, project_id: &str) -> Result<String> {
    let name = db_key_name(project_id);
    if let Some(existing) = store.get(&name)? {
        return Ok(existing);
    }
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).map_err(|e| SecretError::Random(e.to_string()))?;
    let key = to_hex(&buf);
    store.set(&name, &key)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemorySecretStore::default();
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        s.delete("k").unwrap(); // deleting a missing key is not an error
    }

    #[test]
    fn db_key_is_generated_once_and_stable() {
        let s = MemorySecretStore::default();
        let k1 = get_or_create_db_key(&s, "proj-a").unwrap();
        assert_eq!(k1.len(), 64, "256-bit key → 64 hex chars");
        assert!(k1.bytes().all(|b| b.is_ascii_hexdigit()));
        let k2 = get_or_create_db_key(&s, "proj-a").unwrap();
        assert_eq!(k1, k2, "same project → same key across calls");
    }

    #[test]
    fn different_projects_get_different_keys() {
        let s = MemorySecretStore::default();
        let a = get_or_create_db_key(&s, "proj-a").unwrap();
        let b = get_or_create_db_key(&s, "proj-b").unwrap();
        assert_ne!(a, b, "per-project key isolation");
    }
}
