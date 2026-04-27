//! Canonical home for the sentence and word segmenters per Req 11.
//!
//! `segment_sentences` is the canonical sentence segmenter. It operates on
//! whatever string it is given (selection plain text or display plain text);
//! the caller decides which view they need. Callers must not duplicate the
//! segmentation logic — every reader of sentence ranges goes through this
//! function.
//!
//! `segment_words` follows the pinned word-boundary rules and is the
//! canonical word segmenter.
//!
//! Selection plain text per node is built by
//! `selection::index::node_selection_plain_text` during index construction
//! (`SelectionIndex::build`); the resulting string is then read directly off
//! `NodeIndex::selection_plain_text` for any per-node lookup.

use std::ops::Range;

/// Sentence byte ranges within a plain-text input.
///
/// Splits on `\n` (source line break that is **not** a hard-wrap
/// continuation) and on `. ` / `! ` / `? ` followed by an ASCII uppercase
/// letter. Returned ranges are byte offsets into `plain`, sorted,
/// non-overlapping, and with leading/trailing whitespace trimmed.
pub fn segment_sentences(plain: &str) -> Vec<Range<usize>> {
    if plain.trim().is_empty() {
        return Vec::new();
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let bytes = plain.as_bytes();
    let len = bytes.len();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < len {
        if bytes[i] == b'\n' {
            // Look past the newline and any leading spaces to the first
            // content char. A lowercase letter signals a hard-wrap
            // continuation of the current sentence — don't split here.
            let mut j = i + 1;
            while j < len && bytes[j] == b' ' {
                j += 1;
            }
            if j < len && bytes[j].is_ascii_lowercase() {
                i += 1;
                continue;
            }
            push_trimmed_range(&mut ranges, plain, start, i);
            i += 1;
            start = i;
            continue;
        }
        if matches!(bytes[i], b'.' | b'!' | b'?')
            && i + 2 < len
            && bytes[i + 1] == b' '
            && bytes[i + 2].is_ascii_uppercase()
        {
            push_trimmed_range(&mut ranges, plain, start, i + 1);
            i += 2;
            start = i;
            continue;
        }
        i += 1;
    }
    push_trimmed_range(&mut ranges, plain, start, len);
    if ranges.is_empty() {
        ranges.push(0..len);
    }
    ranges
}

fn push_trimmed_range(ranges: &mut Vec<Range<usize>>, plain: &str, start: usize, end: usize) {
    if start >= end {
        return;
    }
    let slice = &plain[start..end];
    if slice.trim().is_empty() {
        return;
    }
    let leading = slice.len() - slice.trim_start().len();
    let trailing = slice.len() - slice.trim_end().len();
    ranges.push((start + leading)..(end - trailing));
}

/// Word byte ranges within selection plain text per the pinned
/// word-boundary rules.
///
/// - Word chars = `\p{Alphabetic}` ∪ ASCII digits. Underscore is NOT a word
///   character.
/// - Internal punctuation that preserves a word: `.` `'` `-` between two
///   word chars; `,` between two digits.
/// - Em-dash (`—`), en-dash (`–`), ellipsis (`…`) are always boundaries.
/// - Leading and trailing punctuation at a word boundary is stripped.
/// - Numbers with internal punctuation are one word: `3.14`, `1,000`,
///   `2026-04-24`.
pub fn segment_words(plain: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let chars: Vec<(usize, char)> = plain.char_indices().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if !is_word_char(chars[i].1) {
            i += 1;
            continue;
        }
        let start_byte = chars[i].0;
        let mut end_byte = chars[i].0 + chars[i].1.len_utf8();
        let mut j = i + 1;
        while j < n {
            let ch = chars[j].1;
            if is_word_char(ch) {
                end_byte = chars[j].0 + ch.len_utf8();
                j += 1;
                continue;
            }
            // Internal punctuation: keep going if next char is also a word
            // char per the per-punct rules.
            let prev = chars[j - 1].1;
            let next = chars.get(j + 1).map(|(_, c)| *c);
            if let Some(nc) = next {
                if is_internal_continuation(prev, ch, nc) {
                    j += 1;
                    continue;
                }
            }
            break;
        }
        ranges.push(start_byte..end_byte);
        i = j;
    }
    ranges
}

