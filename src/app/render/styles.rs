use super::*;

impl App {
    pub(super) fn node_indicator(
        state: &RenderState<'_>,
        node_idx: usize,
    ) -> (&'static str, Style) {
        let change_count = state.changes.get(&node_idx).map_or(0, |v| v.len());
        let feedback_count = state.feedbacks.get(&node_idx).map_or(0, |v| v.len());
        let insert_count = state.inserts_before.get(&node_idx).map_or(0, |v| v.len())
            + state.inserts_after.get(&node_idx).map_or(0, |v| v.len());
        let strike_count = state.strikes.get(&node_idx).map_or(0, |v| v.len());

        let has_change = change_count > 0;
        let has_feedback = feedback_count > 0;
        let has_insert = insert_count > 0;
        let has_strike = strike_count > 0;

        let total = change_count + feedback_count + insert_count + strike_count;
        if total > 1 {
            return (
                "*",
                Style::default()
                    .fg(Color::LightMagenta)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_change {
            return (
                "C",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_feedback {
            return (
                "F",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_insert {
            return (
                "+",
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            );
        }
        if has_strike {
            return ("X", Style::default().fg(Color::LightRed));
        }
        (" ", Style::default().fg(Color::DarkGray))
    }

    #[cfg(test)]
    pub(in crate::app) fn render_node_spans(&self, node_idx: usize) -> Vec<Span<'static>> {
        let state = self.render_state();
        Self::render_node_spans_for(&state, node_idx)
    }

    pub(super) fn render_node_spans_for(
        state: &RenderState<'_>,
        node_idx: usize,
    ) -> Vec<Span<'static>> {
        let strike_units: Vec<(SelectionUnit, usize)> = state
            .strikes
            .get(&node_idx)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        let section_highlight_active = state
            .section_highlight_range
            .as_ref()
            .is_some_and(|range| range.contains(&node_idx));

        state
            .view
            .styled_display_spans(DisplaySpanStyleRequest {
                node_idx,
                active_anchor: state.selection_state.anchor,
                section_highlight_active,
                strike_units: &strike_units,
            })
            .unwrap_or_else(|| {
                vec![Span::styled(
                    " ",
                    Style::default().add_modifier(Modifier::DIM),
                )]
            })
    }
}
