use std::collections::{BTreeMap, BTreeSet};

use crate::selection::model::SelectionUnit;

#[derive(Debug, Clone)]
pub(crate) struct ChangeAnnotation {
    pub(crate) created_at: String,
    pub(crate) target_unit: SelectionUnit,
    pub(crate) sentence_index: Option<usize>,
    pub(crate) sentence_text: Option<String>,
    pub(crate) change: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FeedbackAnnotation {
    pub(crate) created_at: String,
    pub(crate) target_unit: SelectionUnit,
    pub(crate) sentence_index: Option<usize>,
    pub(crate) sentence_text: Option<String>,
    pub(crate) feedback: String,
}

#[derive(Debug, Clone)]
pub(crate) struct InsertAnnotation {
    pub(crate) created_at: String,
    pub(crate) target_unit: SelectionUnit,
    pub(crate) sentence_index: Option<usize>,
    pub(crate) sentence_text: Option<String>,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditableAnnotation {
    Change(usize),
    Feedback(usize),
}

/// Canonical annotation storage. BTree containers preserve deterministic
/// document/unit ordering for both frontends and emitted action lists.
#[derive(Debug, Clone, Default)]
pub(crate) struct AnnotationStore {
    pub(crate) changes: BTreeMap<usize, Vec<ChangeAnnotation>>,
    pub(crate) feedbacks: BTreeMap<usize, Vec<FeedbackAnnotation>>,
    pub(crate) inserts_before: BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(crate) inserts_after: BTreeMap<usize, Vec<InsertAnnotation>>,
    pub(crate) strikes: BTreeMap<usize, BTreeSet<(SelectionUnit, usize)>>,
    pub(crate) strike_created_at: BTreeMap<(usize, SelectionUnit, usize), String>,
}

impl AnnotationStore {
    pub(crate) fn has_annotation(&self, node_idx: usize) -> bool {
        self.changes.contains_key(&node_idx)
            || self.feedbacks.contains_key(&node_idx)
            || self.inserts_before.contains_key(&node_idx)
            || self.inserts_after.contains_key(&node_idx)
            || self.strikes.contains_key(&node_idx)
    }

    pub(crate) fn touched_nodes(&self) -> BTreeSet<usize> {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touched_nodes_are_unique_and_in_document_order() {
        let mut store = AnnotationStore::default();
        store.changes.insert(4, Vec::new());
        store.feedbacks.insert(1, Vec::new());
        store.strikes.insert(4, BTreeSet::new());

        assert_eq!(
            store.touched_nodes().into_iter().collect::<Vec<_>>(),
            [1, 4]
        );
        assert!(store.has_annotation(1));
        assert!(store.has_annotation(4));
        assert!(!store.has_annotation(2));
    }
}
