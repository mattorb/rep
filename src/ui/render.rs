use std::ops::Range;
use std::time::Duration;

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{RenderState, truncate_to_columns};
use crate::document_view::{CodeBlockStyleRequest, DisplaySpanStyleRequest};
use crate::output::keybinding_doc_rows;

use super::wrap_styled_spans;

const KEY_HUD_TTL: Duration = Duration::from_millis(5000);
const KEY_HUD_FADE_IN_TTL: Duration = Duration::from_millis(120);
const KEY_HUD_SOLID_TTL: Duration = Duration::from_millis(900);

pub(crate) struct RenderedDocument {
    pub(crate) rows: Vec<Vec<RenderedDisplayRow>>,
    pub(crate) node_heights: Vec<u16>,
}

pub(crate) struct RenderedDisplayRow {
    pub(crate) line: Line<'static>,
    pub(crate) byte_range: Range<usize>,
}

impl RenderedDisplayRow {
    fn spacer() -> Self {
        Self {
            line: Line::from(""),
            byte_range: 0..0,
        }
    }
}

pub(crate) fn document_block_title(state: &RenderState<'_>) -> String {
    let filename = state
        .source_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("markdown");
    let (change_count, feedback_count, insert_count, strike_count) = state.annotation_counts();
    if change_count == 0 && feedback_count == 0 && insert_count == 0 && strike_count == 0 {
        format!(" {filename} ")
    } else {
        let mut parts = Vec::new();
        if change_count > 0 {
            parts.push(format!("{change_count}C"));
        }
        if feedback_count > 0 {
            parts.push(format!("{feedback_count}F"));
        }
        if insert_count > 0 {
            parts.push(format!("{insert_count}I"));
        }
        if strike_count > 0 {
            parts.push(format!("{strike_count}X"));
        }
        format!(" {filename}  {} ", parts.join(" · "))
    }
}

pub(crate) fn build_document_rows(
    state: &RenderState<'_>,
    wrapped_text_width: usize,
) -> RenderedDocument {
    let node_count = state.view.node_count();
    let mut node_heights: Vec<u16> = Vec::with_capacity(node_count);
    let rows = (0..node_count)
        .map(|node_idx| {
            let (indicator, indicator_style) = node_indicator(state, node_idx);
            let add_spacer_after =
                node_idx + 1 < node_count && state.view.is_block_start(node_idx + 1);
            let strike_units = state.strike_units_for(node_idx);

            if let Some(code_rows) = state.view.styled_code_block_rows(CodeBlockStyleRequest {
                node_idx,
                active_anchor: state.selection_state.anchor,
                section_highlight_active: state.section_highlight_active(node_idx),
                strike_units: &strike_units,
            }) {
                let mut display_rows: Vec<RenderedDisplayRow> =
                    Vec::with_capacity(code_rows.len() + 1);
                for (i, mut row) in code_rows.into_iter().enumerate() {
                    let mut spans = vec![if i == 0 {
                        Span::styled(format!("{indicator} "), indicator_style)
                    } else {
                        Span::raw("  ")
                    }];
                    spans.append(&mut row.spans);
                    display_rows.push(RenderedDisplayRow {
                        line: Line::from(spans),
                        byte_range: row.byte_range,
                    });
                }
                if add_spacer_after {
                    display_rows.push(RenderedDisplayRow::spacer());
                }
                let height = display_rows.len().max(1) as u16;
                node_heights.push(height);
                return display_rows;
            }

            let spans = render_node_spans(state, node_idx);
            let mut display_rows: Vec<RenderedDisplayRow> = state
                .view
                .wrapped_display_rows(node_idx, spans, wrapped_text_width)
                .into_iter()
                .enumerate()
                .map(|(seg_idx, mut row)| {
                    let mut line_spans = Vec::new();
                    if seg_idx == 0 {
                        line_spans.push(Span::styled(format!("{indicator} "), indicator_style));
                    } else {
                        line_spans.push(Span::raw("  "));
                    }
                    line_spans.append(&mut row.spans);
                    RenderedDisplayRow {
                        line: Line::from(line_spans),
                        byte_range: row.byte_range,
                    }
                })
                .collect();

            if add_spacer_after {
                display_rows.push(RenderedDisplayRow::spacer());
            }
            let height = display_rows.len().max(1) as u16;
            node_heights.push(height);
            display_rows
        })
        .collect();

    RenderedDocument { rows, node_heights }
}

