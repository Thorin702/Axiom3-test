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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    /// The OS CSPRNG failed — keeps the typed source so callers can tell a fatal
    /// platform failure from a transient one.
    #[error("random source failed: {0}")]
    Random(#[from] getrandom::Error),
    /// Reading/writing the file-backed store failed.
    #[error("secret file io: {0}")]
    Io(#[from] std::io::Error),
    /// A name/value contained a control character the line-based file can't hold
    /// (rejected up front rather than silently dropped on write).
    #[error("secret name/value contains a tab, newline, or carriage return")]
    Invalid,
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

impl MemorySecretStore {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        // Recover from a poisoned lock rather than cascading panics.
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.map().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.map().insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.map().remove(name);
        Ok(())
    }
}

/// File-backed secret store: each secret is one `name\tvalue` line in a single local
/// file (owner-only permissions) under the app data dir.
///
/// This exists so an unsigned / ad-hoc-signed desktop build can avoid the OS keychain's
/// per-launch authorization prompt — the keychain ACL never matches a binary whose hash
/// changes on every rebuild, so macOS re-asks every launch. It is deliberately LESS
/// protected than the keychain (the secrets sit in a local file rather than the OS
/// credential store); the app makes this an explicit, opt-in convenience trade-off.
///
/// An optional read-through `migrate_from` source (typically the keychain) is consulted
/// on a miss and any hit is copied in, so a project already sealed with a keychain-held
/// DB key keeps opening: the key migrates on first access (one last keychain prompt),
/// then never prompts again.
pub struct FileSecretStore {
    path: std::path::PathBuf,
    map: Mutex<HashMap<String, String>>,
    migrate_from: Option<Box<dyn SecretStore>>,
}

impl FileSecretStore {
    /// Open the store at `path`; a missing file is an empty store.
    pub fn open(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let map = Self::load(&path);
        Self { path, map: Mutex::new(map), migrate_from: None }
    }

    /// Set a read-through source to migrate existing secrets from on a miss.
    pub fn migrating_from(mut self, src: Box<dyn SecretStore>) -> Self {
        self.migrate_from = Some(src);
        self
    }

    fn load(path: &std::path::Path) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('\t') {
                    map.insert(k.to_string(), v.to_string());
                }
            }
        }
        map
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Persist atomically (write a uniquely-named sibling temp file, then rename) with
    /// owner-only permissions, so the file is never a half-written secret store after a
    /// crash and secrets are never briefly world-readable. Callers guarantee names/values
    /// are control-char-free (enforced in `set`).
    fn persist(&self, map: &HashMap<String, String>) -> Result<()> {
        let mut out = String::new();
        for (k, v) in map {
            out.push_str(k);
            out.push('\t');
            out.push_str(v);
            out.push('\n');
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let base = self.path.file_name().and_then(|s| s.to_str()).unwrap_or("secrets");
        // A unique temp name (pid + counter) plus O_EXCL create means we never follow or
        // truncate a pre-planted symlink, and two writers never share a temp file. On the
        // rare AlreadyExists (a crash-left temp whose pid+seq gets reused) retry with a fresh
        // sequence number rather than failing the write.
        let mut last: Option<std::io::Error> = None;
        for _ in 0..8 {
            let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = self.path.with_file_name(format!(".{base}.{}.{seq}.tmp", std::process::id()));
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600); // 0600 from byte zero — no default-umask window
            }
            let mut f = match opts.open(&tmp) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => { last = Some(e); continue; }
                Err(e) => return Err(e.into()),
            };
            use std::io::Write;
            f.write_all(out.as_bytes())?;
            f.sync_all()?;
            drop(f);
            match std::fs::rename(&tmp, &self.path) {
                Ok(()) => {
                    // fsync the parent directory so the rename itself is crash-durable
                    // (otherwise a crash right after rename can lose it on some filesystems).
                    #[cfg(unix)]
                    if let Some(dir) = self.path.parent() {
                        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
                    }
                    return Ok(());
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp); // don't leave the temp behind on failure
                    return Err(e.into());
                }
            }
        }
        Err(last.map(Into::into).unwrap_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not create temp secret file").into()
        }))
    }

    /// Insert `value` for `name` only if it is still absent on disk (under the write lock),
    /// returning whatever value ends up stored — so a concurrent `set` of a newer value is
    /// never clobbered by a slower read-through migration.
    fn rmw_get_or_insert(&self, name: &str, value: &str) -> Result<String> {
        let _lock = FILE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut disk = Self::load(&self.path);
        let stored = if let Some(existing) = disk.get(name) {
            existing.clone()
        } else {
            disk.insert(name.to_string(), value.to_string());
            self.persist(&disk)?;
            value.to_string()
        };
        *self.map() = disk;
        Ok(stored)
    }
}

