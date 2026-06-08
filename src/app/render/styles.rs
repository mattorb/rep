use super::*;

impl App {
    pub(super) fn node_indicator(&self, node_idx: usize) -> (&'static str, Style) {
        let change_count = self.changes.get(&node_idx).map_or(0, |v| v.len());
        let feedback_count = self.feedbacks.get(&node_idx).map_or(0, |v| v.len());
        let insert_count = self.inserts_before.get(&node_idx).map_or(0, |v| v.len())
            + self.inserts_after.get(&node_idx).map_or(0, |v| v.len());
        let strike_count = self.strikes.get(&node_idx).map_or(0, |v| v.len());

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

    pub(in crate::app) fn render_node_spans(&self, node_idx: usize) -> Vec<Span<'static>> {
        let strike_units: Vec<(SelectionUnit, usize)> = self
            .strikes
            .get(&node_idx)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default();
        let section_highlight_active = self
            .section_highlight_range
            .as_ref()
            .is_some_and(|range| range.contains(&node_idx));

        self.view
            .styled_display_spans(DisplaySpanStyleRequest {
                node_idx,
                active_anchor: self.selection_state.anchor,
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
