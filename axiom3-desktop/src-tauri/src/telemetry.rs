//! Lightweight, privacy-safe usage telemetry. ONLY event names + counts + version/os are sent to the
//! website's /api/telemetry — NEVER the research topic, data, or deliverable content (matches the
//! product's "数据留本机" promise). Fire-and-forget: a telemetry failure never affects the app.

/// Send one event: attributes it to the signed-in account, stamps app version + OS, posts async,
/// and swallows any error so telemetry can never block or break a run.
pub fn emit(app: &tauri::AppHandle, event: &str, props: serde_json::Value) {
    let account = crate::auth::current_account(app).unwrap_or_else(|| "anon".into());
    let version = app.package_info().version.to_string();
    let event = event.to_string();
    let os = std::env::consts::OS.to_string();
    tauri::async_runtime::spawn(async move {
        let body = serde_json::json!({
            "account": account, "event": event, "props": props, "version": version, "os": os
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .unwrap_or_default();
        let _ = client
            .post(format!("{}/api/telemetry", crate::auth::AUTH_BASE))
            .json(&body)
            .send()
            .await;
    });
}

/// Front-end hook for lifecycle events the UI knows about (e.g. app launch).
#[tauri::command]
pub fn telemetry(app: tauri::AppHandle, event: String, props: Option<serde_json::Value>) {
    emit(&app, &event, props.unwrap_or(serde_json::Value::Null));
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