/// Distinguishes concurrent temp files written by this process (see `persist`).
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Serializes writes to the secret file across ALL `FileSecretStore` instances in the
/// process, so concurrent migrations (e.g. the DB key and a provider key at launch, each
/// from its own store handle) can't interleave read-modify-write and clobber one another.
static FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

impl FileSecretStore {
    /// Read-modify-write the on-disk file under the process-wide write lock: reload the
    /// current contents (so we merge with whatever another handle just wrote), apply
    /// `mutate`, persist atomically, and refresh this handle's cache.
    fn rmw(&self, mutate: impl FnOnce(&mut HashMap<String, String>)) -> Result<()> {
        let _lock = FILE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut disk = Self::load(&self.path);
        mutate(&mut disk);
        self.persist(&disk)?;
        *self.map() = disk;
        Ok(())
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        if let Some(v) = self.map().get(name) {
            return Ok(Some(v.clone()));
        }
        // Cache miss: reload from disk in case another store handle wrote this key since
        // we opened, before deciding to migrate.
        {
            let fresh = Self::load(&self.path);
            let mut cache = self.map();
            *cache = fresh;
            if let Some(v) = cache.get(name) {
                return Ok(Some(v.clone()));
            }
        }
        // Still a miss: read through the migration source (keychain) and copy any hit in,
        // so we adopt an already-created key exactly once instead of minting a conflicting
        // one (which would seal the DB with a key nobody holds).
        if let Some(src) = &self.migrate_from {
            if let Some(v) = src.get(name)? {
                // Insert-if-absent so a concurrent set() of a newer value wins over the
                // (older) migrated value instead of being clobbered.
                let stored = self.rmw_get_or_insert(name, &v)?;
                return Ok(Some(stored));
            }
        }
        Ok(None)
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        // The line format can't represent these; reject up front so we never silently
        // drop a value while reporting success.
        let bad = |s: &str| s.contains(|c| c == '\t' || c == '\n' || c == '\r');
        if bad(name) || bad(value) {
            return Err(SecretError::Invalid);
        }
        let (name, value) = (name.to_string(), value.to_string());
        self.rmw(move |m| { m.insert(name, value); })
    }

