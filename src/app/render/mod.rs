use super::*;

mod document;
mod footer;
mod popups;
mod styles;

impl App {
    // ── Drawing ───────────────────────────────────────────────────────────────

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(FOOTER_HEIGHT)])
            .split(area);

        let list_inner = self.draw_document(frame, layout[0]);
        self.draw_footer(frame, layout[1]);
        self.draw_active_input_popup(frame, list_inner);

        if self.link_popup_urls.is_some() {
            self.draw_link_popup(frame, area);
        }

        if self.show_help {
            Self::draw_help(frame, area);
        }

        if self.ast_view_scroll.is_some() {
            self.draw_ast_popup(frame, area);
        }
    }
}
