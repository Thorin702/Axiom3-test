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
        // Must start "10." at a non-digit boundary (so "810.1234/x" isn't a DOI).
        let boundary = i == 0 || !c[i - 1].is_ascii_digit();
        if boundary && c[i] == '1' && c[i + 1] == '0' && c[i + 2] == '.' {
            let dstart = i + 3;
            let mut j = dstart;
            while j < c.len() && c[j].is_ascii_digit() {
                j += 1;
            }
            let registrant_len = j - dstart;
            if (4..=9).contains(&registrant_len) && c.get(j) == Some(&'/') {
                // Suffix runs to whitespace; trim trailing sentence punctuation.
                let mut k = j + 1;
                while k < c.len() && !c[k].is_whitespace() {
                    k += 1;
                }
                let mut end = k;
                while end > j + 1
                    && matches!(c[end - 1], '.' | ',' | ';' | ')' | ']' | '}' | '"' | '\'' | '>' | '。' | '，')
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
    fn handles_real_retracted_doi_with_parens() {
        // The Wakefield Lancet DOI has parens in the suffix.
        let dois = extract_dois("as in 10.1016/S0140-6736(97)11096-0 which was retracted");
        assert_eq!(dois, vec!["10.1016/S0140-6736(97)11096-0"]);
    }
}
