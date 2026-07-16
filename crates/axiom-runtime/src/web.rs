//! General web tools for the research agent: `web_search` (find sources across the whole web) and
//! `web_fetch` (read a specific URL as text). Keyless — search is DuckDuckGo's HTML endpoint. Both
//! are BOUNDED (peak memory + output length) and SSRF-GUARDED: the URL/query is model-chosen and the
//! goal is untrusted, so a fetch to localhost / cloud-metadata / private ranges is refused.
use crate::http::bytes_bounded;
use crate::supervisor::{is_retryable_error, retry_async, RetryPolicy};

const UA: &str = "Mozilla/5.0 (compatible; Axiom3Research/1.0; +https://axiom3.local)";
const MAX_FETCH_BYTES: usize = 3 * 1024 * 1024; // page download cap
const MAX_TEXT_CHARS: usize = 30_000; // extracted-text cap handed to the model

#[derive(Debug, Clone)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone)]
pub struct WebClient {
    client: reqwest::Client,
}

impl Default for WebClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WebClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(std::time::Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none()) // we follow redirects manually + guard each hop
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// Fetch a URL and return its readable text (HTML stripped, bounded). SSRF-guarded.
    pub async fn fetch(&self, url: &str) -> anyhow::Result<String> {
        let url = url.trim().to_string();
        guard_public_url(&url).await?;
        let body = retry_async(&RetryPolicy::default(), is_retryable_error, || {
            let client = self.client.clone();
            let start = url.clone();
            async move {
                let mut current = reqwest::Url::parse(&start).map_err(|_| anyhow::anyhow!("invalid URL"))?;
                for _hop in 0..6 {
                    // guard BEFORE connecting to each hop so an internal host is never even contacted
                    guard_public_url(current.as_str()).await?;
                    let resp = client.get(current.clone()).send().await.map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    let status = resp.status();
                    if status.is_redirection() {
                        let loc = resp
                            .headers()
                            .get(reqwest::header::LOCATION)
                            .and_then(|v| v.to_str().ok())
                            .ok_or_else(|| anyhow::anyhow!("redirect with no Location header"))?;
                        current = current.join(loc).map_err(|_| anyhow::anyhow!("bad redirect target"))?;
                        continue;
                    }
                    if status.is_server_error() {
                        anyhow::bail!("upstream error HTTP {} fetching the page", status.as_u16());
                    }
                    if !status.is_success() {
                        anyhow::bail!("HTTP {} fetching {current}", status.as_u16());
                    }
                    let ctype = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let bytes = bytes_bounded(resp, MAX_FETCH_BYTES).await?;
                    return Ok::<_, anyhow::Error>((ctype, bytes));
                }
                anyhow::bail!("too many redirects fetching {start}")
            }
        })
        .await?;
        let (ctype, bytes) = body;
        let raw = String::from_utf8_lossy(&bytes);
        // HTML → text; plain text / json / csv pass through (just capped).
        let text = if ctype.contains("html") || raw.trim_start().starts_with('<') {
            html_to_text(&raw)
        } else {
            raw.into_owned()
        };
        Ok(cap_chars(&text, MAX_TEXT_CHARS))
    }

    /// Search the whole web via DuckDuckGo's HTML endpoint → up to `limit` (title, url, snippet).
    /// Best-effort: an empty query returns `[]`; a blocked/changed page returns `[]` (the agent then
    /// falls back to litsearch or a direct URL) rather than erroring the whole run.
    pub async fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<WebResult>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let limit = limit.clamp(1, 15);
        let html = retry_async(&RetryPolicy::default(), is_retryable_error, || {
            let client = self.client.clone();
            let q = q.to_string();
            async move {
                let resp = client
                    .get("https://html.duckduckgo.com/html/")
                    .query(&[("q", q.as_str())])
                    .send()
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                if resp.status().is_server_error() {
                    anyhow::bail!("search upstream error HTTP {}", resp.status().as_u16());
                }
                let bytes = bytes_bounded(resp, MAX_FETCH_BYTES).await?;
                Ok::<_, anyhow::Error>(String::from_utf8_lossy(&bytes).into_owned())
            }
        })
        .await?;
        Ok(parse_ddg(&html, limit))
    }
}

