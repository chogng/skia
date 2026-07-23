use unicode_linebreak::{BreakClass, BreakOpportunity, break_property};

// Unicode 15: Line_Break=OP intersected with East_Asian_Width=F/W/H.
const EAST_ASIAN_OPEN_PUNCTUATION: &[u32] = &[
    0x2329, 0x3008, 0x300A, 0x300C, 0x300E, 0x3010, 0x3014, 0x3016, 0x3018, 0x301A, 0x301D, 0xFE17,
    0xFE35, 0xFE37, 0xFE39, 0xFE3B, 0xFE3D, 0xFE3F, 0xFE41, 0xFE43, 0xFE47, 0xFE59, 0xFE5B, 0xFE5D,
    0xFF08, 0xFF3B, 0xFF5B, 0xFF5F, 0xFF62,
];

// Unicode 15: Extended_Pictographic intersected with General_Category=Cn.
const UNASSIGNED_EXTENDED_PICTOGRAPHIC: &[(u32, u32)] = &[
    (0x1F02C, 0x1F02F),
    (0x1F094, 0x1F09F),
    (0x1F0AF, 0x1F0B0),
    (0x1F0C0, 0x1F0C0),
    (0x1F0D0, 0x1F0D0),
    (0x1F0F6, 0x1F0FF),
    (0x1F1AE, 0x1F1E5),
    (0x1F203, 0x1F20F),
    (0x1F23C, 0x1F23F),
    (0x1F249, 0x1F24F),
    (0x1F252, 0x1F25F),
    (0x1F266, 0x1F2FF),
    (0x1F6D8, 0x1F6DB),
    (0x1F6ED, 0x1F6EF),
    (0x1F6FD, 0x1F6FF),
    (0x1F777, 0x1F77A),
    (0x1F7DA, 0x1F7DF),
    (0x1F7EC, 0x1F7EF),
    (0x1F7F1, 0x1F7FF),
    (0x1F80C, 0x1F80F),
    (0x1F848, 0x1F84F),
    (0x1F85A, 0x1F85F),
    (0x1F888, 0x1F88F),
    (0x1F8AE, 0x1F8AF),
    (0x1F8B2, 0x1F8FF),
    (0x1FA54, 0x1FA5F),
    (0x1FA6E, 0x1FA6F),
    (0x1FA7D, 0x1FA7F),
    (0x1FA89, 0x1FA8F),
    (0x1FABE, 0x1FABE),
    (0x1FAC6, 0x1FACD),
    (0x1FADC, 0x1FADF),
    (0x1FAE9, 0x1FAEF),
    (0x1FAF9, 0x1FAFF),
    (0x1FC00, 0x1FFFD),
];

#[derive(Clone, Copy)]
struct LineToken {
    start: usize,
    end: usize,
    scalar: u32,
    class: BreakClass,
    ends_with_zwj: bool,
}

/// Applies the Unicode 15 conformance test's documented regex-number
/// tailoring on top of `unicode-linebreak`, plus the LB30 East Asian opening-
/// punctuation exception and LB30b potential-emoji rule omitted by its pair
/// table.
pub(crate) fn linebreaks(text: &str) -> std::vec::IntoIter<(usize, BreakOpportunity)> {
    let tokens = line_tokens(text);
    let protected = numeric_expression_boundaries(&tokens);
    let mut breaks: Vec<_> = unicode_linebreak::linebreaks(text).collect();

    breaks.retain(|(offset, _)| protected.binary_search(offset).is_err());
    for pair in tokens.windows(2) {
        let left = pair[0];
        let right = pair[1];
        let boundary = right.start;
        if protected.binary_search(&boundary).is_ok() {
            continue;
        }
        if numeric_pair_table_overreach(left.class, right.class)
            || wide_opening_overreach(left, right)
        {
            insert_allowed(&mut breaks, boundary);
        }
    }

    for pair in tokens.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if right.class == BreakClass::EmojiModifier
            && is_unassigned_extended_pictographic(left.scalar)
        {
            remove_break(&mut breaks, right.start);
        }
    }
    breaks.into_iter()
}

