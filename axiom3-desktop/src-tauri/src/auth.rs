//! Account login + license-expiry gate. The desktop app authenticates against the website's
//! account system (`/api/login`, `/api/validate`, `/api/logout`) so only an APPROVED, IN-DATE
//! account can use it. All auth HTTP goes through here (Rust), never the webview — so the strict
//! CSP / CORS don't apply and the password never lives in the front-end.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

/// The website / account server. IP for now; switch to `https://<域名>` after ICP 备案 + 证书.
const AUTH_BASE: &str = "http://39.96.27.187";
const HTTP_TIMEOUT_S: u64 = 8;

#[derive(Serialize, Deserialize, Clone, Default)]
struct Session {
    account: String,
    token: String,
    expiry: Option<String>,
}

/// What every auth command returns to the front-end.
#[derive(Serialize, Default)]
pub struct AuthResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expiry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// server reason on a definitive rejection: expired / inactive / kicked / no_account / no_session
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// true when the server was unreachable — the front-end then falls back to the local expiry date
    offline: bool,
}

fn auth_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("auth.json"))
}
fn load_session(app: &tauri::AppHandle) -> Option<Session> {
    let p = auth_path(app)?;
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}
fn save_session(app: &tauri::AppHandle, s: &Session) {
    if let (Some(p), Ok(js)) = (auth_path(app), serde_json::to_string(s)) {
        let _ = std::fs::write(p, js);
    }
}
fn clear_session(app: &tauri::AppHandle) {
    if let Some(p) = auth_path(app) {
        let _ = std::fs::remove_file(p);
    }
}
/// Best-effort human-readable device name (for the single-device-login "kicked" message).
fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "桌面端".into())
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_S))
        .build()
        .unwrap_or_default()
}

/// Log in with a website account. On success the session (token + expiry) is stored locally.
#[tauri::command]
pub async fn account_login(app: tauri::AppHandle, account: String, password: String) -> AuthResult {
    let device = format!("{} · {}", device_name(), std::env::consts::OS);
    let body = serde_json::json!({ "account": account, "password": password, "device": device });
    match client().post(format!("{AUTH_BASE}/api/login")).json(&body).send().await {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => {
                if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                    let acc = v.get("account").and_then(|s| s.as_str()).unwrap_or(&account).to_string();
                    let token = v.get("token").and_then(|s| s.as_str()).unwrap_or("").to_string();
                    let expiry = v.get("expiry").and_then(|s| s.as_str()).map(str::to_string);
                    save_session(&app, &Session { account: acc.clone(), token, expiry: expiry.clone() });
                    AuthResult { ok: true, account: Some(acc), expiry, ..Default::default() }
                } else {
                    AuthResult { ok: false, error: v.get("error").and_then(|s| s.as_str()).map(str::to_string), ..Default::default() }
                }
            }
            Err(_) => AuthResult { ok: false, error: Some("授权服务器返回异常".into()), ..Default::default() },
        },
        Err(_) => AuthResult { ok: false, error: Some("无法连接授权服务器,请检查网络后重试".into()), offline: true, ..Default::default() },
    }
}

/// Fast, LOCAL read of the stored session — used for the very first render before the server
/// round-trip. Does NOT prove validity; always followed by `account_validate`.
#[tauri::command]
pub fn account_session(app: tauri::AppHandle) -> AuthResult {
    match load_session(&app) {
        Some(s) => AuthResult { ok: true, account: Some(s.account), expiry: s.expiry, ..Default::default() },
        None => AuthResult { ok: false, reason: Some("no_session".into()), ..Default::default() },
    }
}

/// Re-check with the server: called on launch, periodically, and before each run. A definitive
/// rejection (expired / inactive / kicked) drops the local session so the app returns to login;
/// a network error returns `offline:true` and keeps the session (the front-end then trusts the
/// local expiry date, so a server hiccup never locks out an in-date user).
#[tauri::command]
pub async fn account_validate(app: tauri::AppHandle) -> AuthResult {
    let s = match load_session(&app) {
        Some(s) => s,
        None => return AuthResult { ok: false, reason: Some("no_session".into()), ..Default::default() },
    };
    let body = serde_json::json!({ "account": s.account, "token": s.token });
    match client().post(format!("{AUTH_BASE}/api/validate")).json(&body).send().await {
        Ok(r) => match r.json::<serde_json::Value>().await {
            Ok(v) => {
                if v.get("ok").and_then(|b| b.as_bool()).unwrap_or(false) {
                    AuthResult { ok: true, account: Some(s.account), expiry: s.expiry, ..Default::default() }
                } else {
                    clear_session(&app);
                    AuthResult {
                        ok: false,
                        reason: v.get("reason").and_then(|x| x.as_str()).map(str::to_string),
                        error: v.get("error").and_then(|x| x.as_str()).map(str::to_string),
                        ..Default::default()
                    }
                }
            }
            Err(_) => AuthResult { ok: true, account: Some(s.account), expiry: s.expiry, offline: true, ..Default::default() },
        },
        Err(_) => AuthResult { ok: true, account: Some(s.account), expiry: s.expiry, offline: true, ..Default::default() },
    }
}

/// Explicit logout: tells the server (so the single-device slot frees) and clears the local session.
#[tauri::command]
pub async fn account_logout(app: tauri::AppHandle) {
    if let Some(s) = load_session(&app) {
        let body = serde_json::json!({ "account": s.account, "token": s.token });
        let _ = client().post(format!("{AUTH_BASE}/api/logout")).json(&body).send().await;
    }
    clear_session(&app);
}
