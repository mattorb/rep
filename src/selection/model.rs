//! Selection model types: `SelectionUnit`, `SelectionAnchor`, `SelectionState`,
//! `NavOutcome`.
//!
//! Phase-1 will land the canonical `(node_idx, unit, unit_idx)` anchor; this
//! file is the stub.

#![allow(unused)]

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionUnit {
    Section,
    Paragraph,
    Line,
    Sentence,
    Word,
}

impl SelectionUnit {
    /// Stable string identifier used in golden artifacts and the status line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Section => "Section",
            Self::Paragraph => "Paragraph",
            Self::Line => "Line",
            Self::Sentence => "Sentence",
            Self::Word => "Word",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectionAnchor {
    pub node_idx: usize,
    pub unit: SelectionUnit,
    pub unit_idx: usize,
}

impl SelectionAnchor {
    pub fn new(node_idx: usize, unit: SelectionUnit, unit_idx: usize) -> Self {
        let unit_idx = match unit {
            SelectionUnit::Paragraph | SelectionUnit::Section => 0,
            _ => unit_idx,
        };
        Self {
            node_idx,
            unit,
            unit_idx,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionState {
    pub anchor: SelectionAnchor,
}

impl SelectionState {
    pub fn new(anchor: SelectionAnchor) -> Self {
        Self { anchor }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavOutcome {
    Moved(SelectionAnchor),
    Boundary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_and_section_anchors_zero_unit_idx() {
        let p = SelectionAnchor::new(3, SelectionUnit::Paragraph, 7);
        let s = SelectionAnchor::new(0, SelectionUnit::Section, 4);
        assert_eq!(p.unit_idx, 0);
        assert_eq!(s.unit_idx, 0);
    }

    #[test]
    fn finer_units_keep_unit_idx() {
        let line = SelectionAnchor::new(2, SelectionUnit::Line, 5);
        let sentence = SelectionAnchor::new(2, SelectionUnit::Sentence, 1);
        let word = SelectionAnchor::new(2, SelectionUnit::Word, 9);
        assert_eq!(line.unit_idx, 5);
        assert_eq!(sentence.unit_idx, 1);
        assert_eq!(word.unit_idx, 9);
    }

    #[test]
    fn anchors_compare_by_components() {
        let a = SelectionAnchor::new(2, SelectionUnit::Sentence, 1);
        let b = SelectionAnchor::new(2, SelectionUnit::Sentence, 1);
        assert_eq!(a, b);
        let c = SelectionAnchor::new(2, SelectionUnit::Word, 1);
        assert_ne!(a, c);
    }
}
