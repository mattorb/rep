use super::*;

impl App {
    pub(super) fn draw_footer(frame: &mut Frame, area: Rect, state: &RenderState<'_>) {
        // Two-zone footer: persistent left mode indicator + transient right
        // zone (nav feedback / notification / hint). Mode indicator is never
        // truncated; right zone shrinks first under width pressure.
        let mode_text = format!(" mode: {}", state.mode_indicator);
        let mode_style = Style::default().fg(Color::Cyan);
        let hint_style = Style::default().fg(Color::DarkGray);
        // Right zone priority: transient nav_feedback (one-keypress
        // boundary message) > transient notification (clipboard result) >
        // persistent status (current mode / last action context) > help
        // hint. The status field accumulates input-mode prompts and
        // action-confirmation messages; without showing it the user never
        // sees mode prompts like "Change mode: type text and press Enter."
        let right_text = if let Some(fb) = state.nav_feedback {
            (fb.to_string(), Style::default().fg(Color::Yellow))
        } else if let Some(note) = state.notification {
            (note.to_string(), Style::default().fg(Color::Green))
        } else if !state.status.is_empty() {
            (state.status.to_string(), Style::default().fg(Color::Gray))
        } else {
            ("? for help ".to_string(), hint_style)
        };
        // Account for terminal column width (not byte length) so user-supplied
        // text in the right zone — e.g. a search query containing CJK or
        // emoji — doesn't underestimate the gap and squish the footer when
        // the message contains wide-width characters.
        let total_width = area.width as usize;
        let mode_w = UnicodeWidthStr::width(mode_text.as_str());
        let right_avail = total_width.saturating_sub(mode_w + 1);
        let right_str = truncate_to_columns(&right_text.0, right_avail);
        let right_w = UnicodeWidthStr::width(right_str.as_str());
        let gap = total_width.saturating_sub(mode_w + right_w);
        let footer_line = Line::from(vec![
            Span::styled(mode_text, mode_style),
            Span::raw(" ".repeat(gap)),
            Span::styled(right_str, right_text.1),
        ]);
        frame.render_widget(Paragraph::new(footer_line), area);
    }
}
