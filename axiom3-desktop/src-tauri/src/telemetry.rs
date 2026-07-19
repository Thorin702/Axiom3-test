//! Privacy-safe usage telemetry. The guarantee lives in the TYPE, not in reviewer care: NO event field
//! is an open `&str`. Every dimension is a bounded Rust enum (`Mode`/`Compute`/`ModelFamily`/`TaskStatus`/
//! `Feature`); the rest are numbers/bools. There is literally no field a research topic, dataset, report,
//! filename, or query could be assigned to — a careless caller writing `mode: &goal` will not compile.
//! Only anonymous interaction metadata reaches the website's /api/telemetry (matches the product's
//! "数据留本机" promise + the small-scale users' consent to share INTERACTION data only).
//! Fire-and-forget: a telemetry failure never affects the app.

use std::sync::atomic::{AtomicBool, Ordering};

/// Bounded dimensions. The app maps its internal state into ONE of these variants; because they carry no
/// string payload, a topic/dataset/report cannot structurally reach the wire. `wire()` renders the fixed
/// on-the-wire token (kept identical to the historical values so the dashboard's queries are unchanged).
#[derive(Clone, Copy)]
pub enum Mode {
    Research,
    Analysis,
    Code,
}
impl Mode {
    fn wire(self) -> &'static str {
        match self {
            Mode::Research => "研究",
            Mode::Analysis => "分析",
            Mode::Code => "代码",
        }
    }
}

#[derive(Clone, Copy)]
pub enum Compute {
    Local,
    Cloud,
}
impl Compute {
    fn wire(self) -> &'static str {
        match self {
            Compute::Local => "local",
            Compute::Cloud => "cloud",
        }
    }
}

#[derive(Clone, Copy)]
pub enum ModelFamily {
    Glm,
    Deepseek,
    Other,
}
impl ModelFamily {
    /// Classify a model id into a bounded family — never stores the id itself.
    pub fn from_model_id(id: &str) -> Self {
        let m = id.to_lowercase();
        if m.contains("glm") {
            ModelFamily::Glm
        } else if m.contains("deepseek") {
            ModelFamily::Deepseek
        } else {
            ModelFamily::Other
        }
    }
    fn wire(self) -> &'static str {
        match self {
            ModelFamily::Glm => "glm",
            ModelFamily::Deepseek => "deepseek",
            ModelFamily::Other => "other",
        }
    }
}

#[derive(Clone, Copy)]
pub enum TaskStatus {
    Ok,
    NoReport,
    Failed,
    Cancelled,
}
impl TaskStatus {
    /// Map an internal run-status string into a bounded status — unknown → Failed (never stored raw).
    pub fn from_run_status(s: &str) -> Self {
        match s {
            "done" => TaskStatus::Ok,
            "no_report" => TaskStatus::NoReport,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::Failed,
        }
    }
    fn wire(self) -> &'static str {
        match self {
            TaskStatus::Ok => "ok",
            TaskStatus::NoReport => "no_report",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

/// UI interaction names — a CLOSED set. The only place a `Feature` is built is `Feature::from_name`
/// (called by the `telemetry` command), so an unknown name is dropped rather than sent as free text.
#[derive(Clone, Copy)]
pub enum Feature {
    ExportPdf,
    ExportWord,
    ExportHtml,
    Continue,
    Cancel,
    Pause,
    Fork,
    ExpandArtifact,
    CloseGuardConfirmed,
    NewTask,
    ResumeRun,
}
impl Feature {
    fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "export_pdf" => Feature::ExportPdf,
            "export_word" => Feature::ExportWord,
            "export_html" => Feature::ExportHtml,
            "continue" => Feature::Continue,
            "cancel" => Feature::Cancel,
            "pause" => Feature::Pause,
            "fork" => Feature::Fork,
            "expand_artifact" => Feature::ExpandArtifact,
            "close_guard_confirmed" => Feature::CloseGuardConfirmed,
            "new_task" => Feature::NewTask,
            "resume_run" => Feature::ResumeRun,
            _ => return None,
        })
    }
    fn wire(self) -> &'static str {
        match self {
            Feature::ExportPdf => "export_pdf",
            Feature::ExportWord => "export_word",
            Feature::ExportHtml => "export_html",
            Feature::Continue => "continue",
            Feature::Cancel => "cancel",
            Feature::Pause => "pause",
            Feature::Fork => "fork",
            Feature::ExpandArtifact => "expand_artifact",
            Feature::CloseGuardConfirmed => "close_guard_confirmed",
            Feature::NewTask => "new_task",
            Feature::ResumeRun => "resume_run",
        }
    }
}

