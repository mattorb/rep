//! Pure navigation: `next`, `prev`, `clamp`.
//!
//! Operates against an immutable `SelectionIndex` and a `SelectionAnchor`.
//! Returns `NavOutcome::Boundary` when there is no further anchor of the
//! requested unit; selection state is unchanged on a boundary outcome.
//!
//! All five units (Section / Paragraph / Line / Sentence / Word) traverse via
//! the linear-order tables in `SelectionIndex`; clamp re-anchors via the same
//! tables with backward-then-forward fallback when the target unit is
//! unavailable in the current node.

use std::borrow::Cow;
use std::ops::Range;

use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};

/// Advance the anchor one step forward in its unit's linear-order table.
/// Returns `Boundary` when no further anchor exists; selection state is
/// the caller's responsibility to update on `Moved`.
pub fn next(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome {
    step(index, anchor, true)
}

/// Symmetric to `next` — retreat one step in the active unit's table.
pub fn prev(index: &SelectionIndex, anchor: SelectionAnchor) -> NavOutcome {
    step(index, anchor, false)
}

fn step(index: &SelectionIndex, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
    let table = unit_table(index, anchor.unit);
    if table.is_empty() {
        return NavOutcome::Boundary;
    }
    let current = locate(&table, anchor.node_idx, anchor.unit_idx);
    let next_pos = match (current, forward) {
        (Some(p), true) if p + 1 < table.len() => p + 1,
        (Some(_), true) => return NavOutcome::Boundary,
        (Some(p), false) if p > 0 => p - 1,
        (Some(_), false) => return NavOutcome::Boundary,
        // Anchor is not in the table (e.g. node has zero anchors of this unit)
        // — fall back to the closest table entry in the requested direction.
        (None, true) => match table.iter().position(|&(n, u)| {
            n > anchor.node_idx || (n == anchor.node_idx && u > anchor.unit_idx)
        }) {
            Some(p) => p,
            None => return NavOutcome::Boundary,
        },
        (None, false) => {
            let mut found: Option<usize> = None;
            for (i, &(n, u)) in table.iter().enumerate() {
                if n < anchor.node_idx || (n == anchor.node_idx && u < anchor.unit_idx) {
                    found = Some(i);
                } else {
                    break;
                }
            }
            match found {
                Some(p) => p,
                None => return NavOutcome::Boundary,
            }
        }
    };
    let (n, u) = table[next_pos];
    NavOutcome::Moved(SelectionAnchor::new(n, anchor.unit, u))
}

fn unit_table(index: &SelectionIndex, unit: SelectionUnit) -> Cow<'_, [(usize, usize)]> {
    match unit {
        SelectionUnit::Sentence => Cow::Borrowed(&index.sentences),
        SelectionUnit::Paragraph => Cow::Borrowed(&index.paragraphs),
        SelectionUnit::Line => Cow::Borrowed(&index.lines),
        SelectionUnit::Word => Cow::Borrowed(&index.words),
        SelectionUnit::Section => Cow::Owned(
            index
                .sections
                .iter()
                .map(|s| (s.start_node_idx, 0))
                .collect(),
        ),
    }
}

fn locate(table: &[(usize, usize)], node_idx: usize, unit_idx: usize) -> Option<usize> {
    table
        .iter()
        .position(|&(n, u)| n == node_idx && u == unit_idx)
}

