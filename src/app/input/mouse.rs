use std::time::Instant;

use super::*;

impl App {
    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Mouse interactions clear transient nav/notification feedback the
        // same way keypresses do (see handle_normal_key) — otherwise stale
        // "at end" / clipboard messages could linger after a click or
        // scroll.
        self.notification = None;
        self.nav_feedback = None;
        // Swallow clicks when a popup is up or the user is typing into
        // an input mode — mouse activity shouldn't yank the selection
        // out from under their text entry.
        let popup_or_input_active = self.input_mode != InputMode::Normal
            || self.show_help
            || self.ast_view_scroll.is_some()
            || self.link_popup_urls.is_some()
            || self.quit_confirm_pending;
        match mouse.kind {
            MouseEventKind::ScrollUp if !popup_or_input_active => self.move_node(-1),
            MouseEventKind::ScrollDown if !popup_or_input_active => self.move_node(1),
            MouseEventKind::Down(MouseButton::Left) if !popup_or_input_active => {
                self.handle_left_click(mouse.row, mouse.column);
            }
            _ => {}
        }
    }

    fn handle_left_click(&mut self, row: u16, col: u16) {
        let count = self.bump_click_count(row, col);
        let Some(anchor) = self.mouse_to_anchor(row, col, count) else {
            // Click outside the list area or on a non-text row: leave
            // the click count alone (the next click on a real cell will
            // start fresh anyway via the cell-change reset above).
            return;
        };
        self.selection_state.anchor = anchor;
        self.refresh_section_highlight(anchor);
        self.status = format!(
            "Node {}/{}  {} {}",
            anchor.node_idx + 1,
            self.view.node_count(),
            anchor.unit.mode_str(),
            anchor.unit_idx + 1,
        );
    }

    /// Increment the click count when the new click is on the same cell
    /// within `CLICK_DOUBLE_INTERVAL` of the previous one; otherwise
    /// reset to 1. Saturates at 3 so a fourth rapid click cycles back
    /// to a single-click selection (matches platform-typical behaviour).
    pub(in crate::app) fn bump_click_count(&mut self, row: u16, col: u16) -> u8 {
        let now = Instant::now();
        let count =
            match self.last_click {
                Some(prev)
                    if prev.row == row
                        && prev.col == col
                        && now.duration_since(prev.at) <= CLICK_DOUBLE_INTERVAL =>
                {
                    if prev.count >= 3 { 1 } else { prev.count + 1 }
                }
                _ => 1,
            };
        self.last_click = Some(LastClick {
            at: now,
            row,
            col,
            count,
        });
        count
    }

    /// Resolve a mouse coordinate to a selection anchor. Returns None for
    /// clicks outside `list_inner`, on spacer rows, or on the
    /// gutter/indicator prefix to the left of the text.
    fn mouse_to_anchor(&self, row: u16, col: u16, click_count: u8) -> Option<SelectionAnchor> {
        self.view.hit_test(self.list_inner, row, col, click_count)
    }
}
