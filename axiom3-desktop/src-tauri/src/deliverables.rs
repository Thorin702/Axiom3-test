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

#[tauri::command]
pub fn deliverable_save(app: tauri::AppHandle, id: String, json: String) -> Result<(), String> {
    let d = dir(&app)?;
    let stem = safe_id(&id);
    let f = d.join(format!("{stem}.json"));
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
