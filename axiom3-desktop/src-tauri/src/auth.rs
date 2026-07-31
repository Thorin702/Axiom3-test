//! Account login + license-expiry gate. The desktop app authenticates against the website's
//! account system (`/api/login`, `/api/validate`, `/api/logout`) so only an APPROVED, IN-DATE
//! account can use it. All auth HTTP goes through here (Rust), never the webview — so the strict
//! CSP / CORS don't apply and the password never lives in the front-end.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;

/// The website / account server. HTTPS on the filed (ICP-备案) domain — passwords and licence
/// tokens now cross the wire encrypted. The certificate is Let's Encrypt, auto-renewed by certbot,
/// so nothing here needs touching when it rolls over.
pub const AUTH_BASE: &str = "https://axiom3.cn";
const HTTP_TIMEOUT_S: u64 = 8;

/// Ed25519 public key matching `license_key.pem` on the account server. PUBLIC by design: it can only
/// CHECK a licence, never produce one. The private half never leaves the server, so no amount of
/// reverse-engineering this binary yields the ability to mint an expiry.
const LICENSE_PUBKEY: [u8; 32] = [
    0xbf, 0x8f, 0x75, 0x0c, 0x54, 0x80, 0xd3, 0x2f, 0xbb, 0x1f, 0x98, 0x7e, 0x38, 0x40, 0x40, 0x29,
    0x27, 0x18, 0xc5, 0x85, 0xd4, 0xf1, 0x5c, 0xb9, 0xe0, 0x13, 0x37, 0x37, 0xec, 0x94, 0x37, 0xfe,
];

/// How long a signed licence stays usable without the server confirming it again. The app cannot do
/// research offline anyway (every model call is a network call), so this only ever bounds a genuine
/// outage — while making sure someone who logs in once and then stays offline still has to come back
/// and be re-checked. That check-in is the moment a disable / expiry change actually reaches them.
const OFFLINE_GRACE_DAYS: i64 = 7;

#[derive(Serialize, Deserialize, Clone, Default)]
struct Session {
    account: String,
    token: String,
    /// Kept only so an OLD session file still parses. Never trusted: it is plain text on the user's
    /// own disk. Everything that matters reads `license` instead.
    expiry: Option<String>,
    /// `base64url(payload).base64url(signature)` — the server's signed statement of this account's
    /// expiry. Absent for sessions written before this existed; those simply have to go online once.
    license: Option<String>,
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Check a licence and return the expiry it certifies. `None` means "do not trust anything here" —
/// a bad signature, someone else's account, or a licence too old to still speak for the server.
///
/// This is the whole point of the mechanism: the expiry the app enforces comes from a value the user
/// cannot edit. Before this, `expiry` was a plain field in `auth.json`; setting it to 2099 and
/// staying offline was an unlimited licence.
fn verify_license(license: &str, account: &str) -> Option<String> {
    verify_license_at(license, account, &LICENSE_PUBKEY, chrono::Local::now().date_naive())
}

/// The checkable core. Public key and "today" are parameters so the tests can sign with a key of
/// their own and pin the date — a fixture signed by the real server key would drift out of its grace
/// window as the calendar moved and the test would start failing for no reason.
fn verify_license_at(
    license: &str,
    account: &str,
    pubkey: &[u8],
    now: chrono::NaiveDate,
) -> Option<String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let (p, s) = license.split_once('.')?;
    let payload = b64.decode(p).ok()?;
    let sig = b64.decode(s).ok()?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pubkey)
        .verify(&payload, &sig)
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    // A validly-signed licence for a DIFFERENT account is still not this user's licence.
    if v.get("account").and_then(|x| x.as_str()) != Some(account) {
        return None;
    }
    let iat = v.get("iat").and_then(|x| x.as_str())?;
    let issued = chrono::NaiveDate::parse_from_str(iat, "%Y-%m-%d").ok()?;
    // Also reject a licence dated in the future — that means the clock was wound back, which is the
    // other half of the same trick (roll the clock back to sit inside an expired window forever).
    if issued > now || (now - issued).num_days() > OFFLINE_GRACE_DAYS {
        return None;
    }
    let expiry = v.get("expiry").and_then(|x| x.as_str()).unwrap_or("");
    // An empty expiry is a real, deliberate value: an account with no end date.
    Some(expiry.to_string())
}

