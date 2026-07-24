pub const BASIC_A: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-a.ttf");
pub const BASIC_ALEF: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-alef.ttf");
pub const BASIC_B: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-b.ttf");
pub const COVERAGE_CPU_FALLBACK: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/coverage-cpu-fallback.ttf");
pub const COVERAGE_HYPHENATION: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/coverage-hyphenation.ttf");
pub const COVERAGE_LATIN_SPACE: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/coverage-latin-space.ttf");
pub const STYLED_DECORATED: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/styled-decorated.ttf");
pub const STYLED_DECORATION_PATTERNS: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/styled-decoration-patterns.ttf");
pub const STYLED_DISPLAY_LAYOUT: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/styled-display-layout.ttf");
pub const STYLED_SPAN_STYLES: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/styled-span-styles.ttf");

pub fn bytes(fixture: &[u8]) -> Vec<u8> {
    fixture.to_vec()
}
