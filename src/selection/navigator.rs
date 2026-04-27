//! Pure navigation logic. Phase 3a moves the implementation here. For now this
//! is a stub.

#![allow(unused)]

use crate::selection::index::SelectionIndex;
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};

pub fn next(_index: &SelectionIndex, _anchor: SelectionAnchor) -> NavOutcome {
    NavOutcome::Boundary
}

pub fn prev(_index: &SelectionIndex, _anchor: SelectionAnchor) -> NavOutcome {
    NavOutcome::Boundary
}

pub fn clamp(
    _index: &SelectionIndex,
    anchor: SelectionAnchor,
    _target: SelectionUnit,
) -> SelectionAnchor {
    anchor
}