/// The expiry this session can prove, or `None` if it can prove nothing.
fn proven_expiry(s: &Session) -> Option<String> {
    let lic = s.license.as_deref()?;
    let e = verify_license(lic, &s.account)?;
    if e.is_empty() {
        return Some(String::new()); // signed "no end date"
    }
    if e < today() {
        return None; // signed, but the term is over
    }
    Some(e)
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
/// The signed-in account name, if any — used to attribute telemetry events.
pub fn current_account(app: &tauri::AppHandle) -> Option<String> {
    load_session(app).map(|s| s.account)
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
                    let license = v.get("license").and_then(|s| s.as_str()).map(str::to_string);
                    save_session(&app, &Session { account: acc.clone(), token, expiry: expiry.clone(), license });
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
        // Report the expiry the LICENCE proves, not the one sitting in the file. This runs before the
        // server round-trip, so an unverifiable value here would be shown to the user as fact.
        Some(s) => {
            let expiry = proven_expiry(&s);
            AuthResult { ok: true, account: Some(s.account), expiry, ..Default::default() }
        }
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
                    // Persist the licence the server just re-issued: its `iat` is what buys the next
                    // offline grace window, so a session that keeps checking in never runs it down.
                    let mut s2 = s.clone();
                    if let Some(lic) = v.get("license").and_then(|x| x.as_str()) {
                        s2.license = Some(lic.to_string());
                    }
                    if let Some(e) = v.get("expiry").and_then(|x| x.as_str()) {
                        s2.expiry = Some(e.to_string());
                    }
                    save_session(&app, &s2);
                    let expiry = proven_expiry(&s2).or(s2.expiry.clone());
                    AuthResult { ok: true, account: Some(s.account), expiry, ..Default::default() }
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
            Err(_) => offline_verdict(&s),
        },
        Err(_) => offline_verdict(&s),
    }
}

