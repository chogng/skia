use std::{collections::HashMap, sync::Mutex};

use hyphenation::{Hyphenator, Language, Load, Standard};

use crate::{
    TextBreakProvider, TextError, TextErrorCode, TextWordBreak, TextWordBreakKind,
    valid_language_tag,
};

/// Cached provider backed by the embedded Knuth-Liang language dictionaries.
///
/// Dictionaries are loaded on first use and retained for the provider's
/// lifetime. Unsupported but structurally valid language tags produce no
/// opportunities, allowing Unicode line breaking to remain authoritative.
pub struct BuiltinTextBreakProvider {
    dictionaries: Mutex<HashMap<Language, Standard>>,
}

impl BuiltinTextBreakProvider {
    /// Creates an empty lazy dictionary cache.
    pub fn new() -> Self {
        Self {
            dictionaries: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the embedded dictionary selected for a BCP 47-style language.
    ///
    /// Region and variant subtags fall back to their base language when an
    /// exact embedded dictionary does not exist. English defaults to US,
    /// modern German to the 1996 rules, and monotonic Greek to `el-monoton`.
    pub fn dictionary_language(language: &str) -> Result<Option<&'static str>, TextError> {
        resolve_language(language).map(|language| language.map(|language| language.code()))
    }

    /// Returns whether the language resolves to one embedded dictionary.
    pub fn supports_language(language: &str) -> Result<bool, TextError> {
        Self::dictionary_language(language).map(|language| language.is_some())
    }
}

impl Default for BuiltinTextBreakProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBreakProvider for BuiltinTextBreakProvider {
    fn opportunities(&self, word: &str, language: &str) -> Result<Vec<TextWordBreak>, TextError> {
        let Some(language) = resolve_language(language)? else {
            return Ok(Vec::new());
        };
        let mut dictionaries = self
            .dictionaries
            .lock()
            .map_err(|_| TextError::new(TextErrorCode::DictionaryUnavailable))?;
        if !dictionaries.contains_key(&language) {
            let dictionary = Standard::from_embedded(language)
                .map_err(|_| TextError::new(TextErrorCode::DictionaryUnavailable))?;
            dictionaries
                .try_reserve(1)
                .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
            dictionaries.insert(language, dictionary);
        }
        let dictionary = dictionaries
            .get(&language)
            .ok_or(TextError::new(TextErrorCode::DictionaryUnavailable))?;
        let breaks = dictionary.hyphenate(word).breaks;
        let mut opportunities = Vec::new();
        opportunities
            .try_reserve_exact(breaks.len())
            .map_err(|_| TextError::new(TextErrorCode::AllocationFailed))?;
        opportunities.extend(
            breaks
                .into_iter()
                .map(|offset| TextWordBreak::new(offset, TextWordBreakKind::Hyphenated)),
        );
        Ok(opportunities)
    }
}

fn resolve_language(language: &str) -> Result<Option<Language>, TextError> {
    if !valid_language_tag(language) {
        return Err(TextError::new(TextErrorCode::InvalidLanguage));
    }
    let normalized = language.to_ascii_lowercase();
    if let Some(language) = Language::try_from_code(&normalized) {
        return Ok(Some(language));
    }
    let base = normalized.split('-').next().unwrap_or(normalized.as_str());
    let fallback = match base {
        "de" => Some(Language::German1996),
        "el" => Some(Language::GreekMono),
        "en" => Some(Language::EnglishUS),
        "mn" => Some(Language::Mongolian),
        "sr" => Some(Language::SerbianCyrillic),
        _ => Language::try_from_code(base),
    };
    Ok(fallback)
}

#[cfg(test)]
mod tests {
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
}
