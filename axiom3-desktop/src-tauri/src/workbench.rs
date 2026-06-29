//! Per-project workbench: opens and owns the encrypted system-of-record and
//! exposes it to the Tauri shell.
//!
//! Every store call is SYNC — rusqlite (`SqliteStore`) and LanceDB (bridged via
//! its own runtime inside `axiom-vector`). So every command runs the store work on
//! the blocking pool (`spawn_blocking`) and never on the async UI runtime; calling
//! a LanceStore method on the async runtime would panic (nested `block_on`), and a
//! rusqlite mutex would stall the executor. This is the `spawn_blocking` boundary
//! that storage design "decision B" was built for.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axiom_core::store::{EventLog, VectorStore};
use axiom_secrets::{get_or_create_db_key, KeyringSecretStore, SecretStore};
use axiom_store::SqliteStore;
use axiom_vector::LanceStore;
use serde::Serialize;
use serde_json::Value;
use tauri::Manager;

/// Keychain service name under which all Axiom3 secrets live.
const KEYCHAIN_SERVICE: &str = "axiom3";

/// Dimension for the vector self-test only. The project's *persistent* vector
/// store is opened once the embedding model fixes the real dimension (deferred) —
/// we don't bake a guessed dim into on-disk state. P0.6 only proves the LanceStore
/// wiring survives the shell's `spawn_blocking` boundary.
const SELFTEST_DIM: usize = 8;

/// Open per-project state. Held behind an `Arc` in [`AppState`] so commands can
/// clone it out and move it onto a blocking thread.
pub struct Workbench {
    project_id: String,
    root: PathBuf,
    durable: SqliteStore,
}

/// A snapshot of the open project for the UI.
#[derive(Serialize)]
pub struct ProjectStatus {
    pub project_id: String,
    pub root: String,
    pub event_count: usize,
    pub chain_head: Option<String>,
    pub chain_ok: bool,
}

/// The result of appending one event.
#[derive(Serialize)]
pub struct EventInfo {
    pub seq: i64,
    pub ts: i64,
    pub actor: String,
    pub op: String,
    pub entry_hash: String,
}

impl Workbench {
    /// Open (creating if needed) the project at `root`, sealing `durable.db` with
    /// the project's keychain-managed SQLCipher key from `secrets`.
    pub fn open(root: PathBuf, project_id: String, secrets: &dyn SecretStore) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&root)?;
        let key = get_or_create_db_key(secrets, &project_id)?;
        let durable = SqliteStore::open(root.join("durable.db"), &key)?;
        Ok(Self { project_id, root, durable })
    }

    pub fn id(&self) -> &str {
        &self.project_id
    }

    pub fn append_event(&self, actor: &str, op: &str, payload: &Value) -> anyhow::Result<EventInfo> {
        let e = self.durable.append_event(actor, op, payload)?;
        Ok(EventInfo { seq: e.seq, ts: e.ts, actor: e.actor, op: e.op, entry_hash: e.entry_hash })
    }

    pub fn verify_chain(&self) -> anyhow::Result<bool> {
        Ok(self.durable.verify_chain()?)
    }

    pub fn status(&self) -> anyhow::Result<ProjectStatus> {
        Ok(ProjectStatus {
            project_id: self.project_id.clone(),
            root: self.root.display().to_string(),
            event_count: self.durable.replay()?.len(),
            chain_head: self.durable.head_hash()?,
            chain_ok: self.durable.verify_chain()?,
        })
    }

    /// Prove the LanceDB vector tier works through the shell's blocking boundary:
    /// open an ephemeral store, upsert a synthetic vector, and read it back. The
    /// store lives inside the (access-controlled) project dir, not a predictable
    /// world-readable temp path.
    pub fn vector_self_test(&self) -> anyhow::Result<String> {
        let dir = self.root.join(".vector-selftest");
        let _ = std::fs::remove_dir_all(&dir); // clear any stale leftover first
        let outcome = vector_round_trip(&dir);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!("warning: vector self-test cleanup failed for {dir:?}: {e}");
        }
        outcome
    }
}

/// Open an ephemeral LanceDB store, upsert a synthetic vector, and read it back —
/// proving the vector tier works through the shell's blocking boundary.
fn vector_round_trip(dir: &Path) -> anyhow::Result<String> {
    let store = LanceStore::open(dir, SELFTEST_DIM)?;
    store.upsert("probe", &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])?;
    let hits = store.search(&[0.0, 0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 1)?;
    hits.into_iter()
        .next()
        .map(|(id, dist)| format!("nearest={id} dist={dist:.4}"))
        .ok_or_else(|| anyhow::anyhow!("vector self-test returned no hits"))
}

/// Shell-wide state: the currently open project, if any.
#[derive(Default)]
pub struct AppState {
    current: Mutex<Option<Arc<Workbench>>>,
}

impl AppState {
    fn get(&self) -> Result<Arc<Workbench>, String> {
        self.current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| "no project is open".to_string())
    }
    fn set(&self, wb: Arc<Workbench>) {
        *self.current.lock().unwrap_or_else(|p| p.into_inner()) = Some(wb);
    }
    fn clear(&self) {
        *self.current.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    fn current_id(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|w| w.id().to_string())
    }
}

