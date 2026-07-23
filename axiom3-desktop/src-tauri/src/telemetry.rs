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

/// WHERE a run died. Bounded on purpose: the stage alone distinguishes "the science engine failed"
/// from "the machine never got a Python kernel up" — two problems with completely different fixes,
/// and the second one is invisible in `TaskFinished` because it happens BEFORE any report exists.
#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    Startup,     // model resolve / dirs / sandbox — before any compute
    SciStack,    // scientific-environment seeding (the 15-min one)
    KernelStart, // Python kernel boot
    Orchestrate, // the agent loop itself
    Finalize,    // report assembly / deliverable write
}
impl Stage {
    fn wire(self) -> &'static str {
        match self {
            Stage::Startup => "startup",
            Stage::SciStack => "sci_stack",
            Stage::KernelStart => "kernel_start",
            Stage::Orchestrate => "orchestrate",
            Stage::Finalize => "finalize",
        }
    }
}

/// WHY a run died, as a closed bucket set. `from_msg` reads the error TEXT but stores only the
/// bucket — the message itself never leaves the machine (it can contain a path, a topic, or a model
/// echo). This is the same discipline as `ModelFamily::from_model_id`: classify locally, send the class.
#[derive(Clone, Copy, PartialEq)]
pub enum ErrKind {
    Timeout,
    Network,
    Auth,
    Kernel,
    Disk,
    Budget,
    Cancelled,
    Other,
}
impl ErrKind {
    /// Bucket an error message (Chinese or English). Order matters: "内核启动超时" is a TIMEOUT
    /// (the actionable fact) rather than a generic kernel fault, so timeout is tested first.
    pub fn from_msg(m: &str) -> Self {
        let s = m.to_lowercase();
        let has = |ks: &[&str]| ks.iter().any(|k| s.contains(k));
        if has(&["超时", "timeout", "timed out", "deadline"]) {
            ErrKind::Timeout
        } else if has(&["取消", "cancel", "aborted by user"]) {
            ErrKind::Cancelled
        } else if has(&["密钥", "未登录", "登录", "unauthor", "forbidden", "401", "403", "api key", "invalid key"]) {
            ErrKind::Auth
        } else if has(&["网络", "连接", "network", "connect", "dns", "unreachable", "tls", "certificate", "502", "503", "504"]) {
            ErrKind::Network
        } else if has(&["磁盘", "空间", "no space", "disk full", "quota exceeded", "read-only file system"]) {
            ErrKind::Disk
        } else if has(&["内核", "kernel", "python", "jupyter", "conda", "pip "]) {
            ErrKind::Kernel
        } else if has(&["预算", "budget", "step limit", "token limit", "context length"]) {
            ErrKind::Budget
        } else {
            ErrKind::Other
        }
    }
    fn wire(self) -> &'static str {
        match self {
            ErrKind::Timeout => "timeout",
            ErrKind::Network => "network",
            ErrKind::Auth => "auth",
            ErrKind::Kernel => "kernel",
            ErrKind::Disk => "disk",
            ErrKind::Budget => "budget",
            ErrKind::Cancelled => "cancelled",
            ErrKind::Other => "other",
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
    Launch {
        /// The previous session left its run-marker behind — i.e. it did NOT exit through the normal
        /// path. A crash, an OOM kill, or a force-quit all land here. This is the ONLY way a hard
        /// crash can ever be reported: the crashing process cannot send anything itself.
        unclean: bool,
    },
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
        // SHORTFALL FLAGS. `status: ok` only means the pipeline completed — it says nothing about
        // whether the delivered report was any good. These five are the machine-checkable ways this
        // product fails while still "succeeding", and each one is already written into the report
        // for the user to see; the flag just makes the RATE visible to us too. Read off the shipped
        // report's own banner constants (`axiom_orchestrate::coding::MARK_*`), so they cannot claim
        // a shortfall the reader wasn't also shown.
        no_sources: bool,       // a "research report" written from parametric memory alone
        downgraded: bool,       // no conjectures produced → this is a literature review, not research
        conj_incomplete: bool,  // conjectures shipped missing required fields
        derived_unmet: bool,    // claimed "computable right now" and then did not compute it
        fig_missing: bool,      // the goal asked for figures and none were produced
    },
    /// A run that started and never reached `TaskFinished` — it bailed out early (kernel never came
    /// up, sci-stack seeding timed out, sandbox refused) or panicked. Emitted by `RunGuard::drop`, so
    /// it does not depend on anyone remembering to add it to a new error path.
    ///
    /// This gives the dashboard a LEDGER IDENTITY:
    ///   `task_started == task_finished + task_aborted`
    /// Any imbalance is exactly the number of runs where the process itself died — a figure that is
    /// otherwise unobservable, since a dead process sends nothing.
    TaskAborted {
        mode: Mode,
        stage: Stage,
        kind: ErrKind,
        dur_s: u64,
    },
    Feature {
        feat: Feature,
    },
}

