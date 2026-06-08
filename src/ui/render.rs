use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{RenderState, truncate_to_columns};
use crate::output::keybinding_doc_rows;

use super::wrap_styled_spans;

pub(crate) fn draw_footer(frame: &mut Frame, area: Rect, state: &RenderState<'_>) {
    let mode_text = format!(" mode: {}", state.mode_indicator);
    let mode_style = Style::default().fg(Color::Cyan);
    let hint_style = Style::default().fg(Color::DarkGray);
    let right_text = if let Some(fb) = state.nav_feedback {
        (fb.to_string(), Style::default().fg(Color::Yellow))
    } else if let Some(note) = state.notification {
        (note.to_string(), Style::default().fg(Color::Green))
    } else if !state.status.is_empty() {
        (state.status.to_string(), Style::default().fg(Color::Gray))
    } else {
        ("? for help ".to_string(), hint_style)
    };
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

pub(crate) fn draw_active_input_popup(
    frame: &mut Frame,
    list_inner: Rect,
    state: &RenderState<'_>,
) {
    if let Some((title, hint, prompt, buf)) = state.input_popup_spec() {
        draw_input_popup(frame, list_inner, state, title, hint, prompt, buf);
    }
}

fn draw_input_popup(
    frame: &mut Frame,
    list_inner: Rect,
    state: &RenderState<'_>,
    title: &str,
    hint: &str,
    prompt: &str,
    buf: &str,
) {
    let heights = state.cached_node_heights;
    if list_inner.width < 12
        || list_inner.height < 4
        || state.selection_state.anchor.node_idx >= heights.len()
    {
        return;
    }

    let list_offset = state.scroll_offset;
    if state.selection_state.anchor.node_idx < list_offset {
        return;
    }

    let selected_top: u16 = heights
        .iter()
        .skip(list_offset)
        .take(state.selection_state.anchor.node_idx - list_offset)
        .copied()
        .sum();
    let selected_height = heights[state.selection_state.anchor.node_idx].max(1);

    if selected_top >= list_inner.height {
        return;
    }

    let popup_width = list_inner.width.clamp(20, 80);
    let inner_width = popup_width.saturating_sub(2) as usize;

    let hint_height = wrap_styled_spans(vec![Span::raw(hint.to_owned())], inner_width).len() as u16;
    let body_height =
        wrap_styled_spans(vec![Span::raw(format!("{prompt}{buf}"))], inner_width).len() as u16;
    let needed_height = hint_height
        .max(1)
        .saturating_add(body_height.max(1))
        .saturating_add(2);
    let max_popup_height = list_inner.height.saturating_sub(2).max(4);
    let popup_height = needed_height.clamp(4, max_popup_height);

    let list_bottom = list_inner.y + list_inner.height;
    let preferred_below_y = list_inner.y
        + selected_top
            .saturating_add(selected_height)
            .min(list_inner.height.saturating_sub(1));
    let anchor_above_top = list_inner.y + selected_top;
    let y = if preferred_below_y.saturating_add(popup_height) <= list_bottom {
        preferred_below_y
    } else if anchor_above_top >= list_inner.y.saturating_add(popup_height) {
        anchor_above_top - popup_height
    } else {
        list_bottom.saturating_sub(popup_height).max(list_inner.y)
    };

    let popup = Rect {
        x: list_inner.x,
        y,
        width: popup_width,
        height: popup_height,
    };

    let lines = vec![
        Line::from(Span::styled(
            hint.to_owned(),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(format!("{prompt}{buf}")),
    ];

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(title.to_owned())
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub(crate) fn draw_help(frame: &mut Frame, area: Rect) {
    let keybindings = keybinding_doc_rows();
    let key_action = |action: &str| {
        keybindings
            .iter()
            .find(|row| row.action == action)
            .map(|row| row.keys.replace('`', ""))
            .expect("help action has keybinding documentation")
    };
    let help_lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Navigate", Style::default().fg(Color::Cyan))),
        Line::from(format!(
            "  {}, {}    next/prev unit",
            key_action("Move to the next active unit"),
            key_action("Move to the previous active unit")
        )),
        Line::from(format!(
            "  {}, {}    next/prev selection unit",
            key_action("Cycle to the next selection unit"),
            key_action("Cycle to the previous selection unit")
        )),
        Line::from(format!(
            "  {}, {}    finer/coarser units",
            key_action("Use a finer selection unit"),
            key_action("Use a coarser selection unit")
        )),
        Line::from(Span::styled("  Annotate", Style::default().fg(Color::Cyan))),
        Line::from(format!(
            "  {}       change (literal)",
            key_action("Add or edit a literal change request")
        )),
        Line::from(format!(
            "  {}       feedback (intent)",
            key_action("Add or edit feedback or intent")
        )),
        Line::from(format!(
            "  {}, {}    insert before/after",
            key_action("Insert text before the current unit"),
            key_action("Insert text after the current unit")
        )),
        Line::from(format!(
            "  {}       clear or strike",
            key_action("Clear existing annotations or mark the unit for deletion")
        )),
        Line::from(""),
        Line::from(format!(
            "  {}    prev/next annotation",
            key_action("Jump to the previous or next annotation")
        )),
        Line::from(format!(
            "  {}       edit annotation",
            key_action("Edit an existing annotation")
        )),
        Line::from(format!("  {}       search", key_action("Search"))),
        Line::from(format!(
            "  {}    next/prev search match",
            key_action("Jump to the next or previous search match")
        )),
        Line::from(""),
        Line::from(format!(
            "  {}       copy annotations to clipboard",
            key_action("Copy annotations to the clipboard")
        )),
        Line::from(format!(
            "  {}       quit; printing changes to stdout",
            key_action("Quit and print annotations to stdout")
        )),
        Line::from(format!(
            "  {}       silent quit (discard annotations)",
            key_action("Quit silently and discard annotations")
        )),
    ];

    let content_width: u16 = help_lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(40) as u16;
    let content_height = help_lines.len() as u16;
    let popup_width = (content_width + 2).max(72).min(area.width);
    let popup_height = (content_height + 2).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(help_lines))
            .block(
                Block::default()
                    .title(" Help ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

pub(crate) fn draw_ast_popup(frame: &mut Frame, area: Rect, state: &RenderState<'_>) {
    let popup_width = (area.width * 4 / 5).max(40).min(area.width);
    let popup_height = (area.height * 4 / 5).max(6).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    let lines: Vec<Line> = state
        .ast_lines
        .iter()
        .map(|l| Line::from(Span::raw(l.clone())))
        .collect();

    let total = state.ast_lines.len() as u16;
    // With wrap enabled the scroll axis is display-rows, not
    // source-lines; long lines wrap to multiple rows so the user
    // can drift past `total` worth of "lines" before exhausting
    // visible content. Cap scroll to total source lines as a
    // reasonable upper bound — overshoot just shows blank rows
    // at the bottom rather than truncating right-edge content.
    let inner_height = popup_height.saturating_sub(2);
    let max_scroll = total.saturating_sub(inner_height);
    let scroll = state.ast_view_scroll.unwrap_or(0).min(max_scroll);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(format!(
                        " AST  [{}/{}]  j/k scroll · I/Esc close ",
                        scroll + 1,
                        total
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        popup,
    );
}

pub(crate) fn draw_link_popup(frame: &mut Frame, area: Rect, state: &RenderState<'_>) {
    // Caller in draw() gates on link_popup_urls.is_some(), so the
    // None case here is unreachable; default to an empty slice if
    // it ever fires.
    let urls = state.link_popup_urls.unwrap_or(&[]);
    let popup_width = area.width.saturating_sub(10).clamp(40, 100);
    let max_height = area.height.saturating_sub(6).max(6);
    let desired_height = (urls.len() as u16).saturating_add(5).clamp(6, max_height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(desired_height) / 2,
        width: popup_width,
        height: desired_height,
    };

    let mut lines = Vec::new();
    lines.push(Line::from("Links in current sentence:"));
    lines.push(Line::from(""));
    for (idx, url) in urls.iter().enumerate() {
        lines.push(Line::from(format!("{}. {}", idx + 1, url)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press i or Esc to close",
        Style::default().fg(Color::Gray),
    )));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(" Link ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}
