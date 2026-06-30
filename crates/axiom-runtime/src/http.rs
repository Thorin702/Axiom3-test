//! Shared HTTP helpers for the network checkers (Crossref, OpenAlex …).
//!
//! The per-field output caps in each client (abstract length, result count) apply only
//! AFTER a response is deserialized — so a malicious or compromised upstream could make
//! us materialize an unbounded body into memory first. [`json_bounded`] reads the body
//! with a HARD size cap (a Content-Length pre-check plus streaming accumulation that
//! aborts the moment the cap is crossed), then parses. This bounds peak memory against
//! an upstream we don't control, which matters precisely because these clients are the
//! moat-aligned, must-be-robust tier.

/// Hard ceiling on a response body. A legitimate 25-result OpenAlex page or a single
/// Crossref work is far smaller; 16 MiB is generous headroom while closing the
/// unbounded-allocation door.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Read a response body with a hard size cap, then deserialize it as JSON. Errors
/// (fail-closed) if the declared or streamed size exceeds [`MAX_RESPONSE_BYTES`], or on
/// any transport/parse fault.
pub async fn json_bounded(resp: reqwest::Response) -> anyhow::Result<serde_json::Value> {
    json_bounded_to(resp, MAX_RESPONSE_BYTES).await
}

/// [`json_bounded`] with an explicit cap — the seam the tests drive with a tiny limit.
async fn json_bounded_to(resp: reqwest::Response, max: usize) -> anyhow::Result<serde_json::Value> {
    // Cheap pre-check: reject before reading a byte when the server declares an
    // over-size body. (Content-Length may be absent or lie — the stream cap below is
    // the real guard.)
    if let Some(len) = resp.content_length() {
        if len > max as u64 {
            anyhow::bail!("response too large: {len} bytes (cap {max})");
        }
    }
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| anyhow::anyhow!(e.to_string()))? {
        if buf.len() + chunk.len() > max {
            anyhow::bail!("response exceeded {max} bytes");
        }
        buf.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&buf).map_err(|e| anyhow::anyhow!(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_a_small_json_body() {
        let resp = reqwest::Response::from(http::Response::new(r#"{"ok":true}"#));
        let v = json_bounded_to(resp, MAX_RESPONSE_BYTES).await.unwrap();
        assert_eq!(v["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn rejects_a_body_over_the_cap() {
        // A 100-byte body with a 10-byte cap must fail closed (Content-Length pre-check).
        let resp = reqwest::Response::from(http::Response::new("x".repeat(100)));
        assert!(json_bounded_to(resp, 10).await.is_err());
    }
}
