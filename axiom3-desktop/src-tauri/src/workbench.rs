//! Per-project workbench: opens and owns the encrypted system-of-record and
//! exposes it to the Tauri shell.
//!
//! Every store call is SYNC — rusqlite (`SqliteStore`) and LanceDB (bridged via
//! its own runtime inside `axiom-vector`). So every command runs the store work on
//! the blocking pool (`spawn_blocking`) and never on the async UI runtime; calling
//! a LanceStore method on the async runtime would panic (nested `block_on`), and a
//! rusqlite mutex would stall the executor. This is the `spawn_blocking` boundary
//! that storage design "decision B" was built for.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axiom_core::model::{new_id, Independence, Verdict, VerdictStatus, Verifier, VerifierKind};
use axiom_core::store::{DurableStore, EventLog, VectorStore};
use axiom_core::verify::{verify_draft, ClaimReport};
use axiom_runtime::fleet::{verify_bearing_claim, FleetMember, FleetVerdict, MultiProviderRigBackend};
use axiom_runtime::Provider;
use axiom_secrets::{get_or_create_db_key, KeyringSecretStore, SecretStore};
use axiom_store::SqliteStore;
use axiom_vector::LanceStore;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;

/// Keychain service name under which all Axiom3 secrets live.
const KEYCHAIN_SERVICE: &str = "axiom3";

/// Dimension for the vector self-test only. The project's *persistent* vector
/// store is opened once the embedding model fixes the real dimension (deferred) —
/// we don't bake a guessed dim into on-disk state. P0.6 only proves the LanceStore
/// wiring survives the shell's `spawn_blocking` boundary.
const SELFTEST_DIM: usize = 8;

/// Default cap on bearing claims given full cross-vendor fleet verification per
/// check (each costs API calls across vendors). Override with `AXIOM_FLEET_MAX`.
/// Beyond the cap, claims stay at the deterministic tier.
const DEFAULT_FLEET_MAX: usize = 24;

