use super::*;

#[test]
fn sentence_segmentation_is_a_total_function_on_plain_text() {
    let cases = [
        "",
        "  ",
        "Single line",
        "First. Second.",
        "First sentence here. Second sentence here.",
        "Line one\nline two",
        "Line one\nLine two",
        "U.S.A is one acronym. Another sentence.",
    ];
    for s in cases {
        let r = segment_sentences(s);
        for range in &r {
            assert!(range.end <= s.len(), "out of bounds in {s:?}");
            assert!(range.start <= range.end, "inverted range in {s:?}");
        }
        for w in r.windows(2) {
            assert!(w[0].end <= w[1].start, "overlap in {s:?}");
        }
    }
}

#[test]
fn word_segmentation_round_trips_per_unit_text() {
    let s = "state-of-the-art foo_bar 2026-04-24 don't U.S.A";
    let words: Vec<&str> = segment_words(s).into_iter().map(|r| &s[r]).collect();
    assert_eq!(
        words,
        vec![
            "state-of-the-art",
            "foo",
            "bar",
            "2026-04-24",
            "don't",
            "U.S.A"
        ]
    );
}