fn is_word_char(ch: char) -> bool {
    if ch == '_' {
        return false;
    }
    ch.is_alphabetic() || ch.is_ascii_digit()
}

fn is_internal_continuation(prev: char, sep: char, next: char) -> bool {
    if !is_word_char(prev) || !is_word_char(next) {
        return false;
    }
    match sep {
        '.' | '\'' => true,
        '-' => {
            (prev.is_alphabetic() && next.is_alphabetic())
                || (prev.is_ascii_digit() && next.is_ascii_digit())
        }
        ',' => prev.is_ascii_digit() && next.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty() {
        assert!(segment_sentences("").is_empty());
        assert!(segment_sentences("   ").is_empty());
    }

    #[test]
    fn single_segment_when_no_terminator() {
        let r = segment_sentences("just words no period");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], 0..20);
    }

    #[test]
    fn period_space_uppercase_splits() {
        let r = segment_sentences("First sentence. Second sentence.");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], 0..15);
    }

    #[test]
    fn newline_splits_unless_lowercase_continuation() {
        let r = segment_sentences("First line\ncontinuation here");
        assert_eq!(r.len(), 1);
        let r = segment_sentences("First line\nSecond line");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn newline_indented_lowercase_continuation_does_not_split() {
        // Soft-wrap continuation indented with spaces still folds into
        // the prior sentence — the whitespace skip ahead of the lowercase
        // check looks past leading spaces.
        let r = segment_sentences("Some text\n  continued here");
        assert_eq!(r.len(), 1, "{r:?}");
    }

    #[test]
    fn newline_indented_uppercase_splits() {
        // The uppercase character after the leading-space skip still
        // counts as a sentence boundary.
        let r = segment_sentences("Some text\n  Capitalized here");
        assert_eq!(r.len(), 2, "{r:?}");
    }

    #[test]
    fn exclamation_and_question_mark_split_like_period() {
        let r = segment_sentences("Done! Already? Sure.");
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn ranges_are_sorted_non_overlapping_in_bounds() {
        let s = "First. Second one. Third.";
        let r = segment_sentences(s);
        assert!(r.iter().all(|range| range.end <= s.len()));
        for w in r.windows(2) {
            assert!(w[0].end <= w[1].start, "overlap: {w:?}");
        }
    }

    fn words_of(s: &str) -> Vec<&str> {
        segment_words(s).into_iter().map(|r| &s[r]).collect()
    }

    #[test]
    fn segment_words_empty_input_yields_empty() {
        assert!(segment_words("").is_empty());
        assert!(segment_words("   ").is_empty());
        assert!(segment_words(",,,!!").is_empty());
    }

    #[test]
    fn word_basic_punct() {
        assert_eq!(words_of("word, word."), vec!["word", "word"]);
    }

    #[test]
    fn word_contractions_keep_apostrophe() {
        assert_eq!(
            words_of("don't won't can't"),
            vec!["don't", "won't", "can't"]
        );
    }

    #[test]
    fn word_leading_and_trailing_apostrophe_stripped() {
        assert_eq!(words_of("'tis users'"), vec!["tis", "users"]);
    }

    #[test]
    fn word_hyphenated_compound_is_one_word() {
        assert_eq!(
            words_of("state-of-the-art word-level"),
            vec!["state-of-the-art", "word-level"]
        );
    }

    #[test]
    fn word_underscore_is_a_boundary() {
        assert_eq!(words_of("foo_bar"), vec!["foo", "bar"]);
    }

    #[test]
    fn word_internal_periods_kept() {
        assert_eq!(words_of("U.S.A and e.g."), vec!["U.S.A", "and", "e.g"]);
    }

    #[test]
    fn word_em_dash_and_ellipsis_boundary() {
        assert_eq!(
            words_of("foo—bar baz…qux"),
            vec!["foo", "bar", "baz", "qux"]
        );
    }

    #[test]
    fn word_numbers_with_internal_punct() {
        assert_eq!(
            words_of("3.14 1,000 2026-04-24"),
            vec!["3.14", "1,000", "2026-04-24"]
        );
    }

    #[test]
    fn word_unicode_alphabetic() {
        assert_eq!(
            words_of("café naïve 日本語"),
            vec!["café", "naïve", "日本語"]
        );
    }
}
