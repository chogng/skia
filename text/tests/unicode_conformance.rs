use std::{env, fs, path::PathBuf};

use unicode_bidi::{BidiInfo, LTR_LEVEL, RTL_LEVEL};
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;

const DATA_DIRECTORY_ENV: &str = "SKIA_UNICODE_CONFORMANCE_DIR";

#[test]
fn unicode_dependency_versions_match_the_pinned_conformance_data() {
    assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
    assert_eq!(unicode_linebreak::UNICODE_VERSION, (15, 0, 0));
    assert_eq!(unicode_bidi::UNICODE_VERSION, (16, 0, 0));
}

#[test]
#[ignore = "requires scripts/fetch_unicode_conformance.sh"]
fn extended_grapheme_boundaries_conform_to_unicode_17() {
    let data = read_data("GraphemeBreakTest-17.0.0.txt");
    let mut cases = 0_usize;
    for (line_index, line) in data.lines().enumerate() {
        let Some(case) = parse_boundary_case(line, line_index + 1) else {
            continue;
        };
        let mut actual: Vec<usize> = case
            .text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .collect();
        actual.push(case.text.len());
        assert_eq!(actual, case.breaks, "Unicode data line {}", line_index + 1);
        cases += 1;
    }
    assert!(cases > 700, "expected the complete grapheme test data");
}

#[test]
#[ignore = "requires scripts/fetch_unicode_conformance.sh"]
fn default_line_breaks_match_the_unicode_15_conformance_baseline() {
    const KNOWN_DEVIATIONS: &[usize] = &[
        125, 127, 815, 1161, 1163, 1165, 1167, 1331, 2189, 2191, 2873, 2875, 3567, 3739, 4081,
        4083, 4425, 4427, 4473, 4475, 4597, 4599, 4645, 4647, 4943, 5109, 5111, 5459, 6149, 6151,
        6153, 6155, 6489, 6491, 6663, 6833, 6835, 7005, 7007, 7177, 7179, 7477, 7486, 7491, 7576,
        7577, 7578, 7579, 7580, 7581, 7583, 7584, 7585, 7586, 7587, 7604, 7610, 7611, 7681,
    ];
    let data = read_data("LineBreakTest-15.0.0.txt");
    let mut cases = 0_usize;
    let mut failures = Vec::new();
    for (line_index, line) in data.lines().enumerate() {
        let Some(case) = parse_boundary_case(line, line_index + 1) else {
            continue;
        };
        let actual: Vec<usize> = linebreaks(&case.text).map(|(offset, _)| offset).collect();
        if actual != case.breaks {
            failures.push((line_index + 1, actual, case.breaks));
        }
        cases += 1;
    }
    let failure_lines: Vec<usize> = failures.iter().map(|failure| failure.0).collect();
    assert_eq!(
        failure_lines,
        KNOWN_DEVIATIONS,
        "line-break conformance changed; first={:?}; last={:?}",
        failures.first(),
        failures.last()
    );
    assert!(cases > 7_000, "expected the complete line-break test data");
}

#[test]
#[ignore = "requires scripts/fetch_unicode_conformance.sh"]
fn bidi_levels_and_visual_order_conform_to_unicode_16() {
    let data = read_data("BidiCharacterTest-16.0.0.txt");
    let mut cases = 0_usize;
    for (line_index, line) in data.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        assert_eq!(fields.len(), 5, "Unicode data line {}", line_index + 1);
        let text: String = fields[0]
            .split_whitespace()
            .map(|value| parse_codepoint(value, line_index + 1))
            .collect();
        let base_level = match fields[1].trim() {
            "0" => Some(LTR_LEVEL),
            "1" => Some(RTL_LEVEL),
            "2" => None,
            value => panic!(
                "invalid paragraph direction {value} on line {}",
                line_index + 1
            ),
        };
        let expected_paragraph_level = fields[2]
            .trim()
            .parse::<u8>()
            .expect("paragraph level is an integer");
        let expected_levels: Vec<&str> = fields[3].split_whitespace().collect();
        let expected_order: Vec<usize> = fields[4]
            .split_whitespace()
            .map(|value| value.parse().expect("visual index is an integer"))
            .collect();

        let bidi = BidiInfo::new(&text, base_level);
        assert_eq!(
            bidi.paragraphs.len(),
            1,
            "Unicode data line {}",
            line_index + 1
        );
        let paragraph = &bidi.paragraphs[0];
        assert_eq!(
            paragraph.level.number(),
            expected_paragraph_level,
            "Unicode data line {}",
            line_index + 1
        );
        let levels = bidi.reordered_levels_per_char(paragraph, paragraph.range.clone());
        assert_eq!(levels.len(), expected_levels.len());
        for (logical_index, (&actual, expected)) in levels.iter().zip(&expected_levels).enumerate()
        {
            if *expected != "x" {
                assert_eq!(
                    actual.number().to_string(),
                    *expected,
                    "Unicode data line {}, logical index {logical_index}",
                    line_index + 1
                );
            }
        }
        let actual_order: Vec<usize> = BidiInfo::reorder_visual(&levels)
            .into_iter()
            .filter(|logical_index| expected_levels[*logical_index] != "x")
            .collect();
        assert_eq!(
            actual_order,
            expected_order,
            "Unicode data line {}",
            line_index + 1
        );
        cases += 1;
    }
    assert!(cases > 90_000, "expected the complete bidi test data");
}

struct BoundaryCase {
    text: String,
    breaks: Vec<usize>,
}

fn parse_boundary_case(line: &str, line_number: usize) -> Option<BoundaryCase> {
    let data = line.split('#').next().unwrap_or_default().trim();
    if data.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut breaks = Vec::new();
    let mut expect_boundary = true;
    for token in data.split_whitespace() {
        if expect_boundary {
            match token {
                "÷" => breaks.push(text.len()),
                "×" => {}
                _ => panic!("invalid boundary marker {token} on line {line_number}"),
            }
        } else {
            text.push(parse_codepoint(token, line_number));
        }
        expect_boundary = !expect_boundary;
    }
    assert!(
        !expect_boundary,
        "missing final boundary on line {line_number}"
    );
    Some(BoundaryCase { text, breaks })
}

fn parse_codepoint(value: &str, line_number: usize) -> char {
    let value = u32::from_str_radix(value, 16)
        .unwrap_or_else(|_| panic!("invalid code point {value} on line {line_number}"));
    char::from_u32(value)
        .unwrap_or_else(|| panic!("non-scalar code point {value:04X} on line {line_number}"))
}

fn read_data(filename: &str) -> String {
    let configured = env::var_os(DATA_DIRECTORY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {DATA_DIRECTORY_ENV} to the downloaded data directory"));
    let directory = if configured.is_absolute() {
        configured
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("text crate is inside the workspace")
            .join(configured)
    };
    fs::read_to_string(directory.join(filename))
        .unwrap_or_else(|error| panic!("failed to read {filename}: {error}"))
}
