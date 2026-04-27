//! Single canonical home for `plain_text_for_node` and the sentence/word
//! segmenters per Req 11.
//!
//! `segment_sentences` is the canonical sentence segmenter. It operates on
//! whatever string it is given (selection plain text or display plain text);
//! the caller decides which view they need. Callers must not duplicate the
//! segmentation logic — every reader of sentence ranges goes through this
//! function.
//!
//! `segment_words` is reserved for phase 5.

use std::ops::Range;

use crate::document::DocNode;

/// Selection plain text for a single node, with markdown markers stripped per
/// the pinned visibility rules (footnote refs, task markers, image wrappers,
/// code-block fences). Phase 1's implementation is the conservative one in
/// `selection::index`; phase 2 keeps that for now and expands the rules in
/// later phases.
pub fn plain_text_for_node(node: &DocNode, source_lines: &[String]) -> String {
    crate::selection::index::node_selection_plain_text(node, source_lines)
}

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

/// Word byte ranges within selection plain text. Phase 5 lands this.
pub fn segment_words(_plain: &str) -> Vec<Range<usize>> {
    Vec::new()
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
    fn ranges_are_sorted_non_overlapping_in_bounds() {
        let s = "First. Second one. Third.";
        let r = segment_sentences(s);
        assert!(r.iter().all(|range| range.end <= s.len()));
        for w in r.windows(2) {
            assert!(w[0].end <= w[1].start, "overlap: {:?}", w);
        }
    }
}
