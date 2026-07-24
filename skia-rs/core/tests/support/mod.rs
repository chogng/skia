pub(crate) const BASIC_A: &[u8] = include_bytes!("../../../text/tests/fonts/synthetic/basic-a.ttf");
pub(crate) const DECORATED_A: &[u8] =
    include_bytes!("../../../text/tests/fonts/synthetic/decorated-a.ttf");

pub(crate) fn font_bytes(fixture: &[u8]) -> Vec<u8> {
    fixture.to_vec()
}