fn max_fleet_claims() -> usize {
    std::env::var("AXIOM_FLEET_MAX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_FLEET_MAX)
}

/// Build the verification fleet from whichever providers have a key in the
/// environment — a vendor with no key simply isn't in the fleet.
fn fleet_from_env() -> Vec<FleetMember> {
    let candidates = [
        (Provider::Anthropic, "claude-haiku-4-5-20251001"),
        (Provider::OpenAi, "gpt-4o-mini"),
        (Provider::DeepSeek, "deepseek-chat"),
        (Provider::Gemini, "gemini-1.5-flash"),
        (Provider::Qwen, "qwen-plus"),
    ];
    candidates
        .iter()
        .filter(|(p, _)| std::env::var(p.env_var()).is_ok())
        .map(|(p, model)| FleetMember {
            family: p.family().to_string(),
            provider: p.family().to_string(),
            model: (*model).to_string(),
        })
        .collect()
}

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

/// One deterministic checker observation, for the UI.
#[derive(Serialize)]
pub struct FindingView {
    pub checker: String,
    pub message: String,
    pub excerpt: Option<String>,
}

/// One model vendor's judgement, for the UI.
#[derive(Serialize)]
pub struct VendorView {
    pub family: String,
    pub model: String,
    pub status: String,
    /// The model's stated reason (its `REASON:` line) — why it judged this way.
    pub reasoning: String,
}

/// Pull a concise reason out of a model's reply: drop the `VERDICT:` line, strip a
/// leading `REASON:` label, collapse to one line, and cap the length.
fn short_reason(reply: &str) -> String {
    let body = reply
        .lines()
        .filter(|l| !l.trim().to_ascii_lowercase().starts_with("verdict:"))
        .collect::<Vec<_>>()
        .join(" ");
    let body = body.trim();
    let body = body
        .strip_prefix("REASON:")
        .or_else(|| body.strip_prefix("Reason:"))
        .or_else(|| body.strip_prefix("reason:"))
        .unwrap_or(body)
        .trim();
    let truncated: String = body.chars().take(240).collect();
    if body.chars().count() > 240 {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// The cross-vendor fleet result for a bearing claim.
#[derive(Serialize)]
pub struct FleetView {
    pub status: String,
    pub independence: String,
    pub reason: String,
    pub vendors: Vec<VendorView>,
}

/// One adjudicated claim in the ledger.
#[derive(Serialize)]
pub struct ClaimEntry {
    pub claim_id: String,
    pub text: String,
    pub claim_type: String,
    pub is_bearing: bool,
    /// Final 5-state verdict code (fleet result for bearing claims, else deterministic).
    pub status: String,
    /// Stable machine reason code(s).
    pub reason: Option<String>,
    pub findings: Vec<FindingView>,
    /// Cross-vendor breakdown, when a bearing claim was fleet-verified.
    pub fleet: Option<FleetView>,
}

/// The result of checking a draft: the persisted claim ledger.
#[derive(Serialize)]
pub struct DraftCheck {
    /// The event seq that recorded this check (audit anchor).
    pub event_seq: i64,
    pub total: usize,
    /// How many claims carry at least one deterministic risk flag.
    pub flagged: usize,
    /// How many bearing claims got cross-vendor fleet verification.
    pub fleet_verified: usize,
    pub entries: Vec<ClaimEntry>,
}

/// A claim's deterministic report joined with its final adjudicated verdict.
struct Combined<'a> {
    report: &'a ClaimReport,
    status: VerdictStatus,
    reason: Option<String>,
    independence: Independence,
    fleet: Option<FleetView>,
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

    /// Run the deterministic verification engine over a draft and PERSIST the
    /// result: one `draft.checked` event anchors a batch of immutable claims +
    /// verdicts in the encrypted store. Returns the ledger for the UI.
    /// The deterministic tier: extract claims + run the always-on checkers. Pure +
    /// fast; the (async, networked) fleet runs between this and [`Self::persist_check`].
    pub fn deterministic_reports(&self, draft: &str) -> Vec<ClaimReport> {
        verify_draft(&self.project_id, draft)
    }

    /// Combine the deterministic reports with the per-claim fleet verdicts, then
    /// ATOMICALLY persist (one audit event + verifier + all claims + verdicts).
    pub fn persist_check(
        &self,
        reports: Vec<ClaimReport>,
        fleet: HashMap<String, FleetVerdict>,
    ) -> anyhow::Result<DraftCheck> {
        let used_fleet = !fleet.is_empty();
        let verifier = Verifier {
            id: new_id(),
            kind: if used_fleet { VerifierKind::Model } else { VerifierKind::Deterministic },
            model_id: None,
            model_family: None,
            version: Some(axiom_core::version().to_string()),
            prompt_hash: None,
            role: Some(if used_fleet { "adjudicate" } else { "routine_verify" }.to_string()),
        };

        // Final per-claim verdict = fleet result for bearing claims, deterministic
        // otherwise — with one conservative rule: a deterministic risk flag (e.g. an
        // over-claim) BLOCKS a model-only `verified`, because models judging from
        // parametric knowledge can't confirm novelty/priority. Refuted passes through.
        let combined: Vec<Combined> = reports
            .iter()
            .map(|r| match fleet.get(&r.claim.id) {
                Some(fv) => {
                    let has_flags = !r.findings.is_empty();
                    let status = if fv.status == VerdictStatus::Verified && has_flags {
                        VerdictStatus::Uncertain
                    } else {
                        fv.status
                    };
                    // Complete machine-coded reason: the fleet reason + (block note
                    // if downgraded) + every deterministic flag code — the persisted
                    // audit must omit nothing that shaped the final verdict.
                    let mut parts = vec![fv.reason.clone()];
                    if status != fv.status {
                        parts.push("deterministic_flags_block_verify".to_string());
                    }
                    let mut det: Vec<&str> = r.findings.iter().map(|f| f.checker).collect();
                    det.sort_unstable();
                    det.dedup();
                    parts.extend(det.into_iter().map(String::from));
                    let reason = parts.join(",");
                    let view = FleetView {
                        status: fv.status.code().to_string(),
                        independence: fv.independence.code().to_string(),
                        reason: fv.reason.clone(),
                        vendors: fv
                            .judgements
                            .iter()
                            .map(|j| VendorView {
                                family: j.family.clone(),
                                model: j.model.clone(),
                                status: j.status.code().to_string(),
                                reasoning: short_reason(&j.reasoning),
                            })
                            .collect(),
                    };
                    Combined { report: r, status, reason: Some(reason), independence: fv.independence, fleet: Some(view) }
                }
                None => Combined {
                    report: r,
                    status: r.status,
                    reason: r.reason.clone(),
                    independence: Independence::LocalOnly,
                    fleet: None,
                },
            })
            .collect();

        let items: Vec<(axiom_core::model::Claim, Verdict)> = combined
            .iter()
            .map(|c| {
                let verdict = Verdict {
                    id: new_id(),
                    claim_id: c.report.claim.id.clone(),
                    verifier_id: verifier.id.clone(),
                    status: c.status,
                    evidence_redacted: false,
                    calibrated_confidence: None,
                    uncertain_reason: c.reason.clone(),
                    independence: Some(c.independence),
                    revises_verdict_id: None,
                    created_event_seq: 0, // assigned atomically below
                };
                (c.report.claim.clone(), verdict)
            })
            .collect();

        // Audit payload carries fleet provenance (which model said what).
        let provenance: Vec<Value> = combined
            .iter()
            .map(|c| {
                json!({
                    "claim_id": c.report.claim.id,
                    "status": c.status.code(),
                    // The fleet's own conclusion, recorded alongside the final status
                    // so a downgrade is explicit (not a hidden discrepancy).
                    "fleet_status": c.fleet.as_ref().map(|v| v.status.clone()),
                    "independence": c.independence.code(),
                    "vendors": c.fleet.as_ref().map(|v| v.vendors.iter()
                        .map(|x| json!({"family": x.family, "model": x.model, "status": x.status}))
                        .collect::<Vec<_>>()),
                })
            })
            .collect();

        let fleet_verified = fleet.len();
        let event = self.durable.record_verification(
            "engine",
            "draft.checked",
            &json!({ "claims": reports.len(), "fleet_verified": fleet_verified, "verdicts": provenance }),
            &verifier,
            &items,
        )?;

        let entries: Vec<ClaimEntry> = combined
            .into_iter()
            .map(|c| ClaimEntry {
                claim_id: c.report.claim.id.clone(),
                text: c.report.claim.text.clone(),
                claim_type: c.report.claim.claim_type.code().to_string(),
                is_bearing: c.report.claim.is_bearing,
                status: c.status.code().to_string(),
                reason: c.reason,
                findings: c
                    .report
                    .findings
                    .iter()
                    .map(|f| FindingView {
                        checker: f.checker.to_string(),
                        message: f.message.clone(),
                        excerpt: f.excerpt.clone(),
                    })
                    .collect(),
                fleet: c.fleet,
            })
            .collect();

        let flagged = entries.iter().filter(|e| !e.findings.is_empty()).count();
        Ok(DraftCheck {
            event_seq: event.seq,
            total: entries.len(),
            flagged,
            fleet_verified,
            entries,
        })
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
pub async fn check_draft(
    state: tauri::State<'_, AppState>,
    draft: String,
) -> Result<DraftCheck, String> {
    let wb = state.get()?;

    // 1. Deterministic tier (CPU) — off the async runtime.
    let reports = {
        let wb = wb.clone();
        tauri::async_runtime::spawn_blocking(move || wb.deterministic_reports(&draft))
            .await
            .map_err(|e| e.to_string())?
    };

    // 2. Cross-vendor fleet verification for bearing claims (async network I/O).
    //    Needs ≥2 vendor keys in the env; otherwise the gate keeps them Uncertain.
    let members = std::sync::Arc::new(fleet_from_env());
    let mut fleet_verdicts = HashMap::new();
    if members.len() >= 2 {
        use futures::StreamExt;
        // Owned (id, text) pairs so each future borrows nothing from this scope —
        // the Tauri command macro needs the future general over lifetimes.
        let bearing: Vec<(String, String)> = reports
            .iter()
            .filter(|r| r.claim.is_bearing)
            .take(max_fleet_claims())
            .map(|r| (r.claim.id.clone(), r.claim.text.clone()))
            .collect();
        // Verify up to 6 bearing claims concurrently (each fleets across vendors),
        // so a whole paper's claims don't run strictly one-at-a-time.
        let verdicts: Vec<(String, FleetVerdict)> = futures::stream::iter(bearing)
            .map(|(id, text)| {
                let members = members.clone();
                async move {
                    let backend = MultiProviderRigBackend;
                    (id, verify_bearing_claim(&text, &members, &backend).await)
                }
            })
            .buffer_unordered(6)
            .collect()
            .await;
        fleet_verdicts.extend(verdicts);
    }

    // 3. Combine + atomically persist — off the async runtime.
    let wb2 = wb.clone();
    tauri::async_runtime::spawn_blocking(move || wb2.persist_check(reports, fleet_verdicts))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Extract plain text from an uploaded document (pdf / docx / txt / md) so it can
/// be dropped into the draft checker. Pure file work — runs off the async runtime.
#[tauri::command]
pub async fn import_document(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        axiom_ingest::extract_text(std::path::Path::new(&path)).map_err(|e| e.to_string())
    })
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
    fn check_draft_persists_claims_and_verdicts() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(
            dir.path().join("proj"),
            "draft-test".into(),
            &MemorySecretStore::default(),
        )
        .unwrap();

        let draft = "This is the first-ever room-temperature synthesis. The yield was 94%.";
        // Deterministic only (no fleet verdicts).
        let reports = wb.deterministic_reports(draft);
        let check = wb.persist_check(reports, HashMap::new()).unwrap();

        assert_eq!(check.total, 2, "two claims extracted");
        assert!(check.flagged >= 1, "at least the over-claim is flagged");
        // deterministic tier: every verdict is uncertain
        assert!(check.entries.iter().all(|e| e.status == "uncertain"));
        // the check is recorded in the audit chain and the chain stays intact
        assert!(check.event_seq >= 1);
        assert!(wb.verify_chain().unwrap());
        // verdicts were persisted and are retrievable per claim
        let first = &check.entries[0];
        let verdicts = wb.durable.verdicts_for_claim(&first.claim_id).unwrap();
        assert_eq!(verdicts.len(), 1, "one verdict persisted per claim");
    }

    #[test]
    fn fleet_verdict_becomes_the_final_status() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(dir.path().join("p"), "fleet-test".into(), &MemorySecretStore::default())
            .unwrap();
        let reports = wb.deterministic_reports("Therefore the treatment causes recovery.");
        assert_eq!(reports.len(), 1);
        assert!(reports[0].findings.is_empty(), "no deterministic flag on this one");
        let mut fleet = HashMap::new();
        fleet.insert(
            reports[0].claim.id.clone(),
            FleetVerdict {
                status: VerdictStatus::Verified,
                independence: Independence::MultiFamily,
                reason: "multi_family_supported".into(),
                judgements: vec![],
            },
        );
        let check = wb.persist_check(reports, fleet).unwrap();
        assert_eq!(check.fleet_verified, 1);
        assert_eq!(check.entries[0].status, "verified", "a clean bearing claim takes the fleet verdict");
        assert!(check.entries[0].fleet.is_some());
    }

    #[test]
    fn deterministic_flag_blocks_a_model_only_verify() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(dir.path().join("p"), "block-test".into(), &MemorySecretStore::default())
            .unwrap();
        let reports = wb.deterministic_reports("We present the first-ever room-temperature superconductor.");
        assert!(!reports[0].findings.is_empty(), "over-claim is flagged");
        let mut fleet = HashMap::new();
        fleet.insert(
            reports[0].claim.id.clone(),
            FleetVerdict {
                status: VerdictStatus::Verified,
                independence: Independence::MultiFamily,
                reason: "multi_family_supported".into(),
                judgements: vec![],
            },
        );
        let check = wb.persist_check(reports, fleet).unwrap();
        // Models can't confirm a novelty claim from parametric knowledge → blocked.
        assert_eq!(check.entries[0].status, "uncertain");
        assert!(check.entries[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("deterministic_flags_block_verify"));
    }

    #[test]
    fn persisted_reason_keeps_both_layers_under_fleet_uncertain() {
        let dir = tempdir().unwrap();
        let wb = Workbench::open(dir.path().join("p"), "reason-test".into(), &MemorySecretStore::default())
            .unwrap();
        let reports = wb.deterministic_reports("We present the first-ever room-temperature superconductor.");
        assert!(!reports[0].findings.is_empty(), "over-claim flagged");
        let mut fleet = HashMap::new();
        fleet.insert(
            reports[0].claim.id.clone(),
            FleetVerdict {
                status: VerdictStatus::Uncertain,
                independence: Independence::MultiFamily,
                reason: "insufficient_corroboration".into(),
                judgements: vec![],
            },
        );
        let check = wb.persist_check(reports, fleet).unwrap();
        let reason = check.entries[0].reason.as_deref().unwrap();
        // Both the fleet reason AND the deterministic flag survive into the audit.
        assert!(reason.contains("insufficient_corroboration"), "fleet reason kept: {reason}");
        assert!(reason.contains("overclaim"), "deterministic flag kept: {reason}");
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