/// True for any IP we must never fetch: loopback / private / link-local (cloud metadata 169.254.169.254) /
/// reserved. Checks the REAL address, so decimal/octal/hex/IPv4-mapped-IPv6 encodings all collapse here.
fn ip_is_forbidden(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || v4.is_private() || v4.is_link_local()
                || v4.is_broadcast() || v4.is_unspecified() || v4.is_documentation()
                || o[0] == 0                                    // 0.0.0.0/8
                || (o[0] == 100 && (64..128).contains(&o[1]))   // 100.64/10 CGNAT
                || o[0] >= 240                                  // 240/4 reserved
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00        // fc00::/7 ULA
                || (v6.segments()[0] & 0xffc0) == 0xfe80        // fe80::/10 link-local
                || v6.to_ipv4_mapped().map(|m| ip_is_forbidden(IpAddr::V4(m))).unwrap_or(false)
                || v6.to_ipv4().map(|m| ip_is_forbidden(IpAddr::V4(m))).unwrap_or(false)
        }
    }
}

/// Refuse non-http(s) schemes, internal hostnames, and hosts that RESOLVE to a forbidden IP. Unlike a
/// lexical blocklist this parses the URL and runs the resolver, so decimal/octal/hex/IPv4-mapped IP
/// encodings AND DNS names that point at an internal address are all caught. Called before EVERY
/// redirect hop (see `fetch`) so a public URL can't 302 into the metadata endpoint.
async fn guard_public_url(raw: &str) -> anyhow::Result<()> {
    let u = reqwest::Url::parse(raw.trim()).map_err(|_| anyhow::anyhow!("invalid URL"))?;
    match u.scheme() {
        "http" | "https" => {}
        s => anyhow::bail!("web_fetch only allows http(s) URLs (got {s})"),
    }
    let host = u.host_str().ok_or_else(|| anyhow::anyhow!("URL has no host"))?.to_string();
    let port = u.port_or_known_default().unwrap_or(80);
    // host_str() brackets an IPv6 literal ("[::ffff:a9fe:a9fe]") — strip them for the IpAddr fast-path.
    let bare = host.trim_start_matches('[').trim_end_matches(']').to_string();
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        if ip_is_forbidden(ip) {
            anyhow::bail!("refused to fetch an internal IP ({ip})");
        }
        return Ok(());
    }
    let h = bare.to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local")
        || h.ends_with(".internal") || h == "metadata.google.internal"
    {
        anyhow::bail!("refused to fetch an internal host ({h})");
    }
    // resolve off the async runtime (getaddrinfo is blocking)
    let rhost = bare.clone();
    let addrs = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (rhost.as_str(), port).to_socket_addrs().map(|it| it.map(|s| s.ip()).collect::<Vec<_>>())
    })
    .await
    .map_err(|e| anyhow::anyhow!(e.to_string()))?
    .map_err(|e| anyhow::anyhow!("could not resolve {host}: {e}"))?;
    if addrs.is_empty() {
        anyhow::bail!("host {host} did not resolve");
    }
    for ip in addrs {
        if ip_is_forbidden(ip) {
            anyhow::bail!("refused: {host} resolves to an internal IP ({ip})");
        }
    }
    Ok(())
}