/// The server could not be reached. Previously this returned `ok: true` unconditionally and let the
/// front-end fall back to the expiry stored on disk — which is exactly the gap that made "edit
/// auth.json, then unplug the network" an unlimited licence.
///
/// Now the session has to PROVE its term with the server's signature. Note what this does not do: it
/// does not punish a user for a flaky connection, because a valid licence keeps working for the whole
/// grace window. It only refuses a claim nobody signed.
fn offline_verdict(s: &Session) -> AuthResult {
    match proven_expiry(s) {
        Some(expiry) => AuthResult {
            ok: true,
            account: Some(s.account.clone()),
            expiry: if expiry.is_empty() { None } else { Some(expiry) },
            offline: true,
            ..Default::default()
        },
        None => AuthResult {
            ok: false,
            reason: Some("offline_unverified".into()),
            error: Some("离线状态下无法验证授权(或授权已到期),请连网后重新登录".into()),
            offline: true,
            ..Default::default()
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ring::signature::KeyPair;

    const B64: base64::engine::general_purpose::GeneralPurpose =
        base64::engine::general_purpose::URL_SAFE_NO_PAD;

    /// A throwaway signing key standing in for the server's. Tests must be able to MINT licences to
    /// prove the checks that matter; they cannot use the real key, and must not need to.
    fn keypair() -> ring::signature::Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    /// Mirrors the server's `issue_license`.
    fn sign(kp: &ring::signature::Ed25519KeyPair, account: &str, expiry: &str, iat: &str) -> String {
        let payload = format!(
            r#"{{"v":1,"account":"{account}","expiry":"{expiry}","iat":"{iat}"}}"#
        );
        let sig = kp.sign(payload.as_bytes());
        format!("{}.{}", B64.encode(payload.as_bytes()), B64.encode(sig.as_ref()))
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_properly_signed_licence_yields_its_expiry() {
        let kp = keypair();
        let lic = sign(&kp, "alice", "2026-12-31", "2026-07-21");
        assert_eq!(
            verify_license_at(&lic, "alice", kp.public_key().as_ref(), day("2026-07-22")),
            Some("2026-12-31".to_string())
        );
    }

    /// THE defect this whole mechanism exists to close: the expiry used to be a plain field the user
    /// could edit. Editing it must now invalidate the signature rather than extend the licence.
    #[test]
    fn an_edited_expiry_is_refused() {
        let kp = keypair();
        let lic = sign(&kp, "alice", "2026-08-16", "2026-07-21");
        let (p, s) = lic.split_once('.').unwrap();
        let payload = String::from_utf8(B64.decode(p).unwrap()).unwrap();
        let forged = payload.replace("2026-08-16", "2099-01-01");
        let tampered = format!("{}.{}", B64.encode(forged.as_bytes()), s);
        assert_eq!(
            verify_license_at(&tampered, "alice", kp.public_key().as_ref(), day("2026-07-22")),
            None,
            "改掉期限后必须验签失败"
        );
    }

    /// A licence minted by anything other than the server is not a licence. Without this the whole
    /// design collapses into the symmetric scheme it was chosen over.
    #[test]
    fn a_licence_signed_by_a_different_key_is_refused() {
        let real = keypair();
        let attacker = keypair();
        let lic = sign(&attacker, "alice", "2099-01-01", "2026-07-21");
        assert_eq!(
            verify_license_at(&lic, "alice", real.public_key().as_ref(), day("2026-07-22")),
            None
        );
    }

    /// Copying a colleague's licence file must not work, however validly it is signed.
    #[test]
    fn another_accounts_licence_is_refused() {
        let kp = keypair();
        let lic = sign(&kp, "bob", "2099-01-01", "2026-07-21");
        assert_eq!(verify_license_at(&lic, "alice", kp.public_key().as_ref(), day("2026-07-22")), None);
    }

    /// The grace window is what forces a re-check: log in once and stay offline forever, and the
    /// licence eventually stops speaking for the server.
    #[test]
    fn a_licence_past_its_grace_window_stops_being_accepted() {
        let kp = keypair();
        let lic = sign(&kp, "alice", "2099-01-01", "2026-07-01");
        let pk = kp.public_key();
        // Inside the window: fine.
        assert!(verify_license_at(&lic, "alice", pk.as_ref(), day("2026-07-08")).is_some());
        // One day past it: refused, even though the expiry itself is decades away.
        assert_eq!(verify_license_at(&lic, "alice", pk.as_ref(), day("2026-07-09")), None);
    }

    /// Winding the clock back is the other half of the same trick — it would otherwise park the
    /// machine inside a grace window indefinitely.
    #[test]
    fn a_licence_dated_in_the_future_is_refused() {
        let kp = keypair();
        let lic = sign(&kp, "alice", "2099-01-01", "2026-07-21");
        assert_eq!(verify_license_at(&lic, "alice", kp.public_key().as_ref(), day("2026-07-20")), None);
    }

    #[test]
    fn malformed_input_is_refused_rather_than_panicking() {
        let kp = keypair();
        let pk = kp.public_key();
        for bad in ["", ".", "not-base64.at-all", "aaaa", "eyJhIjoxfQ.zzzz"] {
            assert_eq!(verify_license_at(bad, "alice", pk.as_ref(), day("2026-07-22")), None, "{bad:?}");
        }
    }

    /// An old session file (written before licences existed) has nothing to prove with, so it must
    /// not be trusted offline — it has to go online once. It must NOT fall back to `expiry`.
    #[test]
    fn a_session_without_a_licence_can_prove_nothing() {
        let s = Session {
            account: "alice".into(),
            token: "t".into(),
            expiry: Some("2099-01-01".into()), // the field a user could edit
            license: None,
        };
        assert_eq!(proven_expiry(&s), None);
    }
}

#[cfg(test)]
mod tls_probe {
    /// 真实网络探针:确认 APP 的 TLS 栈(rustls + 其内置根证书)能验证账号服务器的证书。
    ///
    /// 存在的理由:证书链的根是 Let's Encrypt 2025 年的新根(ISRG Root YR)。浏览器和 curl 用的是
    /// 系统根证书库,而 reqwest 在此处配了 rustls,带的是自己编译进去的一套根 —— 两者可能不一致。
    /// 若这套根里没有该证书,所有登录/校验会在真机上直接失败,而本地 curl 一切正常,极易误判。
    /// 默认 #[ignore],只在需要确认线上证书时手动跑。
    #[tokio::test]
    #[ignore]
    async fn app_tls_stack_accepts_the_account_server_certificate() {
        let c = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("client");
        let r = c
            .get(format!("{}/api/notice", super::AUTH_BASE))
            .send()
            .await
            .expect("TLS 握手或连接失败 —— APP 将无法登录");
        assert!(r.status().is_success(), "HTTP {}", r.status());
        eprintln!("  ✓ {} 证书通过 rustls 验证,HTTP {}", super::AUTH_BASE, r.status());
    }
}
