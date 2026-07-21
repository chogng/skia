use std::fmt;

/// Stable machine-readable text-resource failure.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextErrorCode {
    /// A coordinate or intermediate calculation overflowed.
    NumericOverflow,
    /// A glyph run has no glyphs.
    EmptyGlyphRun,
    /// Text supplied to the shaper is empty.
    EmptyText,
    /// A font size is zero or negative.
    InvalidFontSize,
    /// A font's units-per-em value is zero.
    InvalidUnitsPerEm,
    /// A requested font weight is outside the supported range.
    InvalidFontStyle,
    /// A variable-font instance identity or coordinate request is invalid.
    InvalidFontVariation,
    /// A shaping-feature instance identity or feature set is invalid.
    InvalidFontFeature,
    /// A language tag is empty or structurally invalid.
    InvalidLanguage,
    /// A language break provider returned a non-grapheme or out-of-word offset.
    InvalidWordBreak,
    /// An embedded language dictionary could not be loaded.
    DictionaryUnavailable,
    /// Styled paragraph spans are invalid, incomplete, or split a grapheme.
    InvalidTextStyleSpan,
    /// Font bytes are malformed or omit required tables.
    InvalidFontData,
    /// A font-collection face index is out of bounds.
    InvalidFaceIndex,
    /// A resource ceiling configuration contains zero.
    InvalidLimits,
    /// A font collection contains no faces.
    EmptyFontCollection,
    /// A font collection already contains the supplied stable font identifier.
    DuplicateFontId,
    /// No font in a collection covers one source grapheme.
    MissingGlyph,
    /// A requested decoration has no corresponding font metrics.
    MissingDecorationMetrics,
    /// Paragraph shaping received more than one paragraph.
    MultipleParagraphs,
    /// Line-break analysis did not produce a forward layout boundary.
    InvalidLayout,
    /// Glyph outline segments do not form valid contours.
    InvalidOutline,
    /// A resource ceiling was reached.
    ResourceLimit,
    /// A fallible allocation failed.
    AllocationFailed,
}

/// Source-redacted text-resource error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextError {
    code: TextErrorCode,
}

impl TextError {
    /// Creates one stable text-resource failure.
    pub const fn new(code: TextErrorCode) -> Self {
        Self { code }
    }

    /// Returns the stable failure code.
    pub const fn code(self) -> TextErrorCode {
        self.code
    }
}

impl fmt::Display for TextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.code)
    }
}

impl std::error::Error for TextError {}