/// Strip HTML to readable text: drop script/style blocks, remove tags, decode a few common
/// entities, and collapse whitespace. Deliberately simple (no dependency) — good enough to read the
/// gist of an article / abstract / doc.
pub fn html_to_text(html: &str) -> String {
    // Drop content-bearing non-text blocks. Note: NOT <head> — matching "<head" also catches
    // "<header>", and with </head> already gone the fallback would delete the whole body. The head's
    // residual text (a <title>) is harmless; its scripts/styles are removed here anyway.
    let mut s = html.to_string();
    for (open, close) in [("<script", "</script>"), ("<style", "</style>"), ("<!--", "-->"), ("<noscript", "</noscript>")] {
        loop {
            let lower = s.to_ascii_lowercase();
            let Some(a) = lower.find(open) else { break };
            let end = lower[a..].find(close).map(|e| a + e + close.len()).unwrap_or(s.len());
            s.replace_range(a..end, " ");
        }
    }
    // block tags → newline so paragraphs/list items don't run together
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut last_tag_start = 0usize;
    let bytes: Vec<char> = s.chars().collect();
    for (i, &c) in bytes.iter().enumerate() {
        if c == '<' {
            in_tag = true;
            last_tag_start = i;
        } else if c == '>' && in_tag {
            in_tag = false;
            let tag: String = bytes[last_tag_start + 1..i].iter().take(12).collect::<String>().to_ascii_lowercase();
            if tag.starts_with("br") || tag.starts_with("/p") || tag.starts_with("/div") || tag.starts_with("/li") || tag.starts_with("/h") || tag.starts_with("/tr") {
                out.push('\n');
            }
        } else if !in_tag {
            out.push(c);
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–");
    // collapse runs of blank lines + trailing spaces
    let mut lines: Vec<String> = Vec::new();
    let mut blanks = 0;
    for raw in out.lines() {
        let t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            blanks += 1;
            if blanks <= 1 && !lines.is_empty() {
                lines.push(String::new());
            }
        } else {
            blanks = 0;
            lines.push(t);
        }
    }
    lines.join("\n").trim().to_string()
}

/// Parse DuckDuckGo HTML results into (title, url, snippet). Tolerant string scan (no regex dep).
fn parse_ddg(html: &str, limit: usize) -> Vec<WebResult> {
    let mut out = Vec::new();
    // Each result anchor: class="result__a" ... href="<link>">Title</a>
    for chunk in html.split("result__a").skip(1) {
        if out.len() >= limit {
            break;
        }
        let Some(href_i) = chunk.find("href=\"") else { continue };
        let rest = &chunk[href_i + 6..];
        let Some(end) = rest.find('"') else { continue };
        let raw_href = &rest[..end];
        let url = ddg_unwrap(raw_href);
        if url.is_empty() {
            continue;
        }
        // title = text between the anchor's ">" and "</a>"
        let title = rest[end..]
            .find('>')
            .map(|g| &rest[end + g + 1..])
            .and_then(|t| t.find("</a>").map(|e| &t[..e]))
            .map(|t| html_to_text(t))
            .unwrap_or_default();
        // snippet (best-effort): the next result__snippet block in this chunk
        let snippet = chunk
            .find("result__snippet")
            .and_then(|s| chunk[s..].find('>').map(|g| &chunk[s + g + 1..]))
            .and_then(|t| t.find("</a>").map(|e| &t[..e]))
            .map(|t| html_to_text(t))
            .unwrap_or_default();
        if !title.is_empty() {
            out.push(WebResult { title: title.trim().to_string(), url, snippet: snippet.trim().chars().take(300).collect() });
        }
    }
    out
}

/// DDG wraps result links as `//duckduckgo.com/l/?uddg=<percent-encoded real url>&...`. Unwrap it.
fn ddg_unwrap(href: &str) -> String {
    let target = if let Some(i) = href.find("uddg=") {
        let rest = &href[i + 5..];
        let enc = rest.split('&').next().unwrap_or(rest);
        percent_decode(enc)
    } else {
        href.to_string()
    };
    let t = target.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else if let Some(rest) = t.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        String::new()
    }
}

/// Minimal percent-decode (enough for a URL query value; '+' → space).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn cap_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push_str("\n…[truncated]");
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ssrf_guard_blocks_internal() {
        for u in [
            "http://localhost/x",
            "http://127.0.0.1:8080/",
            "http://169.254.169.254/latest/meta-data/", // cloud metadata
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://foo.internal/",
            "ftp://example.com/",
            "http://[::ffff:169.254.169.254]/", // IPv4-mapped IPv6 — the old lexical guard missed this
        ] {
            assert!(guard_public_url(u).await.is_err(), "should refuse {u}");
        }
        // a public IP literal is allowed (no DNS lookup needed, deterministic offline)
        assert!(guard_public_url("http://172.32.0.1/").await.is_ok());
    }

    #[test]
    fn html_to_text_strips_and_collapses() {
        let h = "<html><head><style>x{}</style></head><body><h1>Title</h1><p>Hello&nbsp;<b>world</b></p><script>bad()</script></body></html>";
        let t = html_to_text(h);
        assert!(t.contains("Title"));
        assert!(t.contains("Hello world"));
        assert!(!t.contains("bad()"));
        assert!(!t.contains("x{}"));
    }

    #[test]
    fn ddg_unwrap_decodes() {
        let u = ddg_unwrap("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%20b&rut=z");
        assert_eq!(u, "https://example.com/a b");
    }
}
