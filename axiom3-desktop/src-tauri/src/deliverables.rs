//! Durable, disk-backed store for produced deliverables (产出物库) — the BACKUP layer that survives
//! the WebView localStorage quota (a few base64 figures blow ~5MB and used to wipe the task list).
//! Each completed task's FULL JSON (report + figures) is written to <app_data>/deliverables/<id>.json;
//! the UI lists metadata and can load / delete / rename entries. Cloud sync is future; this is local.
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

fn dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let d = base.join("deliverables");
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

// Untrusted id → a safe filename stem (no path traversal, bounded length).
fn safe_id(id: &str) -> String {
    let s: String = id.chars().filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_').take(120).collect();
    if s.is_empty() { "untitled".into() } else { s }
}

#[derive(serde::Serialize)]
pub struct DeliverableMeta {
    pub id: String,
    pub title: String,
    pub kind: String, // research | code | workflow …
    pub ts: f64,
    pub reviewed: bool,
    pub rounds: u32,   // follow-up rounds (artifact versions)
    pub experts: u32,  // agents involved: main + delegated sub-teams (best-effort)
    pub figures: u32,
    pub sources: u32,
    pub chars: u32,    // report length
    pub bytes: u64,    // on-disk size
}

#[derive(serde::Deserialize, Default)]
struct Parse {
    #[serde(default)]
    title: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    ts: f64,
    #[serde(default)]
    review: Option<String>,
    #[serde(default)]
    synthesis: Option<String>,
    #[serde(default)]
    sources: Vec<serde_json::Value>,
    #[serde(default)]
    steps: Vec<StepLite>,
    #[serde(default)]
    versions: Vec<serde_json::Value>,
}
#[derive(serde::Deserialize, Default)]
struct StepLite {
    #[serde(default)]
    role: String,
}

fn meta_from(id: &str, json: &str, bytes: u64) -> DeliverableMeta {
    let p: Parse = serde_json::from_str(json).unwrap_or_default();
    let syn = p.synthesis.unwrap_or_default();
    let figures = (syn.matches("```pyfig").count()
        + syn.matches("```mermaid").count()
        + syn.matches("```network").count()
        + syn.matches("```reaction").count()) as u32;
    let delegates = p.steps.iter().filter(|s| s.role == "delegate").count() as u32;
    DeliverableMeta {
        id: id.to_string(),
        title: if p.title.trim().is_empty() { "(未命名)".into() } else { p.title },
        kind: p.kind,
        ts: p.ts,
        reviewed: p.review.map(|r| !r.trim().is_empty()).unwrap_or(false),
        rounds: p.versions.len().max(1) as u32,
        experts: 1 + delegates, // main agent + one team per delegate call
        figures,
        sources: p.sources.len() as u32,
        chars: syn.chars().count() as u32,
        bytes,
    }
}

/// The placeholder the UI leaves behind when it strips a base64 figure for its localStorage cache.
const LEAN_MARK: &str = "[图形已存入产出物库]";