fn line_tokens(text: &str) -> Vec<LineToken> {
    let mut tokens: Vec<LineToken> = Vec::new();
    for (start, character) in text.char_indices() {
        let end = start + character.len_utf8();
        let raw = break_property(character as u32);
        if matches!(raw, BreakClass::CombiningMark | BreakClass::ZeroWidthJoiner)
            && let Some(previous) = tokens.last_mut()
            && accepts_combining_mark(previous.class)
        {
            previous.end = end;
            previous.ends_with_zwj = raw == BreakClass::ZeroWidthJoiner;
            continue;
        }
        tokens.push(LineToken {
            start,
            end,
            scalar: character as u32,
            class: resolve_class(raw),
            ends_with_zwj: raw == BreakClass::ZeroWidthJoiner,
        });
    }
    tokens
}

const fn accepts_combining_mark(class: BreakClass) -> bool {
    !matches!(
        class,
        BreakClass::Mandatory
            | BreakClass::CarriageReturn
            | BreakClass::LineFeed
            | BreakClass::NextLine
            | BreakClass::Space
            | BreakClass::ZeroWidthSpace
    )
}

const fn resolve_class(class: BreakClass) -> BreakClass {
    match class {
        BreakClass::Ambiguous
        | BreakClass::ComplexContext
        | BreakClass::Surrogate
        | BreakClass::Unknown
        | BreakClass::CombiningMark
        | BreakClass::ZeroWidthJoiner => BreakClass::Alphabetic,
        BreakClass::ConditionalJapaneseStarter => BreakClass::NonStarter,
        class => class,
    }
}

fn numeric_expression_boundaries(tokens: &[LineToken]) -> Vec<usize> {
    let mut protected = Vec::new();
    for numeric in 0..tokens.len() {
        if tokens[numeric].class != BreakClass::Numeric {
            continue;
        }
        let mut start = numeric;
        if start > 0
            && matches!(
                tokens[start - 1].class,
                BreakClass::OpenPunctuation | BreakClass::Hyphen
            )
        {
            start -= 1;
        }
        if start > 0
            && matches!(
                tokens[start - 1].class,
                BreakClass::Prefix | BreakClass::Postfix
            )
        {
            start -= 1;
        }

        let mut end = numeric + 1;
        while end < tokens.len()
            && matches!(
                tokens[end].class,
                BreakClass::Numeric | BreakClass::Symbol | BreakClass::InfixSeparator
            )
        {
            end += 1;
        }
        if end < tokens.len()
            && matches!(
                tokens[end].class,
                BreakClass::ClosePunctuation | BreakClass::CloseParenthesis
            )
        {
            end += 1;
        }
        if end < tokens.len()
            && matches!(tokens[end].class, BreakClass::Prefix | BreakClass::Postfix)
        {
            end += 1;
        }
        protected.extend(tokens[start + 1..end].iter().map(|token| token.start));
    }
    protected.sort_unstable();
    protected.dedup();
    protected
}

const fn numeric_pair_table_overreach(left: BreakClass, right: BreakClass) -> bool {
    matches!(
        (left, right),
        (
            BreakClass::ClosePunctuation | BreakClass::CloseParenthesis,
            BreakClass::Prefix | BreakClass::Postfix
        ) | (
            BreakClass::Prefix | BreakClass::Postfix,
            BreakClass::OpenPunctuation
        ) | (
            BreakClass::InfixSeparator | BreakClass::Symbol,
            BreakClass::Numeric
        )
    )
}

fn wide_opening_overreach(left: LineToken, right: LineToken) -> bool {
    !left.ends_with_zwj
        && matches!(
            left.class,
            BreakClass::Alphabetic | BreakClass::HebrewLetter | BreakClass::Numeric
        )
        && right.class == BreakClass::OpenPunctuation
        && EAST_ASIAN_OPEN_PUNCTUATION
            .binary_search(&right.scalar)
            .is_ok()
}

fn is_unassigned_extended_pictographic(scalar: u32) -> bool {
    UNASSIGNED_EXTENDED_PICTOGRAPHIC
        .iter()
        .any(|&(start, end)| scalar >= start && scalar <= end)
}

fn insert_allowed(breaks: &mut Vec<(usize, BreakOpportunity)>, offset: usize) {
    if let Err(index) = breaks.binary_search_by_key(&offset, |(offset, _)| *offset) {
        breaks.insert(index, (offset, BreakOpportunity::Allowed));
    }
}

fn remove_break(breaks: &mut Vec<(usize, BreakOpportunity)>, offset: usize) {
    if let Ok(index) = breaks.binary_search_by_key(&offset, |(offset, _)| *offset) {
        breaks.remove(index);
    }
}

#[cfg(test)]
#[path = "line_break_tests.rs"]
mod tests;
