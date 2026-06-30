//! DOI extraction — pure, deterministic, offline.
//!
//! Pulls Digital Object Identifiers out of a claim's text so the (network) retraction
//! / existence checker can verify them. A DOI is `10.<registrant>/<suffix>` where the
//! registrant is 4–9 digits; requiring digits-then-`/` keeps "10.5 mg" / "version
//! 10.2" from matching. Recognises bare DOIs and the `doi:` / `https://doi.org/`
//! prefixes (the scanner simply finds the `10.` regardless of any prefix before it).

use std::collections::HashSet;

/// Extract every distinct DOI in `text` (case-insensitively deduplicated, first form
/// kept). Regex-free.
pub fn extract_dois(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;
    while i + 2 < c.len() {
        // Must start "10." at a boundary that isn't a digit OR a '.', so neither
        // "810.1234/x" nor a dotted-quad/CIDR like "192.168.10.1234/24" matches.
        let boundary = i == 0 || !(c[i - 1].is_ascii_digit() || c[i - 1] == '.');
        if boundary && c[i] == '1' && c[i + 1] == '0' && c[i + 2] == '.' {
            let dstart = i + 3;
            let mut j = dstart;
            while j < c.len() && c[j].is_ascii_digit() {
                j += 1;
            }
            let registrant_len = j - dstart;
            if (4..=9).contains(&registrant_len) && c.get(j) == Some(&'/') {
                // The suffix is ASCII-graphic, so it terminates at whitespace AND at any
                // non-ASCII char — e.g. CJK punctuation/Han with no space (参见10.x/y。该)
                // would otherwise glue prose onto the DOI and 404 a real work. It also
                // stops at <>?# so an angle-bracketed DOI or a query/fragment-injection
                // suffix can't leak into the (network) URL. Trailing ASCII punctuation
                // is then trimmed.
                let mut k = j + 1;
                while k < c.len()
                    && c[k].is_ascii_graphic()
                    && !matches!(c[k], '<' | '>' | '?' | '#')
                {
                    k += 1;
                }
                let mut end = k;
                while end > j + 1
                    && matches!(c[end - 1], '.' | ',' | ';' | ')' | ']' | '}' | '"' | '\'')
                {
                    end -= 1;
                }
                if end > j + 1 {
                    let doi: String = c[i..end].iter().collect();
                    if seen.insert(doi.to_lowercase()) {
                        out.push(doi);
                    }
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bare_and_prefixed_dois() {
        let dois = extract_dois(
            "See doi:10.1016/j.foo.2020.01.001 and https://doi.org/10.1038/nature12373.",
        );
        assert_eq!(dois, vec!["10.1016/j.foo.2020.01.001", "10.1038/nature12373"]);
    }

    #[test]
    fn trims_trailing_punctuation_and_brackets() {
        assert_eq!(extract_dois("(10.1234/abc)."), vec!["10.1234/abc"]);
        assert_eq!(extract_dois("[10.5555/xyz];"), vec!["10.5555/xyz"]);
    }

    #[test]
    fn ignores_non_doi_decimals() {
        assert!(extract_dois("The dose was 10.5 mg, version 10.2, ratio 10.25/100.").is_empty());
        // registrant must be 4–9 digits, then a slash
        assert!(extract_dois("10.99/x (too few digits)").is_empty());
    }

    #[test]
    fn deduplicates_case_insensitively() {
        let dois = extract_dois("10.1016/ABC and again 10.1016/abc and 10.1016/abc");
        assert_eq!(dois.len(), 1, "{dois:?}");
    }

    #[test]
    fn terminates_at_cjk_with_no_space() {
        // Chinese typography puts no space between Han and inline Latin; the DOI must
        // not absorb the following characters (which would 404 a real work).
        assert_eq!(extract_dois("参见10.1038/nature12373。该研究"), vec!["10.1038/nature12373"]);
        assert_eq!(extract_dois("引用10.1038/nature12373，并指出"), vec!["10.1038/nature12373"]);
    }

    #[test]
    fn rejects_dotted_quad_and_injection_chars() {
        // An IP/CIDR last octet is not a DOI.
        assert!(extract_dois("192.168.10.1234/24 subnet").is_empty());
        // Query/fragment/angle-bracket terminate the suffix (no URL injection leaks).
        assert_eq!(extract_dois("10.1234/x?rows=1000"), vec!["10.1234/x"]);
        assert_eq!(extract_dois("10.1234/x#frag"), vec!["10.1234/x"]);
        assert_eq!(extract_dois("<10.1234/abc>"), vec!["10.1234/abc"]);
    }

    #[test]
    fn handles_real_retracted_doi_with_parens() {
        // The Wakefield Lancet DOI has parens in the suffix.
        let dois = extract_dois("as in 10.1016/S0140-6736(97)11096-0 which was retracted");
        assert_eq!(dois, vec!["10.1016/S0140-6736(97)11096-0"]);
    }
}
