pub const BASIC_A: &[u8] = include_bytes!("../fonts/synthetic/basic-a.ttf");
pub const BASIC_ALEF: &[u8] = include_bytes!("../fonts/synthetic/basic-alef.ttf");
pub const BASIC_B: &[u8] = include_bytes!("../fonts/synthetic/basic-b.ttf");
pub const BASIC_BET: &[u8] = include_bytes!("../fonts/synthetic/basic-bet.ttf");
pub const BASIC_HAN_WEN: &[u8] = include_bytes!("../fonts/synthetic/basic-han-wen.ttf");
pub const BASIC_HAN_ZHONG: &[u8] = include_bytes!("../fonts/synthetic/basic-han-zhong.ttf");
pub const COLLECTION_AB: &[u8] = include_bytes!("../fonts/synthetic/collection-ab.ttc");
pub const COVERAGE_A_ELLIPSIS: &[u8] = include_bytes!("../fonts/synthetic/coverage-a-ellipsis.ttf");
pub const COVERAGE_CJK: &[u8] = include_bytes!("../fonts/synthetic/coverage-cjk.ttf");
pub const COVERAGE_DICTIONARY: &[u8] = include_bytes!("../fonts/synthetic/coverage-dictionary.ttf");
pub const COVERAGE_ELLIPSIS: &[u8] = include_bytes!("../fonts/synthetic/coverage-ellipsis.ttf");
pub const COVERAGE_LATIN_SPACE: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-latin-space.ttf");
pub const COVERAGE_MIXED_BIDI: &[u8] = include_bytes!("../fonts/synthetic/coverage-mixed-bidi.ttf");
pub const COVERAGE_MIXED_SCRIPT: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-mixed-script.ttf");
pub const COVERAGE_MIXED_SPACE_ALEF: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-mixed-space-alef.ttf");
pub const COVERAGE_PERIOD_FALLBACK: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-period-fallback.ttf");
pub const COVERAGE_RTL_ELLIPSIS: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-rtl-ellipsis.ttf");
pub const COVERAGE_RTL_PAIR: &[u8] = include_bytes!("../fonts/synthetic/coverage-rtl-pair.ttf");
pub const COVERAGE_SELECTION: &[u8] = include_bytes!("../fonts/synthetic/coverage-selection.ttf");
pub const COVERAGE_SPACING: &[u8] = include_bytes!("../fonts/synthetic/coverage-spacing.ttf");
pub const COVERAGE_UNICODE_SPACES: &[u8] =
    include_bytes!("../fonts/synthetic/coverage-unicode-spaces.ttf");
pub const DECORATED_A: &[u8] = include_bytes!("../fonts/synthetic/decorated-a.ttf");
pub const KERNED_A: &[u8] = include_bytes!("../fonts/synthetic/kerned-a.ttf");
pub const LIGATURE_ATOMIC: &[u8] = include_bytes!("../fonts/synthetic/ligature-atomic.ttf");
pub const LIGATURE_CARETS: &[u8] = include_bytes!("../fonts/synthetic/ligature-carets.ttf");
pub const LIGATURE_MISMATCHED: &[u8] = include_bytes!("../fonts/synthetic/ligature-mismatched.ttf");
pub const LOCALIZED_LAYOUT: &[u8] = include_bytes!("../fonts/synthetic/localized-layout.ttf");
pub const STYLE_EXAMPLE_BOLD: &[u8] = include_bytes!("../fonts/synthetic/style-example-bold.ttf");
pub const STYLE_EXAMPLE_BOLD_CONDENSED: &[u8] =
    include_bytes!("../fonts/synthetic/style-example-bold-condensed.ttf");
pub const STYLE_EXAMPLE_ITALIC: &[u8] =
    include_bytes!("../fonts/synthetic/style-example-italic.ttf");
pub const STYLE_EXAMPLE_MEDIUM: &[u8] =
    include_bytes!("../fonts/synthetic/style-example-medium.ttf");
pub const STYLE_EXAMPLE_OBLIQUE: &[u8] =
    include_bytes!("../fonts/synthetic/style-example-oblique.ttf");
pub const STYLE_EXAMPLE_REGULAR: &[u8] =
    include_bytes!("../fonts/synthetic/style-example-regular.ttf");
pub const STYLE_OTHER_REGULAR: &[u8] = include_bytes!("../fonts/synthetic/style-other-regular.ttf");
pub const STYLED_LARGE: &[u8] = include_bytes!("../fonts/synthetic/styled-large.ttf");
pub const STYLED_SMALL: &[u8] = include_bytes!("../fonts/synthetic/styled-small.ttf");
pub const VARIABLE: &[u8] = include_bytes!("../fonts/synthetic/variable.ttf");

pub const SKIA_EM: &[u8] = include_bytes!("../fonts/skia/resources/fonts/Em.ttf");
pub const SKIA_TEST_COLLECTION: &[u8] = include_bytes!("../fonts/skia/resources/fonts/test.ttc");
pub const SKIA_VARIABLE: &[u8] = include_bytes!("../fonts/skia/resources/fonts/Variable.ttf");

pub fn bytes(fixture: &[u8]) -> Vec<u8> {
    fixture.to_vec()
}