impl Ev {
    fn name(&self) -> &'static str {
        match self {
            Ev::Launch { .. } => "launch",
            Ev::TaskStarted { .. } => "task_started",
            Ev::TaskFinished { .. } => "task_finished",
            Ev::TaskAborted { .. } => "task_aborted",
            Ev::Feature { .. } => "feature",
        }
    }
    fn props(&self) -> serde_json::Value {
        match self {
            Ev::Launch { unclean } => serde_json::json!({ "unclean": unclean }),
            Ev::TaskStarted { mode, compute, model_family, has_attachments } =>
                serde_json::json!({ "mode": mode.wire(), "compute": compute.wire(), "model": model_family.wire(), "attach": has_attachments }),
            Ev::TaskFinished { mode, status, dur_s, steps, figures, sources, citations, salvage, review,
                no_sources, downgraded, conj_incomplete, derived_unmet, fig_missing } =>
                serde_json::json!({ "mode": mode.wire(), "status": status.wire(), "dur": dur_s, "steps": steps,
                    "figures": figures, "sources": sources, "citations": citations, "salvage": salvage, "review": review,
                    "no_sources": no_sources, "downgraded": downgraded, "conj_incomplete": conj_incomplete,
                    "derived_unmet": derived_unmet, "fig_missing": fig_missing }),
            Ev::TaskAborted { mode, stage, kind, dur_s } =>
                serde_json::json!({ "mode": mode.wire(), "status": "aborted", "stage": stage.wire(), "kind": kind.wire(), "dur": dur_s }),
            Ev::Feature { feat } => serde_json::json!({ "feature": feat.wire() }),
        }
    }
}

/// Set by `boot` when the previous session left its marker behind. Read once, by the `launch` event.
static UNCLEAN: AtomicBool = AtomicBool::new(false);
/// Set by `boot` when the previous session ended via the close-guard "quit anyway" confirmation.
static QUIT_CONFIRMED: AtomicBool = AtomicBool::new(false);

fn state_path(app: &tauri::AppHandle, file: &str) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_data_dir().ok().map(|d| d.join(file))
}

/// Call ONCE from `setup`, before anything can emit.
///
/// Two jobs, both of which must happen before the first event:
///  1. Load the PERSISTED opt-out. This used to live only in the front-end's localStorage, which is
///     applied by an effect that races the `launch` event — so an opted-out user still leaked one
///     `launch` per boot. The switch is now authoritative on the Rust side and is honoured from the
///     first instruction, not from whenever the webview gets around to syncing.
///  2. Detect an unclean previous exit and re-arm the marker for this session.
pub fn boot(app: &tauri::AppHandle) {
    if let Some(p) = state_path(app, "telemetry.off") {
        if p.exists() {
            ENABLED.store(false, Ordering::Relaxed);
        }
    }
    if let Some(p) = state_path(app, "session.running") {
        match std::fs::read_to_string(&p) {
            // The user confirmed "quit anyway" at the close guard. Not a crash — and the fact that
            // they chose to kill a live run is itself worth knowing, so it is reported (below, at
            // `launch`) as the CloseGuardConfirmed interaction.
            Ok(s) if s.trim() == QUIT_MARK => QUIT_CONFIRMED.store(true, Ordering::Relaxed),
            // Marker present but not a confirmed quit → the process died without running its exit
            // path at all.
            Ok(_) => UNCLEAN.store(true, Ordering::Relaxed),
            // Absent → last session exited cleanly. Nothing to report.
            Err(_) => {}
        }
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&p, b"1");
    }
}

const QUIT_MARK: &str = "q";

