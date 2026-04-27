//! Anchor → highlight projection. Phase 3a/4 lands the implementation.

#![allow(unused)]

use std::ops::Range;

use crate::selection::index::SelectionIndex;
use crate::selection::model::SelectionAnchor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Highlight {
    /// (node_idx, byte range in the node's selection plain text)
    Range(usize, Range<usize>),
    /// Section span — list of node indices to paint.
    Section(Vec<usize>),
}
