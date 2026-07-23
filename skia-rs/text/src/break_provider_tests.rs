use super::BuiltinTextBreakProvider;
use crate::{TextBreakProvider, TextErrorCode, TextWordBreakKind};

#[test]
fn resolves_exact_and_base_language_tags() {
    assert_eq!(
        BuiltinTextBreakProvider::dictionary_language("en-GB").expect("English GB"),
        Some("en-gb")
    );
    assert_eq!(
        BuiltinTextBreakProvider::dictionary_language("en-CA").expect("English fallback"),
        Some("en-us")
    );
    assert_eq!(
        BuiltinTextBreakProvider::dictionary_language("de-DE").expect("German fallback"),
        Some("de-1996")
    );
    assert_eq!(
        BuiltinTextBreakProvider::dictionary_language("ja").expect("unsupported"),
        None
    );
}

#[test]
fn rejects_malformed_language_tags() {
    assert_eq!(
        BuiltinTextBreakProvider::supports_language("en--US")
            .expect_err("invalid tag")
            .code(),
        TextErrorCode::InvalidLanguage
    );
}

#[test]
fn returns_grapheme_safe_hyphenated_offsets() {
    let provider = BuiltinTextBreakProvider::new();
    let word = "hyphenation";
    let opportunities = provider
        .opportunities(word, "en-US")
        .expect("embedded dictionary");
    assert!(!opportunities.is_empty());
    assert!(opportunities.iter().all(|opportunity| {
        opportunity.offset() > 0
            && opportunity.offset() < word.len()
            && word.is_char_boundary(opportunity.offset())
            && opportunity.kind() == TextWordBreakKind::Hyphenated
    }));
    assert!(
        provider
            .opportunities("日本語", "ja")
            .expect("unsupported language")
            .is_empty()
    );
}
