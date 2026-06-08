use super::*;

impl App {
    pub(super) fn run_search(&mut self, query: &str, forward: bool) {
        let matches = self.view.search_matches(query);
        if matches.is_empty() {
            self.status = format!("No matches for \"{query}\".");
            return;
        }
        let current = self.search_current_position();
        let target_idx = if forward {
            matches.iter().position(|m| *m >= current).unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|m| *m <= current)
                .unwrap_or(matches.len() - 1)
        };
        self.apply_search_target(query, &matches, target_idx);
    }

    pub(super) fn jump_search(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.status = "No previous search. Press / to search.".to_string();
            return;
        };
        let matches = self.view.search_matches(&query);
        if matches.is_empty() {
            self.status = format!("No matches for \"{query}\".");
            return;
        }
        let current = self.search_current_position();
        let target_idx = if forward {
            matches.iter().position(|m| *m > current).unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|m| *m < current)
                .unwrap_or(matches.len() - 1)
        };
        self.apply_search_target(&query, &matches, target_idx);
    }

    /// `(node_idx, sentence_idx)` cursor position used to seed forward /
    /// backward search match lookup. In Sentence mode this is the literal
    /// anchor; in any other mode the unit_idx is not a sentence index, so
    /// we treat the cursor as positioned at sentence 0 of the current node
    /// — forward search finds the first match in or after this node, and
    /// backward search finds the last match strictly before this node.
    fn search_current_position(&self) -> (usize, usize) {
        let n = self.selection_state.anchor.node_idx;
        if self.selection_state.anchor.unit == SelectionUnit::Sentence {
            (n, self.selection_state.anchor.unit_idx)
        } else {
            (n, 0)
        }
    }

    fn apply_search_target(&mut self, query: &str, matches: &[(usize, usize)], target_idx: usize) {
        let (ni, si) = matches[target_idx];
        // search_matches returns sentence-keyed positions, so the
        // search jump always anchors at sentence granularity regardless of
        // the active unit at search time.
        self.selection_state.anchor = SelectionAnchor::new(ni, SelectionUnit::Sentence, si);
        self.clamp_sentence();
        self.status = format!(
            "Match {}/{} for \"{}\".",
            target_idx + 1,
            matches.len(),
            query
        );
    }

    pub(in crate::app) fn move_node(&mut self, delta: isize) {
        if self.view.node_count() == 0 || delta == 0 {
            return;
        }
        let steps = delta.unsigned_abs();
        let forward = delta.is_positive();
        let mut target = self.selection_state.anchor.node_idx;
        let mut moved = 0usize;

        for _ in 0..steps {
            let next = if forward {
                self.view.next_content_node(target.saturating_add(1))
            } else {
                self.view.prev_content_node(target)
            };
            let Some(idx) = next else { break };
            target = idx;
            moved += 1;
        }

        if moved == 0 {
            self.status = if forward {
                "Already at the last node.".to_string()
            } else {
                "Already at the first node.".to_string()
            };
            return;
        }
        self.selection_state.anchor.node_idx = target;
        self.clamp_sentence();
        self.status = format!(
            "Node {}/{}",
            self.selection_state.anchor.node_idx + 1,
            self.view.node_count()
        );
    }

    /// j / k / Down / Up / Right / Left — move by the currently active
    /// selection unit. Pure delegate to `selection::navigator::next/prev`.
    /// On `Boundary`, set `nav_feedback` ("at end" / "at start") for one
    /// keypress in the right zone of the footer.
    pub(super) fn move_active_unit(&mut self, forward: bool) {
        if self.view.node_count() == 0 {
            return;
        }
        let outcome = self.view.navigate(self.selection_state.anchor, forward);
        match outcome {
            crate::selection::model::NavOutcome::Moved(a) => {
                self.selection_state.anchor = a;
                self.refresh_section_highlight(a);
            }
            crate::selection::model::NavOutcome::Boundary => {
                // Zero-anchor units (e.g., section nav on a doc with no
                // headings): silent no-op per modular_plan §"Empty /
                // degenerate documents". Otherwise show "at end" / "at
                // start" feedback in the right zone. Selection state
                // (including section_highlight_range) stays put — the
                // user is still on the boundary section.
                if !self.view.has_any_anchor(self.selection_state.anchor.unit) {
                    return;
                }
                self.nav_feedback = Some(if forward { "at end" } else { "at start" }.to_string());
            }
        }
    }

    /// Space (forward) / Backspace (reverse) — cycle the active selection
    /// unit. Re-anchors via `navigator::clamp` per the pinned rules.
    pub(super) fn mode_cycle(&mut self, forward: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let i = order
            .iter()
            .position(|u| *u == self.selection_state.anchor.unit)
            .unwrap_or(0);
        let next_i = if forward {
            (i + 1) % order.len()
        } else {
            (i + order.len() - 1) % order.len()
        };
        let target = order[next_i];
        let new_anchor = self.view.clamp_anchor(self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// i (finer) / o (coarser) — adjust the active selection unit by
    /// one step, stopping at the ends instead of wrapping around.
    pub(super) fn mode_adjust(&mut self, finer: bool) {
        let order = SelectionUnit::CYCLE_ORDER;
        let i = order
            .iter()
            .position(|u| *u == self.selection_state.anchor.unit)
            .unwrap_or(0);
        let target = if finer {
            order.get(i + 1).copied()
        } else if i > 0 {
            order.get(i - 1).copied()
        } else {
            None
        };
        let Some(target) = target else {
            return;
        };
        let new_anchor = self.view.clamp_anchor(self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// Refresh `section_highlight_range` for an anchor: set the span when
    /// the active unit is Section, clear otherwise. Used after every move
    /// or mode-cycle that lands on a new anchor.
    pub(super) fn refresh_section_highlight(&mut self, anchor: SelectionAnchor) {
        if anchor.unit == SelectionUnit::Section {
            self.section_highlight_range = Some(self.view.section_span_for_start(anchor.node_idx));
        } else {
            self.section_highlight_range = None;
        }
    }

    /// Stable string for the mode indicator in the left zone of the footer.
    pub(in crate::app) const fn mode_indicator(&self) -> &'static str {
        self.selection_state.anchor.unit.mode_str()
    }

    pub(super) fn jump_to_annotation(&mut self, forward: bool) {
        let from = if forward {
            self.selection_state.anchor.node_idx + 1
        } else {
            self.selection_state.anchor.node_idx
        };
        let n = self.view.node_count();
        let target = if forward {
            (from..n).find(|&i| self.has_annotation(i))
        } else {
            (0..from).rev().find(|&i| self.has_annotation(i))
        };

        match target {
            Some(idx) => {
                self.selection_state.anchor.node_idx = idx;
                self.clamp_sentence();
                self.status = format!("Annotated node {}.", idx + 1);
            }
            None => {
                self.status = if forward {
                    "No annotated nodes after this one.".to_string()
                } else {
                    "No annotated nodes before this one.".to_string()
                };
            }
        }
    }
    fn has_annotation(&self, node_idx: usize) -> bool {
        self.changes.contains_key(&node_idx)
            || self.feedbacks.contains_key(&node_idx)
            || self.inserts_before.contains_key(&node_idx)
            || self.inserts_after.contains_key(&node_idx)
            || self.strikes.contains_key(&node_idx)
    }

    /// Snap the anchor to a valid Sentence position on the current node.
    ///
    /// Used by every "go to a different node" path (mouse click, mouse
    /// scroll, jump_to_annotation, search). Resets the active unit to
    /// Sentence so the unit_idx is interpreted consistently — without this
    /// reset, jumping to a different node while in Word/Line/Paragraph mode
    /// would leave a Sentence unit_idx in a slot the unit ought to read as
    /// a word/line/paragraph index.
    pub(super) fn clamp_sentence(&mut self) {
        let total = self
            .view
            .sentence_count_for_node(self.selection_state.anchor.node_idx);
        let unit_idx = if total == 0 {
            0
        } else {
            self.selection_state.anchor.unit_idx.min(total - 1)
        };
        let new_anchor = SelectionAnchor::new(
            self.selection_state.anchor.node_idx,
            SelectionUnit::Sentence,
            unit_idx,
        );
        self.selection_state.anchor = new_anchor;
        // Forced unit change (always to Sentence) — clear any stale section
        // highlight from a prior Section-mode anchor; without this,
        // jump_to_annotation / mouse-click / search-jump from Section mode
        // would leave the section span painted.
        self.refresh_section_highlight(new_anchor);
    }
}