    fn delete(&self, name: &str) -> Result<()> {
        let name = name.to_string();
        self.rmw(move |m| { m.remove(&name); })
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

/// Process-wide guard so two threads doing first-run init don't interleave
/// generate/set and mint conflicting keys. Cross-process first-run races are
/// prevented by running the app single-instance (enforced in the shell, P0.6).
static DB_KEY_INIT_LOCK: Mutex<()> = Mutex::new(());

/// Fetch a project's SQLCipher passphrase, generating + persisting a fresh
/// 256-bit random key (64 hex chars) on first use.
///
/// Idempotent and race-safe within a process: first-run generation is serialized
/// under [`DB_KEY_INIT_LOCK`] with a double-check, so a second thread adopts the
/// first thread's key instead of overwriting it (which would seal the DB with one
/// key while the keychain holds another — unrecoverable).
pub fn get_or_create_db_key(store: &dyn SecretStore, project_id: &str) -> Result<String> {
    let name = db_key_name(project_id);
    // Fast path — already created, no lock needed.
    if let Some(existing) = store.get(&name)? {
        return Ok(existing);
    }
    let _guard = DB_KEY_INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // Re-check under the lock: another thread may have just created it.
    if let Some(existing) = store.get(&name)? {
        return Ok(existing);
    }
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
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

    fn tmp_path(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        p.push(format!("axiom-secrets-{tag}-{n}.local"));
        p
    }

    #[test]
    fn file_store_roundtrip_and_persists_across_reopen() {
        let path = tmp_path("rt");
        {
            let s = FileSecretStore::open(&path);
            assert_eq!(s.get("k").unwrap(), None);
            s.set("db-key:default", "deadbeef").unwrap();
            s.set("api-key:deepseek", "sk-abc.DEF-123").unwrap();
            assert_eq!(s.get("db-key:default").unwrap().as_deref(), Some("deadbeef"));
        }
        // Reopen from disk — secrets survive the process boundary (no keychain).
        {
            let s = FileSecretStore::open(&path);
            assert_eq!(s.get("db-key:default").unwrap().as_deref(), Some("deadbeef"));
            assert_eq!(s.get("api-key:deepseek").unwrap().as_deref(), Some("sk-abc.DEF-123"));
            s.delete("db-key:default").unwrap();
            assert_eq!(s.get("db-key:default").unwrap(), None);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_migrates_from_source_once() {
        let path = tmp_path("mig");
        // A source (stand-in for the keychain) already holds a DB key.
        let src = MemorySecretStore::default();
        src.set("db-key:default", "keychainkey").unwrap();

        let s = FileSecretStore::open(&path).migrating_from(Box::new(src));
        // First read misses the file, migrates the key from the source, and returns it.
        assert_eq!(s.get("db-key:default").unwrap().as_deref(), Some("keychainkey"));

        // It was copied into the file, so a fresh handle WITHOUT the source still has it
        // (proving we no longer depend on the keychain after the one-time migration).
        let s2 = FileSecretStore::open(&path);
        assert_eq!(s2.get("db-key:default").unwrap().as_deref(), Some("keychainkey"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_concurrent_writes_from_separate_handles_all_survive() {
        let path = tmp_path("conc");
        // Many separate handles (as different Tauri commands would) writing distinct keys
        // at once must all land — the read-modify-write merge under the process lock
        // prevents one handle's snapshot from clobbering another's.
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let p = path.clone();
                std::thread::spawn(move || {
                    let s = FileSecretStore::open(&p);
                    s.set(&format!("api-key:p{i}"), &format!("v{i}")).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let s = FileSecretStore::open(&path);
        for i in 0..16 {
            assert_eq!(s.get(&format!("api-key:p{i}")).unwrap().as_deref(), Some(format!("v{i}").as_str()), "key p{i} was lost to a race");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn get_or_create_db_key_is_stable_on_file_store() {
        let path = tmp_path("stable");
        let k1 = { let s = FileSecretStore::open(&path); get_or_create_db_key(&s, "default").unwrap() };
        // A brand-new handle reads the same key back — a rebuilt binary won't re-key the DB.
        let k2 = { let s = FileSecretStore::open(&path); get_or_create_db_key(&s, "default").unwrap() };
        assert_eq!(k1, k2, "key stable across reopen → DB stays decryptable");
        assert_eq!(k1.len(), 64);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn concurrent_first_run_db_key_converges_on_file_store() {
        // Two `open_project` calls for the SAME new project race, each on its own file-store
        // handle. All must converge on ONE key (else the DB gets sealed with a key nobody
        // holds → undecryptable). The process-wide init lock + disk-reload-before-recheck
        // make the second thread adopt the first's key rather than mint its own.
        let path = tmp_path("converge");
        let results: Vec<String> = (0..8)
            .map(|_| {
                let p = path.clone();
                std::thread::spawn(move || {
                    let s = FileSecretStore::open(&p);
                    get_or_create_db_key(&s, "default").unwrap()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let first = &results[0];
        assert!(results.iter().all(|k| k == first), "threads minted divergent DB keys: {results:?}");
        // And the persisted file agrees with what every thread returned.
        let s = FileSecretStore::open(&path);
        assert_eq!(s.get("db-key:default").unwrap().as_ref(), Some(first));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_store_rejects_control_chars_instead_of_dropping() {
        let path = tmp_path("ctrl");
        let s = FileSecretStore::open(&path);
        assert!(matches!(s.set("api-key:x", "line1\nline2"), Err(SecretError::Invalid)));
        assert!(matches!(s.set("bad\tname", "v"), Err(SecretError::Invalid)));
        // A rejected write leaves nothing half-stored.
        assert_eq!(s.get("api-key:x").unwrap(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn file_store_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_path("perms");
        let s = FileSecretStore::open(&path);
        s.set("db-key:default", "deadbeef").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secret file must be owner-read/write only, got {mode:o}");
        let _ = std::fs::remove_file(&path);
    }
}