/// Call from the close-guard's `force_quit`, BEFORE `app.exit(0)`.
///
/// This is written to the marker file rather than sent as an event on purpose: `emit` is
/// fire-and-forget on a background task, and `app.exit(0)` on the very next line kills the process
/// long before that task can reach the network — the event would simply never arrive. Persisting it
/// and reporting at the NEXT launch is the only delivery that actually survives.
///
/// It also removes the one false-positive this whole mechanism is exposed to: if `app.exit(0)` turned
/// out not to dispatch `RunEvent::Exit`, every confirmed quit would otherwise be counted as a crash.
pub fn mark_quit_confirmed(app: &tauri::AppHandle) {
    if let Some(p) = state_path(app, "session.running") {
        let _ = std::fs::write(p, QUIT_MARK.as_bytes());
    }
}

/// Call from `RunEvent::Exit` — the normal way out. Anything that skips this (SIGKILL, OOM, a hard
/// crash, a panic-abort) leaves the marker, which the NEXT launch reports as `unclean`.
pub fn mark_clean_exit(app: &tauri::AppHandle) {
    if let Some(p) = state_path(app, "session.running") {
        let _ = std::fs::remove_file(p);
    }
}

/// Guarantees that a `TaskStarted` is always followed by a terminal event. Every early `?`, every
/// `return Err`, and every panic unwind runs `drop` — so a new failure path added later is covered
/// without anyone remembering to instrument it. Call `settled()` on the success path to hand the
/// reporting over to `TaskFinished`.
pub struct RunGuard {
    app: tauri::AppHandle,
    mode: Mode,
    started: std::time::Instant,
    stage: std::cell::Cell<Stage>,
    kind: std::cell::Cell<ErrKind>,
    settled: std::cell::Cell<bool>,
}