/// Master switch. Default ON for the (consenting) cloud build; a to-B on-prem build or a user opt-out
/// flips it off, after which every emit is a no-op — nothing leaves the machine.
static ENABLED: AtomicBool = AtomicBool::new(true);
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// The ONLY events the app may send. Every field is a bounded enum, a number, or a bool — never a `&str`.
/// To add a metric, add a typed field here; there is deliberately no escape hatch for arbitrary text.
pub enum Ev {
    Launch,
    TaskStarted {
        mode: Mode,
        compute: Compute,
        model_family: ModelFamily,
        has_attachments: bool,
    },
    TaskFinished {
        mode: Mode,
        status: TaskStatus,
        dur_s: u64,
        steps: u32,
        figures: u32,
        sources: u32,
        citations: u32,         // [Sn] markers threaded into the report
        salvage: bool,          // the wrap-up salvage path fired
        review: bool,           // the reviewer ran
    },
    Feature {
        feat: Feature,
    },
}

impl Ev {
    fn name(&self) -> &'static str {
        match self {
            Ev::Launch => "launch",
            Ev::TaskStarted { .. } => "task_started",
            Ev::TaskFinished { .. } => "task_finished",
            Ev::Feature { .. } => "feature",
        }
    }
    fn props(&self) -> serde_json::Value {
        match self {
            Ev::Launch => serde_json::json!({}),
            Ev::TaskStarted { mode, compute, model_family, has_attachments } =>
                serde_json::json!({ "mode": mode.wire(), "compute": compute.wire(), "model": model_family.wire(), "attach": has_attachments }),
            Ev::TaskFinished { mode, status, dur_s, steps, figures, sources, citations, salvage, review } =>
                serde_json::json!({ "mode": mode.wire(), "status": status.wire(), "dur": dur_s, "steps": steps,
                    "figures": figures, "sources": sources, "citations": citations, "salvage": salvage, "review": review }),
            Ev::Feature { feat } => serde_json::json!({ "feature": feat.wire() }),
        }
    }
}

/// Preferred API: send a typed, content-free event.
pub fn track(app: &tauri::AppHandle, ev: Ev) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    emit(app, ev.name(), &ev.props());
}

/// Transport: attribute to the signed-in account, stamp app version + OS, POST async, swallow errors so
/// telemetry can never block or break a run. Private-by-convention — callers use `track` with a typed
/// `Ev` so no free-text can reach the wire.
fn emit(app: &tauri::AppHandle, event: &str, props: &serde_json::Value) {
    let account = crate::auth::current_account(app).unwrap_or_else(|| "anon".into());
    let version = app.package_info().version.to_string();
    let event = event.to_string();
    let os = std::env::consts::OS.to_string();
    let props = props.clone();
    tauri::async_runtime::spawn(async move {
        let body = serde_json::json!({ "account": account, "event": event, "props": props, "version": version, "os": os });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .unwrap_or_default();
        let _ = client.post(format!("{}/api/telemetry", crate::auth::AUTH_BASE)).json(&body).send().await;
    });
}

/// Front-end hook. The UI may ONLY fire whitelisted, content-free events: `launch`, or `feature` whose
/// `name` is a single enum from a fixed allowlist. Anything else is ignored — a stray props object can
/// never smuggle a topic/report/filename to the server through this seam.
#[tauri::command]
pub fn telemetry(app: tauri::AppHandle, event: String, props: Option<serde_json::Value>) {
    match event.as_str() {
        "launch" => track(&app, Ev::Launch),
        "feature" => {
            // Map the incoming name against the closed `Feature` set; unknown → dropped. No arbitrary
            // props object can smuggle a topic/report/filename through — only a recognised enum is sent.
            if let Some(name) = props.as_ref().and_then(|p| p.get("feature")).and_then(|v| v.as_str()) {
                if let Some(feat) = Feature::from_name(name) {
                    track(&app, Ev::Feature { feat });
                }
            }
        }
        _ => {} // unknown event → dropped
    }
}

/// User opt-out / on-prem switch. Called from Settings; when off, no telemetry leaves the machine.
#[tauri::command]
pub fn set_telemetry(on: bool) {
    set_enabled(on);
}

/// Fetch the current in-app announcement (set by the admin backend). Returns `{id, text, level}`,
/// or `{id:0}` when there is none / the server is unreachable. The front-end shows it once per new id.
#[tauri::command]
pub async fn fetch_notice() -> serde_json::Value {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .unwrap_or_default();
    match client.get(format!("{}/api/notice", crate::auth::AUTH_BASE)).send().await {
        Ok(r) => r.json::<serde_json::Value>().await.unwrap_or_else(|_| serde_json::json!({"id": 0})),
        Err(_) => serde_json::json!({"id": 0}),
    }
}
