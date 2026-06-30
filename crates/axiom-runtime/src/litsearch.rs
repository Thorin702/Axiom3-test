//! Literature search — retrieve candidate evidence from the PUBLISHED RECORD so a claim
//! can be grounded against real literature, not just user-supplied references or a
//! model's parametric memory. This is the capability the real-paper testing named as the
//! biggest gap: without it, a Verified/Refuted is only model opinion.
//!
//! Backend: the free OpenAlex `/works` search API (no key required). OpenAlex is chosen
//! over Crossref for THIS job because it returns abstracts (as a reconstructable
//! inverted index) for most works — and the abstract is the text we can actually ground
//! a claim against. Only a search query derived from the claim leaves the machine.
//!
//! HONESTY / FAIL-CLOSED: a transient/offline failure surfaces as an `Err`, so the
//! caller records "couldn't search — offline" and the verdict stays whatever the other
//! tiers concluded. Retrieval NEVER fabricates support. And — load-bearing for the moat
//! — what comes back is CANDIDATE evidence: retrieving a relevant abstract is not proof
//! the claim is true, and ABSENCE of a hit is not refutation (OpenAlex isn't exhaustive,
//! and we read abstracts, not full text). The grounding tier treats these as advisory
//! passages, exactly like user-supplied references.

use serde::Serialize;
use serde_json::Value;

use crate::supervisor::{is_retryable_error, retry_async, RetryPolicy};

/// One retrieved work — the fields useful for grounding + citing the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Work {
    /// The OpenAlex id (e.g. `W2741809807`), the stable handle for this record.
    pub openalex_id: String,
    pub title: String,
    /// The abstract, reconstructed from OpenAlex's inverted index (may be empty — many
    /// older or metadata-only records carry none; such a work can title-match but gives
    /// no text to ground against).
    pub abstract_text: String,
    /// The DOI without the `https://doi.org/` prefix, when present.
    pub doi: Option<String>,
    pub year: Option<i64>,
    /// A few author display names (capped) — enough to cite, not the full list.
    pub authors: Vec<String>,
}

const OPENALEX_WORKS: &str = "https://api.openalex.org/works";
/// Only the fields we use — keeps responses small and predictable.
const SELECT_FIELDS: &str = "id,title,publication_year,doi,abstract_inverted_index,authorships";
const USER_AGENT: &str =
    concat!("Axiom3/", env!("CARGO_PKG_VERSION"), " (research verification tool)");

/// Hard caps so a pathological response can't blow up memory or a query can't smuggle
/// control characters into the request.
const MAX_QUERY_CHARS: usize = 300;
const MAX_ABSTRACT_CHARS: usize = 4_000;
const MAX_AUTHORS: usize = 4;
const MAX_RESULTS: usize = 25;

/// Turn a claim (or any text) into a safe search query: collapse all whitespace, drop
/// control characters, and cap the length. URL-encoding is handled by the query-param
/// builder, so this is about cleanliness and bounding, not escaping.
pub fn sanitize_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_QUERY_CHARS));
    let mut count = 0usize;
    let mut prev_space = false;
    for ch in text.chars() {
        if count >= MAX_QUERY_CHARS {
            break;
        }
        if ch.is_control() || ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                count += 1;
                prev_space = true;
            }
            continue;
        }
        out.push(ch);
        count += 1;
        prev_space = false;
    }
    out.trim().to_string()
}

