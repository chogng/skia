use super::*;

fn offsets(text: &str) -> Vec<usize> {
    linebreaks(text).map(|(offset, _)| offset).collect()
}

#[test]
fn regex_number_tailoring_only_protects_complete_numeric_expressions() {
    assert_eq!(offsets("a.2 "), vec![2, 4]);
    assert_eq!(offsets("$(12.35)%"), vec![9]);
    assert_eq!(offsets("}%"), vec![1, 2]);
    assert_eq!(offsets(",0"), vec![1, 2]);
    assert_eq!(offsets("/0"), vec![1, 2]);
    assert_eq!(offsets("1/0"), vec![3]);
}

#[test]
fn wide_opening_punctuation_and_potential_emoji_follow_unicode_rules() {
    assert_eq!(offsets("a（字"), vec![1, 7]);
    assert_eq!(offsets("\u{1F02C}\u{1F3FF}"), vec![8]);
}
