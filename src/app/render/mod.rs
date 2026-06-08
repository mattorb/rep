use super::*;

mod document;
mod state;

pub(crate) use state::RenderState;

impl App {
    // ── Drawing ───────────────────────────────────────────────────────────────

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(FOOTER_HEIGHT)])
            .split(area);

        let list_inner = self.draw_document(frame, layout[0]);
        let state = self.render_state();

        crate::ui::render::draw_footer(frame, layout[1], &state);
        crate::ui::render::draw_active_input_popup(frame, list_inner, &state);

        if state.link_popup_urls.is_some() {
            crate::ui::render::draw_link_popup(frame, area, &state);
        }

        if state.quit_confirm_pending {
            crate::ui::render::draw_quit_confirmation_popup(frame, area);
        }

        if state.show_help {
            crate::ui::render::draw_help(frame, area);
        }

        if state.ast_view_scroll.is_some() {
            crate::ui::render::draw_ast_popup(frame, area, &state);
        }
    }
}
