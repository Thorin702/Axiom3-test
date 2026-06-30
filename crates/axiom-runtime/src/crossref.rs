//! Crossref-backed DOI verification (existence + retraction) — the first NETWORK
//! checker, the moat-aligned one.
//!
//! Queries the free Crossref REST API (no key) for a DOI and reports whether it
//! resolves and whether the cited work carries a Retraction Watch retraction notice.
//! Only the DOI (a public identifier) leaves the machine — never the draft text. The
//! retraction signal was verified empirically against the live API: a retracted work's
//! `message.updated-by` array contains an entry with `type == "retraction"`.
//!
//! HONESTY / FAIL-CLOSED: a transient/offline failure surfaces as an `Err` so the
//! caller records "couldn't verify — offline", never a silent pass. A 404 is a real
//! `NotFound` RESULT. Neither verdict is a refutation — Crossref isn't exhaustive (a
//! DOI may be registered with another agency), so the findings are Risk-level.

use serde_json::Value;

use crate::supervisor::{is_retryable_error, retry_async, RetryPolicy};

/// The outcome of verifying one DOI against Crossref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoiStatus {
    /// Resolves in Crossref and carries no retraction notice.
    Ok,
    /// Crossref returns 404 — not registered there (possibly fabricated, a typo, or a
    /// non-Crossref agency). A risk to verify, NOT proof of fabrication.
    NotFound,
    /// The cited work has a Retraction Watch retraction notice.
    Retracted,
}

/// Classify a Crossref `/works/{doi}` response. PURE — unit-tested without network.
pub fn classify_response(http_status: u16, body: &Value) -> DoiStatus {
    if http_status == 404 {
        return DoiStatus::NotFound;
    }
    let retracted = body["message"]["updated-by"]
        .as_array()
        .is_some_and(|ups| ups.iter().any(|u| u["type"] == "retraction"));
    if retracted {
        DoiStatus::Retracted
    } else {
        DoiStatus::Ok
    }
}

const CROSSREF_BASE: &str = "https://api.crossref.org/works/";
const USER_AGENT: &str = concat!("Axiom3/", env!("CARGO_PKG_VERSION"), " (research verification tool)");

/// A Crossref client (wraps a `reqwest::Client`; cheap to clone).
#[derive(Clone)]
pub struct Crossref {
    client: reqwest::Client,
}

impl Default for Crossref {
    fn default() -> Self {
        Self::new()
    }
}

impl Crossref {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Verify one DOI. A 404 is a `NotFound` result (not an error). A 5xx / network
    /// blip retries with backoff; on exhaustion the call ERRORS (fail-closed).
    pub async fn check(&self, doi: &str) -> anyhow::Result<DoiStatus> {
        let url = format!("{CROSSREF_BASE}{doi}");
        // The retry closure returns Ok(None) for a 404 (terminal, no retry), Ok(Some)
        // for a parsed 200 body, and Err for a transient fault to retry.
        let body: Option<Value> = retry_async(&RetryPolicy::default(), is_retryable_error, || {
            let client = self.client.clone();
            let url = url.clone();
            async move {
                let resp = client.get(&url).send().await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let status = resp.status();
                if status.as_u16() == 404 {
                    return Ok(None);
                }
                if status.is_server_error() {
                    // Mark retryable for is_retryable_error.
                    anyhow::bail!("crossref upstream error (HTTP {}); service unavailable", status.as_u16());
                }
                if !status.is_success() {
                    anyhow::bail!("crossref returned HTTP {}", status.as_u16());
                }
                let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Ok(Some(body))
            }
        })
        .await?;
        Ok(match body {
            None => DoiStatus::NotFound,
            Some(b) => classify_response(200, &b),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_detects_retraction() {
        // The exact shape verified against the live API (Retraction Watch source).
        let body = json!({
            "message": {
                "title": ["RETRACTED: …"],
                "updated-by": [
                    { "type": "correction", "source": "retraction-watch" },
                    { "type": "retraction", "source": "retraction-watch", "DOI": "10.1016/notice" }
                ]
            }
        });
        assert_eq!(classify_response(200, &body), DoiStatus::Retracted);
    }

    #[test]
    fn classify_passes_a_clean_doi() {
        let clean = json!({ "message": { "title": ["A fine paper"] } });
        assert_eq!(classify_response(200, &clean), DoiStatus::Ok);
        // updated-by present but only a correction → not retracted.
        let corrected = json!({ "message": { "updated-by": [ { "type": "correction" } ] } });
        assert_eq!(classify_response(200, &corrected), DoiStatus::Ok);
    }

    #[test]
    fn classify_maps_404_to_not_found() {
        assert_eq!(classify_response(404, &json!({})), DoiStatus::NotFound);
    }

    /// Live smoke test against the real Crossref API (network) — run explicitly:
    /// `cargo test -p axiom-runtime --features _ -- --ignored crossref_live` (or just
    /// `--ignored`). Verifies a retracted DOI, a clean DOI, and a fabricated one.
    #[tokio::test]
    #[ignore = "hits the live Crossref API"]
    async fn crossref_live_smoke() {
        let cr = Crossref::new();
        assert_eq!(
            cr.check("10.1016/S0140-6736(97)11096-0").await.unwrap(),
            DoiStatus::Retracted,
            "Wakefield 1998 is retracted"
        );
        assert_eq!(cr.check("10.1038/nature12373").await.unwrap(), DoiStatus::Ok);
        assert_eq!(cr.check("10.9999/this.doi.is.fake.999").await.unwrap(), DoiStatus::NotFound);
    }
}