#[tauri::command]
pub fn deliverable_save(app: tauri::AppHandle, id: String, json: String) -> Result<(), String> {
    let d = dir(&app)?;
    let stem = safe_id(&id);
    let f = d.join(format!("{stem}.json"));
    // SAFETY NET — the disk store is the BACKUP: it only ever gets richer, never poorer.
    // A copy coming from the UI's LEAN localStorage cache has had its base64 figure payloads stripped,
    // so writing it wholesale would destroy the figures on every restart. But it may still carry NEWER
    // metadata (notably the `recap`, which is produced AFTER the first backup). So field-MERGE:
    // keep the figure-bearing fields from disk, take everything else from the incoming copy.
    let mut json = json;
    // ANTI-DEMOTION GUARD: the ~8s progress JOURNAL writes a LEAN, status:"running" snapshot of a
    // live run to disk (so a crash leaves a resumable record). Such a late-landing snapshot must NEVER
    // overwrite an already-FINISHED, figure-bearing record (done/no_report) — that would wipe the real
    // report+figures. Block exactly that demotion; every other transition (running→running fresh-wins,
    // running→cancelled, a resumed cancelled→running, anything→done) is allowed.
    if f.exists() {
        if let Ok(existing) = fs::read_to_string(&f) {
            let status_of = |s: &str| {
                serde_json::from_str::<serde_json::Value>(s)
                    .ok()
                    .and_then(|v| v.get("status").and_then(|x| x.as_str()).map(str::to_string))
            };
            let is_final = |s: &Option<String>| matches!(s.as_deref(), Some("done") | Some("no_report"));
            // Only block the SPECIFIC demotion we care about: a finished record being overwritten by an
            // explicit in-flight JOURNAL snapshot (status:"running"/"partial"). A save that OMITS `status`
            // (e.g. a metadata-only patch) is not a demotion — treat unknown/missing status as allowed so
            // it isn't silently dropped onto a done record.
            let is_stub = |s: &Option<String>| matches!(s.as_deref(), Some("running") | Some("partial"));
            if is_final(&status_of(&existing)) && is_stub(&status_of(&json)) {
                return Ok(()); // never demote a finished deliverable to an in-flight/partial snapshot
            }
        }
    }
    if f.exists() && json.contains(LEAN_MARK) {
        if let Ok(existing) = fs::read_to_string(&f) {
            if !existing.contains(LEAN_MARK) {
                match (
                    serde_json::from_str::<serde_json::Value>(&existing),
                    serde_json::from_str::<serde_json::Value>(&json),
                ) {
                    (Ok(mut old), Ok(new)) => {
                        if let (Some(o), Some(n)) = (old.as_object_mut(), new.as_object()) {
                            for (k, v) in n {
                                // these carry the base64 figures — never take them from a lean copy
                                if k == "synthesis" || k == "versions" || k == "steps" {
                                    continue;
                                }
                                o.insert(k.clone(), v.clone());
                            }
                            // The incoming copy OMITS `recapPending` (it's transient), and a merge only
                            // overwrites keys it carries — so an old stale `recapPending: true` would
                            // survive forever. Drop it explicitly.
                            o.remove("recapPending");
                            json = serde_json::to_string(&old).map_err(|e| e.to_string())?;
                        }
                    }
                    // unparseable on either side → keep the full record untouched
                    _ => return Ok(()),
                }
            }
        }
    }
    let tmp = d.join(format!(".{stem}.tmp"));
    fs::write(&tmp, json.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &f).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn deliverable_list(app: tauri::AppHandle) -> Result<Vec<DeliverableMeta>, String> {
    let d = dir(&app)?;
    let mut out = Vec::new();
    for e in fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
        if let Ok(json) = fs::read_to_string(&path) {
            // Hide in-flight / empty JOURNAL stubs from the 产出物库: a status:"running" progress
            // snapshot (or a record with no report and no rounds) is a resume aid, not a deliverable.
            // The stub still lives at <id>.json for openTask to load by id (the resume path).
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let has_report = v.get("synthesis").and_then(|s| s.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
                let has_rounds = v.get("versions").and_then(|a| a.as_array()).map(|a| !a.is_empty()).unwrap_or(false);
                if status == "running" || (!has_report && !has_rounds) {
                    continue;
                }
            }
            out.push(meta_from(&id, &json, bytes));
        }
    }
    out.sort_by(|a, b| b.ts.partial_cmp(&a.ts).unwrap_or(std::cmp::Ordering::Equal));
    Ok(out)
}

#[tauri::command]
pub fn deliverable_load(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let f = dir(&app)?.join(format!("{}.json", safe_id(&id)));
    fs::read_to_string(&f).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deliverable_delete(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let f = dir(&app)?.join(format!("{}.json", safe_id(&id)));
    if f.exists() {
        fs::remove_file(&f).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn deliverable_rename(app: tauri::AppHandle, id: String, title: String) -> Result<(), String> {
    let f = dir(&app)?.join(format!("{}.json", safe_id(&id)));
    let json = fs::read_to_string(&f).map_err(|e| e.to_string())?;
    let mut v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("title".into(), serde_json::Value::String(title));
    }
    let out = serde_json::to_string(&v).map_err(|e| e.to_string())?;
    fs::write(&f, out.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}