/// Re-anchor onto the requested target unit per the pinned `clamp` rules.
///
/// Same-node:
///   - Upgrade (finer → coarser): land on the target unit anchor that
///     **contains** the current anchor's bytes. Word `fast` in sentence
///     S → clamp(Word→Sentence) = S, not sentence 0.
///   - Downgrade (coarser → finer): land on the **first child** anchor
///     of the target unit whose bytes lie within the source anchor's
///     bytes. Sentence "Dogs run fast." → clamp(Sentence→Word) = "Dogs".
///   - Section containment uses the section table (a section can span
///     multiple nodes).
///
/// Cross-node fallback: if the target unit has no representative inside
/// the source anchor's containing context, walk **backward** then
/// **forward** in document order; on backward land on that node's
/// **last** target-unit anchor, on forward land on its **first**.
/// If neither direction finds anything, the switch is a silent no-op.
pub fn clamp(
    index: &SelectionIndex,
    anchor: SelectionAnchor,
    target: SelectionUnit,
) -> SelectionAnchor {
    if anchor.unit == target {
        return anchor;
    }

    if let Some(found) = clamp_within_node(index, anchor, target) {
        return found;
    }

    let table = unit_table(index, target);
    if table.is_empty() {
        return anchor;
    }
    // Walk backward in document order for a node with the target unit, land
    // on its last anchor in that unit.
    if let Some(&(n, u)) = table
        .iter()
        .rev()
        .find(|&&(n, _)| n < anchor.node_idx)
        .and_then(|prev| {
            let target_node = prev.0;
            table.iter().rfind(|&&(nn, _)| nn == target_node)
        })
    {
        return SelectionAnchor::new(n, target, u);
    }
    // Walk forward; land on the first anchor of the first node with the unit.
    if let Some(&(n, u)) = table.iter().find(|&&(nn, _)| nn > anchor.node_idx) {
        return SelectionAnchor::new(n, target, u);
    }
    anchor
}

/// Resolve clamp within the same node when possible, returning Some on
/// match. Section is special — a section can span multiple nodes — so
/// containment uses the section table rather than a per-node check.
fn clamp_within_node(
    index: &SelectionIndex,
    anchor: SelectionAnchor,
    target: SelectionUnit,
) -> Option<SelectionAnchor> {
    if target == SelectionUnit::Section {
        let section = index
            .sections
            .iter()
            .find(|s| s.start_node_idx <= anchor.node_idx && anchor.node_idx <= s.end_node_idx)?;
        return Some(SelectionAnchor::new(
            section.start_node_idx,
            SelectionUnit::Section,
            0,
        ));
    }
    let node = index.nodes.get(anchor.node_idx)?;
    // The source anchor might not have a real byte range in this node —
    // e.g. a CodeBlock has no Sentence anchors but the cursor can still
    // sit at (CodeBlock, Sentence, 0) when the user cycles into Sentence
    // mode. Fall back to whole-node range so within-node clamp lands on
    // the first available target-unit anchor (matches pre-containment
    // behavior for these phantom-anchor cases).
    let source_range = anchor_byte_range(node, anchor.unit, anchor.unit_idx)
        .unwrap_or(0..node.selection_plain_text.len());

    // Build the candidate (unit_idx, byte_range) list for the target unit
    // restricted to this node.
    let candidates: Vec<(usize, Range<usize>)> = match target {
        SelectionUnit::Word => node
            .word_ranges
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.clone()))
            .collect(),
        SelectionUnit::Sentence => node
            .sentence_ranges
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.clone()))
            .collect(),
        SelectionUnit::Line => node
            .source_line_ranges
            .iter()
            .enumerate()
            .map(|(i, (_, r))| (i, r.clone()))
            .collect(),
        SelectionUnit::Paragraph => {
            // Paragraph is whole-node; presence is gated by the index's
            // `paragraphs` linear table (ThematicBreak / empty-content
            // nodes don't appear there).
            if index.paragraphs.iter().any(|&(n, _)| n == anchor.node_idx) {
                vec![(0, 0..node.selection_plain_text.len())]
            } else {
                Vec::new()
            }
        }
        SelectionUnit::Section => unreachable!("handled above"),
    };
    if candidates.is_empty() {
        return None;
    }

    let upgrade = is_upgrade(anchor.unit, target);
    let pick = if upgrade {
        // Upgrade: target range contains the source range's start byte.
        candidates
            .iter()
            .find(|(_, r)| {
                r.start <= source_range.start && source_range.start < r.end.max(r.start + 1)
            })
            .or_else(|| candidates.first())
    } else {
        // Downgrade: first target whose start byte lies within the source
        // range. Falls back to the first overall if the source is wider
        // than any candidate (shouldn't happen for valid anchors but
        // keeps the function total).
        candidates
            .iter()
            .find(|(_, r)| {
                source_range.start <= r.start && r.start < source_range.end.max(r.start + 1)
            })
            .or_else(|| candidates.first())
    };
    let (unit_idx, _) = pick?;
    Some(SelectionAnchor::new(anchor.node_idx, target, *unit_idx))
}