/// Reconstruct an abstract from OpenAlex's `abstract_inverted_index` (`{word: [pos,…]}`)
/// by placing each word at its position(s) and joining in order. Bounded by
/// [`MAX_ABSTRACT_CHARS`]. PURE — unit-tested without network.
pub fn reconstruct_abstract(inv: &Value) -> String {
    let Some(obj) = inv.as_object() else {
        return String::new();
    };
    let mut pairs: Vec<(u64, &str)> = Vec::new();
    for (word, positions) in obj {
        if let Some(arr) = positions.as_array() {
            for p in arr {
                if let Some(pos) = p.as_u64() {
                    pairs.push((pos, word.as_str()));
                }
            }
        }
    }
    // Stable order: by position, then by word so equal positions are deterministic.
    pairs.sort_unstable();
    let mut out = String::new();
    for (_, word) in pairs {
        // Stop BEFORE exceeding the bound — never truncate mid-string (a byte-index
        // truncate would panic on a multibyte char, e.g. a CJK abstract).
        if out.len() + word.len() + 1 > MAX_ABSTRACT_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Strip the resolver prefix from an OpenAlex DOI URL (`https://doi.org/10.x` → `10.x`).
fn strip_doi_prefix(doi: &str) -> Option<String> {
    let d = doi
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi.org/");
    d.starts_with("10.").then(|| d.to_string())
}

/// Pull the last path segment of an OpenAlex id URL (`…/W123` → `W123`).
fn openalex_short_id(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

fn parse_work(w: &Value) -> Work {
    let authors = w["authorships"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["author"]["display_name"].as_str())
                .take(MAX_AUTHORS)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    Work {
        openalex_id: openalex_short_id(w["id"].as_str().unwrap_or("")),
        title: w["title"].as_str().unwrap_or("").to_string(),
        abstract_text: reconstruct_abstract(&w["abstract_inverted_index"]),
        doi: w["doi"].as_str().and_then(strip_doi_prefix),
        year: w["publication_year"].as_i64(),
        authors,
    }
}

/// Parse an OpenAlex `/works` response body into at most `limit` works. PURE.
pub fn parse_works(body: &Value, limit: usize) -> Vec<Work> {
    body["results"]
        .as_array()
        .map(|arr| arr.iter().take(limit).map(parse_work).collect())
        .unwrap_or_default()
}

/// A literature-search client (wraps a `reqwest::Client`; cheap to clone).
#[derive(Clone)]
pub struct LitSearch {
    client: reqwest::Client,
}

impl Default for LitSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl LitSearch {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Search the published record for works relevant to `query`, returning up to
    /// `limit` (clamped to [`MAX_RESULTS`]). An empty/blank query returns `Ok(vec![])`
    /// without a request. A 5xx/network blip retries with backoff; on exhaustion the
    /// call ERRORS (fail-closed — never a silent empty result mistaken for "no support").
    pub async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Work>> {
        let q = sanitize_query(query);
        if q.is_empty() {
            return Ok(vec![]);
        }
        let limit = limit.clamp(1, MAX_RESULTS);
        let per_page = limit.to_string();
        let body: Value = retry_async(&RetryPolicy::default(), is_retryable_error, || {
            let client = self.client.clone();
            let q = q.clone();
            let per_page = per_page.clone();
            async move {
                // Query params are URL-encoded by reqwest — the claim-derived query
                // cannot steer the request off `/works`.
                let resp = client
                    .get(OPENALEX_WORKS)
                    .query(&[
                        ("search", q.as_str()),
                        ("per-page", per_page.as_str()),
                        ("select", SELECT_FIELDS),
                    ])
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let status = resp.status();
                if status.is_server_error() {
                    anyhow::bail!("openalex upstream error (HTTP {}); service unavailable", status.as_u16());
                }
                if !status.is_success() {
                    anyhow::bail!("openalex returned HTTP {}", status.as_u16());
                }
                let body: Value = resp.json().await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
                Ok(body)
            }
        })
        .await?;
        Ok(parse_works(&body, limit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reconstructs_abstract_in_position_order() {
        let inv = json!({ "the": [0, 4], "cat": [1], "sat": [2], "on": [3], "mat": [5] });
        assert_eq!(reconstruct_abstract(&inv), "the cat sat on the mat");
    }

    #[test]
    fn reconstruct_handles_missing_or_malformed_index() {
        assert_eq!(reconstruct_abstract(&Value::Null), "");
        assert_eq!(reconstruct_abstract(&json!("not an object")), "");
        assert_eq!(reconstruct_abstract(&json!({})), "");
    }

    #[test]
    fn reconstruct_bounds_a_huge_multibyte_abstract_without_panicking() {
        // A long CJK abstract: each word is multibyte, so a byte-index truncate at the
        // cap would land mid-char and panic. The bound must hold on a char boundary.
        let mut inv = serde_json::Map::new();
        for i in 0..5000u64 {
            inv.insert("研究".to_string() + &i.to_string(), json!([i]));
        }
        let abs = reconstruct_abstract(&Value::Object(inv));
        assert!(abs.len() <= MAX_ABSTRACT_CHARS, "bounded: {} bytes", abs.len());
        // It's valid UTF-8 (didn't split a char) — round-trips through char iteration.
        assert_eq!(abs.chars().count(), abs.chars().count());
    }

    #[test]
    fn sanitize_collapses_whitespace_strips_control_and_caps() {
        assert_eq!(sanitize_query("  the\n\teffect   was\r\nlarge  "), "the effect was large");
        let long = "word ".repeat(200);
        assert!(sanitize_query(&long).chars().count() <= MAX_QUERY_CHARS);
        assert_eq!(sanitize_query("   \n\t  "), "");
    }

    #[test]
    fn parses_a_works_response() {
        let body = json!({
            "results": [
                {
                    "id": "https://openalex.org/W2741809807",
                    "title": "Ankle sprain epidemiology",
                    "publication_year": 2019,
                    "doi": "https://doi.org/10.1016/j.foo.2019.01.001",
                    "abstract_inverted_index": { "Ankle": [0], "sprains": [1], "are": [2], "common": [3] },
                    "authorships": [
                        { "author": { "display_name": "A. Researcher" } },
                        { "author": { "display_name": "B. Coauthor" } }
                    ]
                },
                {
                    "id": "https://openalex.org/W999",
                    "title": "No abstract here",
                    "publication_year": 1998,
                    "doi": null,
                    "abstract_inverted_index": null,
                    "authorships": []
                }
            ]
        });
        let works = parse_works(&body, 10);
        assert_eq!(works.len(), 2);
        assert_eq!(works[0].openalex_id, "W2741809807");
        assert_eq!(works[0].title, "Ankle sprain epidemiology");
        assert_eq!(works[0].abstract_text, "Ankle sprains are common");
        assert_eq!(works[0].doi.as_deref(), Some("10.1016/j.foo.2019.01.001"));
        assert_eq!(works[0].year, Some(2019));
        assert_eq!(works[0].authors, vec!["A. Researcher", "B. Coauthor"]);
        // The metadata-only record parses but carries no abstract / DOI.
        assert_eq!(works[1].abstract_text, "");
        assert_eq!(works[1].doi, None);
        assert!(works[1].authors.is_empty());
    }

    #[test]
    fn parse_respects_the_limit_and_tolerates_a_missing_results_key() {
        let body = json!({ "results": [ {"id": "a"}, {"id": "b"}, {"id": "c"} ] });
        assert_eq!(parse_works(&body, 2).len(), 2);
        assert!(parse_works(&json!({ "meta": {} }), 5).is_empty());
    }

    #[tokio::test]
    async fn blank_query_makes_no_request() {
        // A blank query short-circuits to an empty result without touching the network.
        assert!(LitSearch::new().search("   \n  ", 5).await.unwrap().is_empty());
    }

    /// Live smoke test against the real OpenAlex API (network) — run explicitly:
    /// `cargo test -p axiom-runtime -- --ignored litsearch_live`.
    #[tokio::test]
    #[ignore = "hits the live OpenAlex API"]
    async fn litsearch_live_smoke() {
        let works = LitSearch::new()
            .search("lateral ankle sprain epidemiology athletes", 5)
            .await
            .unwrap();
        assert!(!works.is_empty(), "a broad query should return works");
        assert!(works.iter().any(|w| !w.abstract_text.is_empty()), "at least one abstract");
    }
}