/// Map a project name to a readable filesystem-safe slug (lossy: many names → one
/// slug). NOT an identity on its own — see [`project_id_for`].
fn sanitize_project_id(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "project".to_string()
    } else {
        s
    }
}

/// A stable, collision-resistant project id: a readable slug plus a short hash of
/// the exact (trimmed) name. Distinct names never share a directory or DB key, so
/// two projects can never silently co-mingle into one encrypted store.
fn project_id_for(name: &str) -> String {
    let slug = sanitize_project_id(name);
    let hash = blake3::hash(name.trim().as_bytes()).to_hex();
    format!("{slug}-{}", &hash[..8])
}

#[tauri::command]
pub async fn open_project(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<ProjectStatus, String> {
    let project_id = project_id_for(&name);
    // Already open — refresh its status instead of opening a second connection to
    // the same durable.db.
    if state.current_id().as_deref() == Some(project_id.as_str()) {
        let wb = state.get()?;
        return tauri::async_runtime::spawn_blocking(move || wb.status().map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())?;
    }
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let root = base.join("projects").join(&project_id);
    let (wb, status) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Arc<Workbench>, ProjectStatus), String> {
            let secrets = KeyringSecretStore::new(KEYCHAIN_SERVICE);
            let wb = Workbench::open(root, project_id, &secrets).map_err(|e| e.to_string())?;
            let status = wb.status().map_err(|e| e.to_string())?;
            Ok((Arc::new(wb), status))
        },
    )
    .await
    .map_err(|e| e.to_string())??;
    state.set(wb);
    Ok(status)
}

#[tauri::command]
pub async fn project_status(state: tauri::State<'_, AppState>) -> Result<ProjectStatus, String> {
    let wb = state.get()?;
    tauri::async_runtime::spawn_blocking(move || wb.status().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn append_event(
    state: tauri::State<'_, AppState>,
    actor: String,
    op: String,
    payload: String,
) -> Result<EventInfo, String> {
    let wb = state.get()?;
    let value: Value = if payload.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&payload).map_err(|e| format!("payload must be JSON: {e}"))?
    };
    tauri::async_runtime::spawn_blocking(move || {
        wb.append_event(&actor, &op, &value).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn verify_chain(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let wb = state.get()?;
    tauri::async_runtime::spawn_blocking(move || wb.verify_chain().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn vector_self_test(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let wb = state.get()?;
    tauri::async_runtime::spawn_blocking(move || wb.vector_self_test().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn close_project(state: tauri::State<'_, AppState>) {
    state.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axiom_secrets::MemorySecretStore;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn open_appends_and_verifies_chain() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(
            dir.path().join("proj"),
            "proj-test".into(),
            &MemorySecretStore::default(),
        )
        .unwrap();

        for i in 0..3 {
            wb.append_event("tester", "note", &json!({ "i": i })).unwrap();
        }
        let st = wb.status().unwrap();
        assert_eq!(st.event_count, 3);
        assert!(st.chain_ok);
        assert!(st.chain_head.is_some());
    }

    #[test]
    fn persists_across_reopen_with_same_key() {
        // The SAME secret store must back both opens — a fresh key would seal the
        // db with one passphrase and fail to decrypt on reopen.
        let secrets = MemorySecretStore::default();
        let dir = tempdir().unwrap();
        let root = dir.path().join("proj");
        {
            let wb = Workbench::open(root.clone(), "p".into(), &secrets).unwrap();
            wb.append_event("tester", "init", &json!({ "v": 1 })).unwrap();
        }
        let wb = Workbench::open(root.clone(), "p".into(), &secrets).unwrap();
        let st = wb.status().unwrap();
        assert_eq!(st.event_count, 1, "event survived reopen");
        assert!(st.chain_ok);
    }

    #[test]
    fn vector_self_test_round_trips() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(
            dir.path().join("proj"),
            "vec-test".into(),
            &MemorySecretStore::default(),
        )
        .unwrap();
        let out = wb.vector_self_test().unwrap();
        assert!(out.contains("nearest=probe"), "got: {out}");
    }

    #[test]
    fn sanitize_project_id_is_filesystem_safe() {
        assert_eq!(sanitize_project_id("  My Project! "), "my-project");
        assert_eq!(sanitize_project_id("研究 2026"), "2026");
        assert_eq!(sanitize_project_id("***"), "project");
    }

    #[test]
    fn distinct_names_get_distinct_ids_but_trim_is_stable() {
        // Names that slugify identically must still get DISTINCT ids — otherwise
        // two projects would share one encrypted db + key (silent co-mingling).
        assert_ne!(project_id_for("My Project"), project_id_for("my-project"));
        assert_ne!(project_id_for("Research Q1"), project_id_for("Research/Q1"));
        // …but leading/trailing whitespace is not a new project.
        assert_eq!(project_id_for("  My Project "), project_id_for("My Project"));
        // The id is still filesystem-safe (slug + hex hash).
        assert!(project_id_for("My Project!")
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    }
}