/// Byte range of an anchor within its node's selection plain text. Used
/// for clamp containment checks. Section returns None — section
/// containment is computed against the section table.
fn anchor_byte_range(
    node: &crate::selection::index::NodeIndex,
    unit: SelectionUnit,
    unit_idx: usize,
) -> Option<Range<usize>> {
    match unit {
        SelectionUnit::Word => node.word_ranges.get(unit_idx).cloned(),
        SelectionUnit::Sentence => node.sentence_ranges.get(unit_idx).cloned(),
        SelectionUnit::Line => node
            .source_line_ranges
            .get(unit_idx)
            .map(|(_, r)| r.clone()),
        SelectionUnit::Paragraph | SelectionUnit::Section => {
            Some(0..node.selection_plain_text.len())
        }
    }
}

/// True when `target` is coarser than `from` — clamp(from→target) is an
/// upgrade. CYCLE_ORDER runs coarse→fine: Section, Paragraph, Line,
/// Sentence, Word.
fn is_upgrade(from: SelectionUnit, target: SelectionUnit) -> bool {
    let order = SelectionUnit::CYCLE_ORDER;
    let pos = |u: SelectionUnit| order.iter().position(|x| *x == u).unwrap_or(0);
    pos(target) < pos(from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::build_test_index as build;

    #[test]
    fn sentence_next_within_node_advances() {
        let idx = build("First sentence. Second sentence. Third.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        match next(&idx, a) {
            NavOutcome::Moved(a2) => {
                assert_eq!(a2.unit_idx, 1);
                assert_eq!(a2.node_idx, 0);
            }
            o => panic!("unexpected: {o:?}"),
        }
    }

    #[test]
    fn sentence_next_at_doc_end_returns_boundary() {
        let idx = build("Only sentence.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        assert_eq!(next(&idx, a), NavOutcome::Boundary);
        assert_eq!(prev(&idx, a), NavOutcome::Boundary);
    }

    #[test]
    fn sentence_next_crosses_node_boundary() {
        let idx = build("First. Second.\n\nThird. Fourth.");
        let last_of_first_node = SelectionAnchor::new(0, SelectionUnit::Sentence, 1);
        match next(&idx, last_of_first_node) {
            NavOutcome::Moved(a) => {
                assert_eq!(a.node_idx, 1);
                assert_eq!(a.unit_idx, 0);
            }
            o => panic!("unexpected: {o:?}"),
        }
    }

    #[test]
    fn roundtrip_invariant() {
        let idx = build("A.\n\nB.\n\nC.");
        let mut a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        for _ in 0..3 {
            if let NavOutcome::Moved(b) = next(&idx, a) {
                if let NavOutcome::Moved(c) = prev(&idx, b) {
                    assert_eq!(a, c);
                    a = b;
                }
            }
        }
    }

    #[test]
    fn clamp_word_to_sentence_lands_on_containing_sentence() {
        // Per modular_plan §"Unit-switch rules": clamp(Word→Sentence)
        // returns the sentence that CONTAINS the word, not sentence 0.
        // Pre-containment shape was wrong: it picked the first sentence
        // anchor in the node regardless of where the word lived.
        let idx = build("First sentence here. Second sentence here. Third here.");
        // Find a word anchor inside the second sentence.
        let s2 = idx.nodes[0].sentence_ranges[1].clone();
        let (w_idx, _) = idx.nodes[0]
            .word_ranges
            .iter()
            .enumerate()
            .find(|(_, r)| r.start >= s2.start && r.end <= s2.end)
            .expect("at least one word in second sentence");
        let word_anchor = SelectionAnchor::new(0, SelectionUnit::Word, w_idx);
        let result = clamp(&idx, word_anchor, SelectionUnit::Sentence);
        assert_eq!(
            result.unit_idx, 1,
            "expected sentence index 1 (containing), got {}",
            result.unit_idx
        );
    }

    #[test]
    fn clamp_sentence_to_word_lands_on_first_word_in_sentence() {
        // Downgrade Sentence→Word lands on the first WORD of the
        // current sentence, not the first word of the node.
        let idx = build("First sentence here. Second sentence here.");
        let s2_anchor = SelectionAnchor::new(0, SelectionUnit::Sentence, 1);
        let result = clamp(&idx, s2_anchor, SelectionUnit::Word);
        assert_eq!(result.unit, SelectionUnit::Word);
        let plain = &idx.nodes[0].selection_plain_text;
        let word_range = idx.nodes[0].word_ranges[result.unit_idx].clone();
        assert_eq!(
            &plain[word_range], "Second",
            "expected first word of second sentence, got idx {}",
            result.unit_idx
        );
    }

    #[test]
    fn clamp_section_picks_containing_section_not_starting() {
        // A section spans multiple nodes. Clamping to Section from a
        // body-paragraph anchor should land on the section's
        // start_node_idx, not require the cursor to already be on a
        // heading.
        let idx = build("# A\n\nbody paragraph one.\n\n# B\n\nbody two.");
        // Sentence anchor in section A's body paragraph (node 1, sentence 0).
        let body_anchor = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
        let result = clamp(&idx, body_anchor, SelectionUnit::Section);
        assert_eq!(result.unit, SelectionUnit::Section);
        assert_eq!(
            result.node_idx, 0,
            "section A starts at node 0; got node_idx {}",
            result.node_idx
        );
    }

    #[test]
    fn section_nav_walks_headings() {
        let idx = build("# A\n\nx\n\n# B\n\ny.");
        // Sections at node 0 and node 2.
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        let n = next(&idx, a);
        match n {
            NavOutcome::Moved(b) => assert_eq!(b.node_idx, 2),
            o => panic!("unexpected: {o:?}"),
        }
    }

    #[test]
    fn section_nav_no_sections_is_boundary() {
        let idx = build("Plain prose.");
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        assert_eq!(next(&idx, a), NavOutcome::Boundary);
    }

    #[test]
    fn clamp_to_word_lands_on_first_word() {
        let idx = build("Plain prose.");
        let a = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        let b = clamp(&idx, a, SelectionUnit::Word);
        assert_eq!(b.unit, SelectionUnit::Word);
        assert_eq!(b.unit_idx, 0);
    }

    #[test]
    fn clamp_to_unavailable_unit_is_noop() {
        // Code-only document: no sentence anchors and no word anchors
        // (sentence-level skip rule). clamp(Section -> Sentence) finds no
        // entries and returns the original anchor.
        let idx = build("```\nfn x() {}\n```");
        let a = SelectionAnchor::new(0, SelectionUnit::Section, 0);
        let b = clamp(&idx, a, SelectionUnit::Sentence);
        assert_eq!(a, b);
    }

    #[test]
    fn clamp_matrix_full_doc_covers_every_unit_pair() {
        // Document with all five units present: heading, multi-sentence
        // paragraph, multi-line paragraph. Verify clamp lands on a valid
        // anchor of the requested unit for every (from, to) pair.
        let idx = build(
            "# Top heading\n\nFirst sentence here. Second sentence here.\n\nA second paragraph\nwrapping two lines.",
        );
        let units = [
            SelectionUnit::Section,
            SelectionUnit::Paragraph,
            SelectionUnit::Line,
            SelectionUnit::Sentence,
            SelectionUnit::Word,
        ];
        // Start anchor: paragraph 1, sentence 0.
        let start = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
        for &from in &units {
            // Move to `from` first.
            let a = clamp(&idx, start, from);
            assert_eq!(a.unit, from, "clamp should land on {from:?}");
            for &to in &units {
                let b = clamp(&idx, a, to);
                assert_eq!(b.unit, to, "clamp({from:?} -> {to:?}) failed");
            }
        }
    }
}
