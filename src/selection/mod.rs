//! Selection / navigation domain.
//!
//! See `modular_plan.md` and `implementation.md` for the architectural
//! contract. This module is the canonical home for selection state, the
//! per-node selection-plain-text view, the eager selection index, the pure
//! navigator, and anchor-to-highlight projection.

pub mod index;
pub mod model;
pub mod navigator;
#[cfg(test)]
pub mod projection;
pub mod segment;

/// Test-only helper: parse a markdown source string into a `SelectionIndex`
/// the same way `App::load` does.
///
/// Used by `index` / `navigator` / `projection` test modules so each doesn't
/// carry its own copy.
#[cfg(test)]
pub(crate) fn build_test_index(src: &str) -> index::SelectionIndex {
    let lines: Vec<String> = src.lines().map(ToOwned::to_owned).collect();
    let doc = crate::document::Document::parse(src).unwrap();
    index::SelectionIndex::build(&doc, &lines)
}
