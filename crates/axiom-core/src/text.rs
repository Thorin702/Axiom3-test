//! Shared text helpers for the bilingual engine.
//!
//! These live in one place so the deterministic verifier ([`crate::verify`]) and the
//! evidence retriever ([`crate::evidence`]) can NEVER disagree about what a character
//! is — segmenting the same draft two different ways was a real integration bug.

/// True for CJK ideographs and Japanese kana — the scripts we tokenize by character
/// (n-grams) rather than by whitespace-delimited words. Deliberately broad and SHARED:
/// claim extraction and passage retrieval must classify identical text identically.
pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF |   // Hiragana + Katakana
        0x3400..=0x4DBF |   // CJK Extension A
        0x4E00..=0x9FFF |   // CJK Unified Ideographs
        0xF900..=0xFAFF)    // CJK Compatibility Ideographs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_the_common_cjk_and_kana_blocks() {
        assert!(is_cjk('催')); // Unified
        assert!(is_cjk('あ')); // Hiragana
        assert!(is_cjk('カ')); // Katakana
        assert!(is_cjk('㐀')); // Extension A (U+3400)
        assert!(!is_cjk('a'));
        assert!(!is_cjk('7'));
        assert!(!is_cjk('%'));
    }
}
