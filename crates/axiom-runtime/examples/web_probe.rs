//! Live check of the web tools: web_search (DuckDuckGo) + web_fetch (read a public URL).
//!   cargo run -p axiom-runtime --example web_probe
use axiom_runtime::web::WebClient;

#[tokio::main]
async fn main() {
    let c = WebClient::new();

    println!("=== web_search: 'CRISPR base editing off-target' ===");
    match c.search("CRISPR base editing off-target review", 5).await {
        Ok(rs) if rs.is_empty() => println!("(no results — DDG may have blocked the request)"),
        Ok(rs) => {
            for (i, r) in rs.iter().enumerate() {
                println!("[{}] {}\n    {}\n    {}", i + 1, r.title, r.url, r.snippet.chars().take(120).collect::<String>());
            }
        }
        Err(e) => println!("search ERROR: {e}"),
    }

    println!("\n=== web_fetch: https://en.wikipedia.org/wiki/CRISPR ===");
    match c.fetch("https://en.wikipedia.org/wiki/CRISPR").await {
        Ok(t) => {
            println!("fetched {} chars; first 400:\n{}", t.chars().count(), t.chars().take(400).collect::<String>());
        }
        Err(e) => println!("fetch ERROR: {e}"),
    }

    println!("\n=== SSRF guard (must be refused): http://169.254.169.254/ ===");
    match c.fetch("http://169.254.169.254/latest/meta-data/").await {
        Ok(_) => println!("!!! FAIL — internal fetch was allowed"),
        Err(e) => println!("refused ✓ — {e}"),
    }
}