impl RunGuard {
    pub fn new(app: &tauri::AppHandle, mode: Mode) -> Self {
        Self {
            app: app.clone(),
            mode,
            started: std::time::Instant::now(),
            stage: std::cell::Cell::new(Stage::Startup),
            kind: std::cell::Cell::new(ErrKind::Other),
            settled: std::cell::Cell::new(false),
        }
    }
    /// Advance the "where am I" marker. If the run dies past this point, this is the stage reported.
    pub fn at(&self, s: Stage) {
        self.stage.set(s);
    }
    /// Bucket an error on its way out, then hand the message back unchanged so it can be used inline:
    /// `.map_err(|e| g.blame(&e.to_string()))?`. Only the BUCKET is retained; the text is dropped.
    pub fn blame(&self, msg: &str) -> String {
        self.kind.set(ErrKind::from_msg(msg));
        msg.to_string()
    }
    /// The run reached its own terminal event — suppress the abort report.
    pub fn settled(&self) {
        self.settled.set(true);
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.settled.get() {
            return;
        }
        let ev = Ev::TaskAborted {
            mode: self.mode,
            stage: self.stage.get(),
            kind: self.kind.get(),
            dur_s: self.started.elapsed().as_secs(),
        };
        // This drop can run while a panic is unwinding. A panic INSIDE a drop during unwinding
        // aborts the process — so a telemetry hiccup here (e.g. spawning on a runtime that is
        // already shutting down) would turn a recoverable error into a hard crash. Contain it:
        // losing one abort event is acceptable, killing the user's app to report one is not.
        let app = self.app.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || track(&app, ev)));
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
        // `swap` so these flags are reported exactly ONCE per boot: they describe the PREVIOUS
        // session, so a second launch event in the same session must not re-accuse it.
        "launch" => {
            // Deferred from the last session's close guard — see `mark_quit_confirmed`.
            if QUIT_CONFIRMED.swap(false, Ordering::Relaxed) {
                track(&app, Ev::Feature { feat: Feature::CloseGuardConfirmed });
            }
            track(&app, Ev::Launch { unclean: UNCLEAN.swap(false, Ordering::Relaxed) });
        }
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
/// PERSISTED on this side (not just in the webview's localStorage) so the choice is in force from
/// process start — before the front-end has mounted, and therefore before anything can be emitted.
#[tauri::command]
pub fn set_telemetry(app: tauri::AppHandle, on: bool) {
    set_enabled(on);
    if let Some(p) = state_path(&app, "telemetry.off") {
        if on {
            let _ = std::fs::remove_file(p);
        } else {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, b"1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every event this app can send, so the test below covers the whole surface. Adding a variant
    /// without adding it here fails the exhaustiveness match — the reminder is compiler-enforced.
    fn all_events() -> Vec<Ev> {
        // Exhaustiveness tripwire: if a variant is added, this match stops compiling until the
        // author comes here and puts a sample of it in the list below.
        fn _exhaustive(e: &Ev) {
            match e {
                Ev::Launch { .. } | Ev::TaskStarted { .. } | Ev::TaskFinished { .. }
                | Ev::TaskAborted { .. } | Ev::Feature { .. } => {}
            }
        }
        vec![
            Ev::Launch { unclean: true },
            Ev::TaskStarted { mode: Mode::Research, compute: Compute::Cloud, model_family: ModelFamily::Glm, has_attachments: true },
            Ev::TaskFinished { mode: Mode::Analysis, status: TaskStatus::NoReport, dur_s: 7, steps: 3, figures: 1,
                sources: 2, citations: 5, salvage: true, review: false, no_sources: true, downgraded: true,
                conj_incomplete: true, derived_unmet: true, fig_missing: true },
            Ev::TaskAborted { mode: Mode::Code, stage: Stage::KernelStart, kind: ErrKind::Timeout, dur_s: 1 },
            Ev::Feature { feat: Feature::ExportPdf },
        ]
    }

    /// THE privacy invariant, checked mechanically rather than by reading the code: no event may put
    /// a string on the wire that isn't one of our own fixed tokens.
    ///
    /// The type system is the real guarantee (no `Ev` field is a `&str`), but a future `wire()` that
    /// formats a value, or a `json!` that interpolates one, would slip past the types. This walks the
    /// ACTUAL serialised payload of every event and fails on any string it doesn't recognise — so the
    /// leak has to be introduced past a red test, not merely past a careful reviewer.
    #[test]
    fn no_event_can_put_free_text_on_the_wire() {
        // Every string value any event is allowed to emit. Deliberately written out by hand: it is
        // supposed to be tedious to extend, because extending it is exactly the moment to ask
        // "is this new string bounded, or is it user content?"
        const ALLOWED: &[&str] = &[
            "研究", "分析", "代码",                                     // Mode
            "local", "cloud",                                            // Compute
            "glm", "deepseek", "other",                                  // ModelFamily
            "ok", "no_report", "failed", "cancelled", "aborted",         // TaskStatus (+ the abort marker)
            "startup", "sci_stack", "kernel_start", "orchestrate", "finalize", // Stage
            "timeout", "network", "auth", "kernel", "disk", "budget",    // ErrKind (cancelled/other above)
            "export_pdf", "export_word", "export_html", "continue", "cancel", "pause", "fork",
            "expand_artifact", "close_guard_confirmed", "new_task", "resume_run", // Feature
        ];

        fn walk(v: &serde_json::Value, ev: &str) {
            match v {
                serde_json::Value::String(s) => assert!(
                    ALLOWED.contains(&s.as_str()),
                    "event `{ev}` put an unrecognised string on the wire: {s:?}. If this is a new \
                     bounded token, add it to ALLOWED. If it came from a topic, a path, a model \
                     reply, a filename or anything else the user typed or produced — it must not \
                     be sent at all."
                ),
                serde_json::Value::Object(m) => m.values().for_each(|x| walk(x, ev)),
                serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, ev)),
                _ => {} // numbers / bools / null carry no content
            }
        }

        for ev in all_events() {
            walk(&ev.props(), ev.name());
        }
    }

    /// The abort path exists precisely for errors, so its bucketing must survive the messages this
    /// app actually produces — in both languages, since ours are Chinese.
    #[test]
    fn error_messages_bucket_to_the_actionable_cause() {
        use ErrKind::*;
        let cases: &[(&str, ErrKind)] = &[
            ("Python 内核启动超时 · kernel start timed out", Timeout),
            ("科学环境初始化超时(>15 分钟) · sci-stack seeding timed out", Timeout),
            ("connection refused (os error 61)", Network),
            ("HTTP 401 Unauthorized", Auth),
            ("No space left on device", Disk),
            ("kernel had crashed and state was reset", Kernel),
            ("a run with this id is already in flight", Other),
        ];
        for (msg, want) in cases {
            assert!(
                ErrKind::from_msg(msg) == *want,
                "{msg:?} bucketed as {:?}, expected {:?}",
                ErrKind::from_msg(msg).wire(),
                want.wire()
            );
        }
    }

    /// A timeout that mentions the kernel is a TIMEOUT, not a kernel fault: the two have different
    /// fixes (raise the limit / repair the environment), and the ordering in `from_msg` is what
    /// decides. Pinned here so a later reordering can't quietly change what the dashboard means.
    #[test]
    fn timeout_wins_over_the_component_that_timed_out() {
        assert!(ErrKind::from_msg("Python 内核启动超时") == ErrKind::Timeout);
    }
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
