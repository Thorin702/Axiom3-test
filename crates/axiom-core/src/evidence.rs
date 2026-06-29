//! Evidence retrieval — rank source passages by relevance to a claim.
//!
//! Deterministic BM25-lite over term overlap (no embeddings, no model, no network):
//! it splits the user's reference sources into passages and, for a claim, returns
//! the passages most likely to bear on it. This is what grounds verification in
//! REAL source text instead of a model's parametric memory — the moat.
//!
//! Bilingual: Latin word tokens + CJK character bigrams.

use std::collections::HashMap;

/// A retrievable chunk of a source document.
#[derive(Debug, Clone, PartialEq)]
pub struct Passage {
    pub source_id: String,
    pub text: String,
}

/// A passage scored for relevance to a query.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub index: usize,
    pub score: f32,
}

/// Split a reference document into retrievable passages — one per non-trivial
/// line / sentence-ish chunk. Crude but adequate; a smarter chunker is a follow-up.
pub fn split_passages(source_id: &str, text: &str) -> Vec<Passage> {
    text.split(['\n', '。', '！', '？'])
        .flat_map(|line| line.split(". "))
        .map(str::trim)
        .filter(|s| s.chars().filter(|c| c.is_alphanumeric()).count() >= 12)
        .map(|s| Passage { source_id: source_id.to_string(), text: s.to_string() })
        .collect()
}

/// Rank `passages` by BM25-lite relevance to `query`; return the top `k` with a
/// positive score, best first. Empty if nothing shares a meaningful term.
pub fn rank_passages(query: &str, passages: &[Passage], k: usize) -> Vec<Scored> {
    if passages.is_empty() {
        return vec![];
    }
    let q_terms = tokenize(query);
    if q_terms.is_empty() {
        return vec![];
    }
    let docs: Vec<Vec<String>> = passages.iter().map(|p| tokenize(&p.text)).collect();
    let n = docs.len() as f32;
    let avg_len: f32 = docs.iter().map(|d| d.len()).sum::<usize>() as f32 / n;

    // Document frequency per term.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for doc in &docs {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for t in doc {
            if seen.insert(t.as_str()) {
                *df.entry(t.as_str()).or_insert(0) += 1;
            }
        }
    }

    const K1: f32 = 1.2;
    const B: f32 = 0.75;
    let q_unique: std::collections::HashSet<&str> = q_terms.iter().map(|s| s.as_str()).collect();

    let mut scored: Vec<Scored> = docs
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            let len = doc.len() as f32;
            let mut tf: HashMap<&str, f32> = HashMap::new();
            for t in doc {
                *tf.entry(t.as_str()).or_insert(0.0) += 1.0;
            }
            let mut score = 0.0;
            for &t in &q_unique {
                let f = *tf.get(t).unwrap_or(&0.0);
                if f == 0.0 {
                    continue;
                }
                let df_t = *df.get(t).unwrap_or(&0) as f32;
                let idf = ((n - df_t + 0.5) / (df_t + 0.5) + 1.0).ln();
                score += idf * (f * (K1 + 1.0)) / (f + K1 * (1.0 - B + B * len / avg_len.max(1.0)));
            }
            Scored { index: i, score }
        })
        .filter(|s| s.score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

/// Tokenize into Latin word tokens (≥2 chars, non-stopword) + CJK character bigrams.
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut latin = String::new();
    let mut cjk: Vec<char> = Vec::new();
    let flush_latin = |latin: &mut String, out: &mut Vec<String>| {
        let w = latin.to_lowercase();
        if w.chars().count() >= 2 && !STOPWORDS.contains(&w.as_str()) {
            out.push(w);
        }
        latin.clear();
    };
    let flush_cjk = |cjk: &mut Vec<char>, out: &mut Vec<String>| {
        // character bigrams (and the single char if length 1)
        if cjk.len() == 1 {
            out.push(cjk[0].to_string());
        } else {
            for w in cjk.windows(2) {
                out.push(w.iter().collect());
            }
        }
        cjk.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            latin.push(ch);
        } else if is_cjk(ch) {
            flush_latin(&mut latin, &mut out);
            cjk.push(ch);
        } else {
            flush_latin(&mut latin, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_latin(&mut latin, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32, 0x4E00..=0x9FFF)
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "is", "are", "was", "were", "and", "or",
    "for", "on", "at", "by", "with", "as", "that", "this", "these", "those", "it",
    "we", "our", "be", "been", "has", "have", "had", "not", "but", "from", "which",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Passage {
        Passage { source_id: "s1".into(), text: text.into() }
    }

    #[test]
    fn ranks_the_supporting_passage_first() {
        let passages = vec![
            p("The faradaic efficiency of the Pd-Cu catalyst reached 94 percent at 80 degrees."),
            p("The weather in Beijing was sunny during the conference last week."),
            p("Researchers have long studied carbon dioxide reduction reactions in general."),
        ];
        let hits = rank_passages("Pd-Cu catalyst faradaic efficiency 94 percent", &passages, 2);
        assert!(!hits.is_empty(), "should find a relevant passage");
        assert_eq!(hits[0].index, 0, "the matching passage ranks first");
    }

    #[test]
    fn returns_empty_when_nothing_relevant() {
        let passages = vec![p("Completely unrelated text about gardening and flowers.")];
        let hits = rank_passages("quantum chromodynamics lattice gauge theory", &passages, 3);
        assert!(hits.is_empty(), "no shared terms → no evidence");
    }

    #[test]
    fn split_passages_drops_trivial_lines() {
        let text = "The yield was 94 percent under standard conditions.\nok\n\nA second substantive sentence about catalysis follows here.";
        let ps = split_passages("src", text);
        assert_eq!(ps.len(), 2, "the 2-char 'ok' line is dropped");
        assert!(ps[0].text.contains("94 percent"));
    }

    #[test]
    fn cjk_retrieval_works() {
        let passages = vec![
            p("催化剂的法拉第效率在八十摄氏度下达到百分之九十四。"),
            p("会议期间北京天气晴朗。"),
        ];
        let hits = rank_passages("法拉第效率 催化剂", &passages, 1);
        assert_eq!(hits[0].index, 0, "CJK bigram overlap finds the catalysis passage");
    }
}