pub(crate) fn visible_document_lines(
    node_rows: &[Vec<RenderedDisplayRow>],
    scroll_offset: usize,
    inner_height: u16,
) -> (Vec<Line<'static>>, Vec<(usize, Range<usize>)>) {
    let mut visible = Vec::new();
    let mut visible_row_ranges = Vec::new();
    let mut count = 0u16;
    'outer: for (node_idx, rows) in node_rows.iter().enumerate().skip(scroll_offset) {
        for row in rows {
            if count >= inner_height {
                break 'outer;
            }
            visible.push(row.line.clone());
            visible_row_ranges.push((node_idx, row.byte_range.clone()));
            count += 1;
        }
    }
    (visible, visible_row_ranges)
}

fn node_indicator(state: &RenderState<'_>, node_idx: usize) -> (&'static str, Style) {
    let (change_count, feedback_count, insert_count, strike_count) =
        state.annotation_counts_for(node_idx);

    let total = change_count + feedback_count + insert_count + strike_count;
    if total > 1 {
        return (
            "*",
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        );
    }
    if change_count > 0 {
        return (
            "C",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    if feedback_count > 0 {
        return (
            "F",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    if insert_count > 0 {
        return (
            "+",
            Style::default()
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        );
    }
    if strike_count > 0 {
        return ("X", Style::default().fg(Color::LightRed));
    }
    (" ", Style::default().fg(Color::DarkGray))
}

pub(crate) fn render_node_spans(state: &RenderState<'_>, node_idx: usize) -> Vec<Span<'static>> {
    let strike_units = state.strike_units_for(node_idx);

    state
        .view
        .styled_display_spans(DisplaySpanStyleRequest {
            node_idx,
            active_anchor: state.selection_state.anchor,
            section_highlight_active: state.section_highlight_active(node_idx),
            strike_units: &strike_units,
        })
        .unwrap_or_else(|| {
            vec![Span::styled(
                " ",
                Style::default().add_modifier(Modifier::DIM),
            )]
        })
}

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

pub(crate) fn draw_key_hud(frame: &mut Frame, area: Rect, state: &RenderState<'_>) {
    let Some(hud) = state.key_hud else {
        return;
    };
    let age = hud.shown_at.elapsed();
    if age >= KEY_HUD_TTL || area.width < 12 || area.height < 5 {
        return;
    }

    let text_width = hud.text.chars().count() as u16;
    let width = (text_width + 10).clamp(16, area.width.min(40));
    let height = 3;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) * 3 / 4;
    let hud_area = Rect::new(x, y, width, height);
    let (text_style, border_style) = if age < KEY_HUD_FADE_IN_TTL {
        (
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        )
    } else if age < KEY_HUD_SOLID_TTL {
        (
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if age < Duration::from_millis(1550) {
        (
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Yellow),
        )
    } else {
        (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        )
    };

    frame.render_widget(Clear, hud_area);
    frame.render_widget(
        Paragraph::new(Span::styled(hud.text.clone(), text_style))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(border_style),
            )
            .alignment(Alignment::Center),
        hud_area,
    );
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
    hint: Option<&str>,
    prompt: &str,
    buf: &str,
) {
    let heights = state.cached_node_heights;
    if list_inner.width < 12
        || list_inner.height < 3
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

    let hint_height = hint
        .map(|hint| wrap_styled_spans(vec![Span::raw(hint.to_owned())], inner_width).len() as u16)
        .unwrap_or(0);
    let body_height =
        wrap_styled_spans(vec![Span::raw(format!("{prompt}{buf}"))], inner_width).len() as u16;
    let needed_height = hint_height
        .saturating_add(body_height.max(1))
        .saturating_add(2);
    let max_popup_height = list_inner.height.saturating_sub(2).max(3);
    let popup_height = needed_height.clamp(3, max_popup_height);

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

    let mut lines = Vec::new();
    if let Some(hint) = hint {
        lines.push(Line::from(Span::styled(
            hint.to_owned(),
            Style::default().fg(Color::Yellow),
        )));
    }
    lines.push(Line::from(format!("{prompt}{buf}")));

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
        Line::from("  j, k    next/prev unit"),
        Line::from("  i, o    finer/coarser units"),
        Line::from(""),
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

pub(crate) fn draw_quit_confirmation_popup(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(" Are you sure? "),
        Line::from("  "),
        Line::from(" Results will be printed to stdout. "),
        Line::from("  "),
        Line::from(vec![
            Span::raw(" "),
            Span::styled("y", Style::default().fg(Color::Green)),
            Span::raw(" confirm   "),
            Span::styled("n", Style::default().fg(Color::Red)),
            Span::raw(" cancel"),
            Span::raw(" "),
        ]),
    ];

    let content_width: u16 = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum::<usize>()
        })
        .max()
        .unwrap_or(40) as u16;
    let popup_width = (content_width + 4).max(44).min(area.width);
    let popup_height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(popup_width) / 2,
        y: area.y + area.height.saturating_sub(popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(" Quit ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
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
