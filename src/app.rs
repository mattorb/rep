use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::document::{DocNode, Document};
use crate::markdown::{MarkdownLinkRange, render_markdown_line};
use crate::output::clean_context;
#[cfg(test)]
use crate::output::{
    AgentOutput, ChangeOutput, FeedbackOutput, InsertOutput, KeymapOutput, LineAnnotationOutput,
    LineContext, ReactionOutput,
};
use crate::selection::index::SelectionIndex;
use crate::selection::model::{SelectionAnchor, SelectionState, SelectionUnit};
use crate::ui::wrap_styled_spans;

const FOOTER_HEIGHT: u16 = 1;
const GUTTER_WIDTH: usize = 2;

/// Max char width for the `target:` quote in `to_human_output`. Long
/// enough that typical sentences fit; short enough to keep emit
/// blocks readable when consumed by an LLM.
const EMIT_TARGET_MAX_CHARS: usize = 180;

/// Max char width for the prev/next CONTEXT lines around `target:`.
/// Slightly narrower than EMIT_TARGET_MAX_CHARS — they're decoration.
const EMIT_CONTEXT_MAX_CHARS: usize = 140;

/// Max char width for the action payload (`CHANGE:` / `FEEDBACK:` /
/// `INSERT:`). Wider than the target/context — the user types these
/// so they tend to be longer.
const EMIT_PAYLOAD_MAX_CHARS: usize = 220;

// ── Annotation types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputMode {
    Normal,
    Change,
    Feedback,
    InsertBefore,
    InsertAfter,
    Search,
    /// Editing the change at (node_idx, change_idx).
    EditChange(usize, usize),
    /// Editing the feedback at (node_idx, feedback_idx).
    EditFeedback(usize, usize),
}

#[derive(Debug, Clone)]
struct ChangeAnnotation {
    created_at: String,
    /// Selection unit at the moment of capture (Sentence / Line / Paragraph
    /// / Section / Word). Drives WHERE: format and target: source per
    /// modular_plan §"target".
    target_unit: SelectionUnit,
    sentence_index: Option<usize>,
    sentence_text: Option<String>,
    change: String,
}

#[derive(Debug, Clone)]
struct FeedbackAnnotation {
    created_at: String,
    target_unit: SelectionUnit,
    sentence_index: Option<usize>,
    sentence_text: Option<String>,
    feedback: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditableAnnotation {
    Change(usize),
    Feedback(usize),
}

#[derive(Debug, Clone)]
struct InsertAnnotation {
    // Consumed only by the #[cfg(test)] InsertOutput in src/output.rs;
    // dead in non-test builds. ChangeAnnotation / FeedbackAnnotation
    // also read created_at for stale-annotation suppression in
    // current_target_capture, so theirs are reachable everywhere.
    #[allow(dead_code)]
    created_at: String,
    target_unit: SelectionUnit,
    sentence_index: Option<usize>,
    sentence_text: Option<String>,
    text: String,
}

// ── Rendered document node ────────────────────────────────────────────────────

/// Per-node rendering cache: styled spans for the joined source text plus
/// sentence byte-range boundaries within `plain`.
struct RenderedNode {
    plain: String,
    spans: Vec<Span<'static>>,
    sentence_ranges: Vec<Range<usize>>,
    links: Vec<MarkdownLinkRange>,
}

impl std::fmt::Debug for RenderedNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderedNode")
            .field("plain", &self.plain)
            .field("sentence_ranges", &self.sentence_ranges)
            .finish_non_exhaustive()
    }
}

fn build_rendered_nodes(doc: &Document, source_lines: &[String]) -> Vec<RenderedNode> {
    doc.nodes
        .iter()
        .map(|n| build_rendered_node(n, source_lines))
        .collect()
}

fn build_rendered_node(node: &DocNode, source_lines: &[String]) -> RenderedNode {
    match node {
        DocNode::Heading { source_line, .. } => {
            let text = source_lines.get(*source_line).cloned().unwrap_or_default();
            let r = render_markdown_line(&text);
            let ranges = single_range(&r.plain);
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: ranges,
                links: r.links,
            }
        }
        DocNode::Paragraph {
            source_lines: range,
            ..
        } => {
            let src = &source_lines[clamp_range(range, source_lines.len())];
            let (plain, spans, links) = render_source_lines_with_breaks(src);
            if plain.is_empty() {
                let joined = join_node_source_lines(src);
                let r = render_markdown_line(&joined);
                let sentence_ranges = single_range(&r.plain);
                RenderedNode {
                    plain: r.plain,
                    spans: r.spans,
                    sentence_ranges,
                    links: r.links,
                }
            } else {
                let sentence_ranges = crate::selection::segment::segment_sentences(&plain);
                RenderedNode {
                    plain,
                    spans,
                    sentence_ranges,
                    links,
                }
            }
        }
        DocNode::ListItem {
            source_lines: range,
            ordered,
            depth,
            ..
        } => {
            let joined =
                join_node_source_lines(&source_lines[clamp_range(range, source_lines.len())]);
            let r = render_markdown_line(&joined);
            let ranges = single_range(&r.plain);
            // Top-level ordered items act as section headings — style them like one.
            let spans = if *ordered && *depth == 0 {
                let style = Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD);
                vec![Span::styled(r.plain.clone(), style)]
            } else {
                r.spans
            };
            RenderedNode {
                plain: r.plain,
                spans,
                sentence_ranges: ranges,
                links: r.links,
            }
        }
        DocNode::CodeBlock {
            source_lines: range,
            ..
        } => {
            let raw = &source_lines[clamp_range(range, source_lines.len())];
            let plain = raw.join("\n");
            let ranges = single_range(&plain);
            let spans = raw
                .iter()
                .flat_map(|line| {
                    let style = if line.trim_start().starts_with("```") {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White).bg(Color::DarkGray)
                    };
                    [
                        Span::styled(line.clone(), style),
                        Span::raw("\n".to_owned()),
                    ]
                })
                .collect();
            RenderedNode {
                plain,
                spans,
                sentence_ranges: ranges,
                links: vec![],
            }
        }
        DocNode::ThematicBreak { .. } => {
            let r = render_markdown_line("---");
            RenderedNode {
                plain: r.plain,
                spans: r.spans,
                sentence_ranges: vec![],
                links: r.links,
            }
        }
    }
}

/// Return a single byte-range covering the entire string, or empty if empty.
#[allow(clippy::single_range_in_vec_init)]
fn single_range(s: &str) -> Vec<std::ops::Range<usize>> {
    if s.is_empty() {
        vec![]
    } else {
        vec![0..s.len()]
    }
}

/// Join soft-wrapped source lines into one string for paragraph
/// rendering. The first line keeps its leading whitespace (so list
/// item markers stay aligned); continuation lines are trimmed before
/// joining. Empty lines are dropped.
fn join_node_source_lines(lines: &[String]) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.as_str() } else { l.trim() })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render multi-line paragraph source into per-line spans joined by '\n'.
///
/// Relative indentation is preserved: each line is shown indented by its source
/// indentation minus the first line's indentation, so sub-items appear visually
/// nested without any content-altering markdown indentation interpretation.
fn render_source_lines_with_breaks(
    src_lines: &[String],
) -> (String, Vec<Span<'static>>, Vec<MarkdownLinkRange>) {
    let mut plain = String::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut links: Vec<MarkdownLinkRange> = Vec::new();
    let first_indent = src_lines
        .first()
        .map_or(0, |l| l.len() - l.trim_start().len());
    for (i, line) in src_lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !plain.is_empty() {
            plain.push('\n');
            spans.push(Span::raw("\n".to_owned()));
        }
        let relative_indent = if i == 0 {
            0
        } else {
            let line_indent = line.len() - line.trim_start().len();
            line_indent.saturating_sub(first_indent)
        };
        if relative_indent > 0 {
            let prefix = " ".repeat(relative_indent);
            plain.push_str(&prefix);
            spans.push(Span::raw(prefix));
        }
        let offset = plain.len();
        let r = render_markdown_line(trimmed);
        for link in r.links {
            links.push(MarkdownLinkRange {
                start: link.start + offset,
                end: link.end + offset,
                url: link.url,
            });
        }
        plain.push_str(&r.plain);
        spans.extend(r.spans);
    }
    (plain, spans, links)
}

fn clamp_range(r: &Range<usize>, len: usize) -> Range<usize> {
    r.start.min(len)..r.end.min(len)
}

/// Truncate `s` to fit within `max_cols` terminal columns, accounting for
/// wide-width characters (CJK, emoji). Wraps zero-width chars (combining
/// marks) into the preceding character's column count. Returns an empty
/// string when `max_cols == 0`, including for inputs that begin with a
/// zero-width character (an orphaned combining mark renders unpredictably
/// across terminals; "no output" is the safer choice).
fn truncate_to_columns(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > max_cols {
            break;
        }
        used += w;
        out.push(ch);
    }
    out
}

// ── App ───────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct App {
    source_path: PathBuf,
    /// Original source lines — used only for output context (prev/next line).
    source_lines: Vec<String>,
    doc: Document,
    rendered_nodes: Vec<RenderedNode>,
    /// Owned eager selection index built once at load time per Req 11.
    /// Drives navigation, per-unit emit, and unit-aware highlight lookup.
    index: SelectionIndex,
    /// Canonical selection state — `(node_idx, unit, unit_idx)` per
    /// modular_plan §"Selection state".
    selection_state: SelectionState,
    /// When set on entry to Section mode (or by `move_section`), highlights
    /// every node from the section start through `end_node_idx` inclusive.
    /// Cleared on the next non-Section move_active_unit step.
    section_highlight_range: Option<Range<usize>>,
    /// Annotations keyed by node index.
    changes: BTreeMap<usize, Vec<ChangeAnnotation>>,
    feedbacks: BTreeMap<usize, Vec<FeedbackAnnotation>>,
    inserts_before: BTreeMap<usize, Vec<InsertAnnotation>>,
    inserts_after: BTreeMap<usize, Vec<InsertAnnotation>>,
    /// Strikes are keyed by (unit, unit_idx) within each node so a single
    /// node can carry strikes at different granularities — Sentence-unit
    /// strikes (the original shape) and Word/Line/Paragraph/Section strikes.
    /// BTreeSet ordering gives deterministic emit order.
    strikes: BTreeMap<usize, BTreeSet<(SelectionUnit, usize)>>,
    input_mode: InputMode,
    change_buffer: String,
    feedback_buffer: String,
    insert_buffer: String,
    search_buffer: String,
    last_search: Option<String>,
    status: String,
    notification: Option<String>,
    /// Transient navigator feedback (e.g. `"at end"`, `"at start"`). Lives in
    /// the right zone of the two-zone footer alongside `notification`. Cleared
    /// at the top of `handle_normal_key` so the message is shown for exactly
    /// one keypress before being overwritten or cleared.
    nav_feedback: Option<String>,
    pub should_quit: bool,
    pub silent_quit: bool,
    /// Some(urls) when the link popup is visible; None when hidden. The
    /// urls and the visibility flag used to be two separate fields, which
    /// allowed inconsistent state (visible-but-empty / hidden-with-urls).
    link_popup_urls: Option<Vec<String>>,
    show_help: bool,
    /// Some(scroll_pos) when the AST popup is visible; None when hidden.
    /// Replaces the prior show_ast: bool + ast_scroll: u16 pair so the
    /// "visible iff scroll-position-is-meaningful" invariant lives in the
    /// type. ast_lines stays separate — it's always loaded at startup.
    ast_view_scroll: Option<u16>,
    ast_lines: Vec<String>,
    scroll_offset: usize,
    list_inner: Rect,
    cached_node_heights: Vec<u16>,
}

impl App {
    pub fn load(path: PathBuf) -> Result<Self> {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read markdown file: {}", path.display()))?;

        let source_lines: Vec<String> = raw.lines().map(ToOwned::to_owned).collect();
        // Match Document::parse's options so the AST popup shows the same
        // tree the selection layer reads — otherwise tables, strikethrough,
        // and footnote refs would render in the popup with one shape and
        // be parsed by Document with another.
        let ast_text = {
            let mut opts = markdown::ParseOptions::gfm();
            opts.constructs.frontmatter = true;
            markdown::to_mdast(&raw, &opts)
        }
        .map_or_else(
            |_| "Failed to parse AST".to_string(),
            |node| format!("{node:#?}"),
        );
        let ast_lines: Vec<String> = ast_text.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(&raw);
        let rendered_nodes = build_rendered_nodes(&doc, &source_lines);
        let index = SelectionIndex::build(&doc, &source_lines);

        let initial_node = doc.next_content_node(0).unwrap_or(0);
        let selection_state = SelectionState::new(SelectionAnchor::new(
            initial_node,
            SelectionUnit::Sentence,
            0,
        ));

        Ok(Self {
            source_path: path,
            source_lines,
            doc,
            rendered_nodes,
            index,
            selection_state,
            section_highlight_range: None,
            changes: BTreeMap::new(),
            feedbacks: BTreeMap::new(),
            inserts_before: BTreeMap::new(),
            inserts_after: BTreeMap::new(),
            strikes: BTreeMap::new(),
            input_mode: InputMode::Normal,
            change_buffer: String::new(),
            feedback_buffer: String::new(),
            insert_buffer: String::new(),
            search_buffer: String::new(),
            last_search: None,
            status: "Loaded file. Press q to quit and print annotations.".to_string(),
            should_quit: false,
            silent_quit: false,
            link_popup_urls: None,
            show_help: false,
            ast_view_scroll: None,
            ast_lines,
            notification: None,
            nav_feedback: None,
            scroll_offset: 0,
            list_inner: Rect::default(),
            cached_node_heights: Vec::new(),
        })
    }

    /// Returns the current selection in the canonical
    /// `(node_idx, unit, unit_idx)` shape, used by the transcript harness.
    pub const fn current_anchor(&self) -> (usize, &'static str, usize) {
        let a = &self.selection_state.anchor;
        (a.node_idx, a.unit.as_str(), a.unit_idx)
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.input_mode.clone() {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Change => self.handle_change_key(key),
            InputMode::Feedback => self.handle_feedback_key(key),
            InputMode::InsertBefore => self.handle_insert_key(key, true),
            InputMode::InsertAfter => self.handle_insert_key(key, false),
            InputMode::Search => self.handle_search_key(key),
            InputMode::EditChange(node_idx, change_idx) => {
                self.handle_edit_change_key(key, node_idx, change_idx);
            }
            InputMode::EditFeedback(node_idx, feedback_idx) => {
                self.handle_edit_feedback_key(key, node_idx, feedback_idx);
            }
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        self.notification = None;
        self.nav_feedback = None;

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if let Some(scroll) = self.ast_view_scroll {
            match key.code {
                KeyCode::Esc | KeyCode::Char('I') => {
                    self.ast_view_scroll = None;
                    self.status = "Closed AST view.".to_string();
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.ast_view_scroll = Some(scroll.saturating_add(3));
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.ast_view_scroll = Some(scroll.saturating_sub(3));
                }
                _ => {}
            }
            return;
        }

        if self.link_popup_urls.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('U') => {
                    self.link_popup_urls = None;
                    self.status = "Closed link popup.".to_string();
                }
                _ => {
                    self.link_popup_urls = None;
                }
            }
            return;
        }

        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                _ => {
                    self.show_help = false;
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('Q') => {
                self.silent_quit = true;
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.search_buffer.clear();
                self.status = "Search: type pattern and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('n') => self.jump_search(true),
            KeyCode::Char('N') => self.jump_search(false),
            KeyCode::Char('c') => self.begin_change_or_edit(),
            KeyCode::Char('f') => self.begin_feedback_or_edit(),
            KeyCode::Char('b') => {
                self.input_mode = InputMode::InsertBefore;
                self.insert_buffer.clear();
                self.status = "Insert before: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('a') => {
                self.input_mode = InputMode::InsertAfter;
                self.insert_buffer.clear();
                self.status = "Insert after: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('e') => self.begin_edit_annotation(),
            KeyCode::Char(' ') => self.mode_cycle(true),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => self.move_active_unit(true),
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => self.move_active_unit(false),
            KeyCode::Backspace => self.mode_cycle(false),
            // i / u cycle the active selection unit by one step. i =
            // "in" / finer; u = "up" / coarser. Synonyms for Space and
            // Backspace.
            KeyCode::Char('i') => self.mode_cycle(true),
            KeyCode::Char('u') => self.mode_cycle(false),
            // Capital variants keep the prior bindings: I opens the
            // AST popup, U reveals links from the current sentence.
            KeyCode::Char('I') => {
                self.ast_view_scroll = Some(0);
                self.status = "AST view. j/k scroll, I or Esc close.".to_string();
            }
            KeyCode::Char('U') if !self.reveal_links_for_current_sentence() => {
                self.status = "No markdown links in current sentence.".to_string();
            }
            KeyCode::Char('x') => self.toggle_strike(),
            KeyCode::Char('r') => {
                let output = self.to_human_output();
                self.notification = Some(match copy_to_clipboard(&output) {
                    ClipboardOutcome::OsCommand => "Copied to clipboard".to_string(),
                    ClipboardOutcome::Osc52 => "Sent via OSC 52".to_string(),
                    ClipboardOutcome::Failed => "Copy failed — no clipboard available".to_string(),
                });
            }
            KeyCode::Char('[') => self.jump_to_annotation(false),
            KeyCode::Char(']') => self.jump_to_annotation(true),
            _ => {}
        }
    }

    fn handle_change_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
                self.status = "Change cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.change_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Change ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = ChangeAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        change: trimmed,
                    };
                    self.changes
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Change saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
            }
            KeyCode::Backspace => {
                self.change_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.change_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_insert_key(&mut self, key: KeyEvent, before: bool) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.insert_buffer.clear();
                self.status = if before {
                    "Insert before cancelled.".to_string()
                } else {
                    "Insert after cancelled.".to_string()
                };
            }
            KeyCode::Enter => {
                let trimmed = self.insert_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Insert ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = InsertAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        text: trimmed,
                    };
                    let bucket = if before {
                        &mut self.inserts_before
                    } else {
                        &mut self.inserts_after
                    };
                    bucket
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    let label = if before { "before" } else { "after" };
                    self.status = format!(
                        "Insert {label} saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.insert_buffer.clear();
            }
            KeyCode::Backspace => {
                self.insert_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.search_buffer.clear();
                self.status = "Search cancelled.".to_string();
            }
            KeyCode::Enter => {
                let query = self.search_buffer.trim().to_string();
                self.input_mode = InputMode::Normal;
                self.search_buffer.clear();
                if query.is_empty() {
                    self.status = "Search cancelled (empty pattern).".to_string();
                    return;
                }
                self.run_search(&query, true);
                self.last_search = Some(query);
            }
            KeyCode::Backspace => {
                self.search_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_buffer.push(ch);
            }
            _ => {}
        }
    }

    /// Find every search hit across rendered nodes as (node, sentence) pairs.
    /// Smart-case: case-sensitive iff the query contains an ASCII uppercase letter.
    fn find_search_matches(&self, query: &str) -> Vec<(usize, usize)> {
        if query.is_empty() {
            return Vec::new();
        }
        let case_sensitive = query.chars().any(|c| c.is_ascii_uppercase());
        let needle = if case_sensitive {
            query.to_owned()
        } else {
            let mut s = query.to_owned();
            s.make_ascii_lowercase();
            s
        };
        let mut matches: Vec<(usize, usize)> = Vec::new();
        for (ni, rn) in self.rendered_nodes.iter().enumerate() {
            let mut hay = rn.plain.clone();
            if !case_sensitive {
                hay.make_ascii_lowercase();
            }
            let mut cursor = 0usize;
            while cursor <= hay.len() {
                let Some(offset) = hay[cursor..].find(&needle) else {
                    break;
                };
                let abs = cursor + offset;
                let sidx = rn
                    .sentence_ranges
                    .iter()
                    .position(|r| abs >= r.start && abs < r.end)
                    .unwrap_or(0);
                matches.push((ni, sidx));
                cursor = abs + needle.len();
            }
        }
        matches
    }

    fn run_search(&mut self, query: &str, forward: bool) {
        let matches = self.find_search_matches(query);
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

    fn jump_search(&mut self, forward: bool) {
        let Some(query) = self.last_search.clone() else {
            self.status = "No previous search. Press / to search.".to_string();
            return;
        };
        let matches = self.find_search_matches(&query);
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
        // find_search_matches returns sentence-keyed positions, so the
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

    fn handle_feedback_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
                self.status = "Feedback cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.feedback_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Feedback ignored because it was empty.".to_string();
                } else {
                    let (sentence_index, sentence_text) =
                        if let Some((idx, text)) = self.current_target_capture() {
                            (Some(idx), Some(text))
                        } else {
                            (None, None)
                        };
                    let annotation = FeedbackAnnotation {
                        created_at: Utc::now().to_rfc3339(),
                        target_unit: self.selection_state.anchor.unit,
                        sentence_index,
                        sentence_text,
                        feedback: trimmed,
                    };
                    self.feedbacks
                        .entry(self.selection_state.anchor.node_idx)
                        .or_default()
                        .push(annotation);
                    self.status = format!(
                        "Feedback saved on node {} (line {}).",
                        self.selection_state.anchor.node_idx + 1,
                        self.current_source_line() + 1
                    );
                }
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
            }
            KeyCode::Backspace => {
                self.feedback_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.feedback_buffer.push(ch);
            }
            _ => {}
        }
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    /// Mouse scroll: move through every content node (nodes that have at
    /// least one selection anchor). Arrow keys are bound to
    /// `move_active_unit` per the mode-switch keymap; this helper
    /// only serves the mouse wheel handler now.
    fn move_node(&mut self, delta: isize) {
        if self.doc.node_count() == 0 || delta == 0 {
            return;
        }
        let steps = delta.unsigned_abs();
        let forward = delta.is_positive();
        let mut target = self.selection_state.anchor.node_idx;
        let mut moved = 0usize;

        for _ in 0..steps {
            let next = if forward {
                self.doc.next_content_node(target.saturating_add(1))
            } else {
                self.doc.prev_content_node(target)
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
            self.doc.node_count()
        );
    }

    /// j / k / Down / Up / Right / Left — move by the currently active
    /// selection unit. Pure delegate to `selection::navigator::next/prev`.
    /// On `Boundary`, set `nav_feedback` ("at end" / "at start") for one
    /// keypress in the right zone of the footer.
    fn move_active_unit(&mut self, forward: bool) {
        if self.doc.node_count() == 0 {
            return;
        }
        let outcome = if forward {
            crate::selection::navigator::next(&self.index, self.selection_state.anchor)
        } else {
            crate::selection::navigator::prev(&self.index, self.selection_state.anchor)
        };
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
                if !self.unit_has_any_anchor(self.selection_state.anchor.unit) {
                    return;
                }
                self.nav_feedback = Some(if forward { "at end" } else { "at start" }.to_string());
            }
        }
    }

    fn unit_has_any_anchor(&self, unit: SelectionUnit) -> bool {
        match unit {
            SelectionUnit::Section => !self.index.sections.is_empty(),
            SelectionUnit::Paragraph => !self.index.paragraphs.is_empty(),
            SelectionUnit::Line => !self.index.lines.is_empty(),
            SelectionUnit::Sentence => !self.index.sentences.is_empty(),
            SelectionUnit::Word => !self.index.words.is_empty(),
        }
    }

    /// Space (forward) / Backspace (reverse) — cycle the active selection
    /// unit. Re-anchors via `navigator::clamp` per the pinned rules.
    fn mode_cycle(&mut self, forward: bool) {
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
        let new_anchor =
            crate::selection::navigator::clamp(&self.index, self.selection_state.anchor, target);
        self.selection_state.anchor = new_anchor;
        self.refresh_section_highlight(new_anchor);
    }

    /// Refresh `section_highlight_range` for an anchor: set the span when
    /// the active unit is Section, clear otherwise. Used after every move
    /// or mode-cycle that lands on a new anchor.
    fn refresh_section_highlight(&mut self, anchor: SelectionAnchor) {
        if anchor.unit == SelectionUnit::Section {
            self.section_highlight_range = Some(self.section_span_for(anchor.node_idx));
        } else {
            self.section_highlight_range = None;
        }
    }

    /// Compute the inclusive-start, exclusive-end node range for the section
    /// starting at `node_idx`. Falls back to the rest of the document if the
    /// section table doesn't carry an entry for this node.
    fn section_span_for(&self, node_idx: usize) -> Range<usize> {
        let end = self
            .index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)
            .map_or_else(|| self.doc.node_count(), |s| s.end_node_idx + 1);
        node_idx..end
    }

    /// Stable string for the mode indicator in the left zone of the footer.
    const fn mode_indicator(&self) -> &'static str {
        self.selection_state.anchor.unit.mode_str()
    }

    fn jump_to_annotation(&mut self, forward: bool) {
        let from = if forward {
            self.selection_state.anchor.node_idx + 1
        } else {
            self.selection_state.anchor.node_idx
        };
        let n = self.doc.node_count();
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

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        // Mouse interactions clear transient nav/notification feedback the
        // same way keypresses do (see handle_normal_key) — otherwise stale
        // "at end" / clipboard messages could linger after a click or
        // scroll.
        self.notification = None;
        self.nav_feedback = None;
        match mouse.kind {
            MouseEventKind::ScrollUp => self.move_node(-1),
            MouseEventKind::ScrollDown => self.move_node(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(node_idx) = self.click_to_node(mouse.row) {
                    self.selection_state.anchor.node_idx = node_idx;
                    self.clamp_sentence();
                    self.status = format!("Node {}/{}", node_idx + 1, self.doc.node_count());
                }
            }
            _ => {}
        }
    }

    fn click_to_node(&self, mouse_row: u16) -> Option<usize> {
        let inner = self.list_inner;
        if mouse_row < inner.y || mouse_row >= inner.y + inner.height {
            return None;
        }
        let visual_row = mouse_row - inner.y;
        let offset = self.scroll_offset;
        let mut cumulative = 0u16;
        for (i, &height) in self.cached_node_heights.iter().skip(offset).enumerate() {
            if visual_row < cumulative + height {
                return Some(offset + i);
            }
            cumulative += height;
        }
        None
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
    fn clamp_sentence(&mut self) {
        let total = self
            .rendered_nodes
            .get(self.selection_state.anchor.node_idx)
            .map_or(0, |rn| rn.sentence_ranges.len());
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

    fn current_source_line(&self) -> usize {
        // Return the source line where the active selection's text begins,
        // routed per the active unit. Used by status messages so the line
        // number shown matches the captured annotation's WHERE: line.
        let node_idx = self.selection_state.anchor.node_idx;
        let node_first_line = self
            .doc
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        self.where_for_annotation(
            self.selection_state.anchor.unit,
            node_idx,
            Some(self.selection_state.anchor.unit_idx),
            node_first_line,
        )
    }

    fn current_sentence_context(&self) -> Option<(usize, String)> {
        // Per modular_plan §"Internal representation" Req 11: emit consumes
        // the selection-plain-text view (markers stripped), not the display
        // view. Reading from rendered_nodes here used to leak `[ ]` task
        // markers, `[^N]` footnote refs, etc., into the captured target.
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node = self.index.nodes.get(self.selection_state.anchor.node_idx)?;
        let range = node.sentence_ranges.get(unit_idx)?;
        let text = node
            .selection_plain_text
            .get(range.clone())?
            .trim()
            .to_string();
        Some((unit_idx, text))
    }

    /// Capture the `(unit_idx, target_text)` snapshot stored on the
    /// annotation. Routes by `selection_state.anchor.unit`:
    ///   - Sentence: rendered-display sentence text via current_sentence_context.
    ///   - Line: source line verbatim for non-ListItem; full item text
    ///     (markers stripped, soft-wrapped lines space-joined) for ListItem.
    ///   - Word: word's selection plain text (punctuation stripped per
    ///     word-boundary rules).
    ///   - Paragraph: full node selection plain text, internal newlines
    ///     collapsed to spaces.
    ///   - Section: constituent nodes' selection plain text, joined with
    ///     single spaces, internal newlines collapsed.
    fn current_target_capture(&self) -> Option<(usize, String)> {
        match self.selection_state.anchor.unit {
            SelectionUnit::Line => self.current_line_capture(),
            SelectionUnit::Word => self.current_word_capture(),
            SelectionUnit::Paragraph => self.current_paragraph_capture(),
            SelectionUnit::Section => self.current_section_capture(),
            SelectionUnit::Sentence => self.current_sentence_context(),
        }
    }

    fn current_paragraph_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let plain = self
            .index
            .nodes
            .get(node_idx)
            .map(|n| n.selection_plain_text.clone())?;
        // Per modular_plan §"target": Paragraph emit is single-line. The
        // index stores tables and other multi-line paragraph plain text
        // joined by `\n` for line-unit navigation; the emit collapses that
        // back to single space.
        Some((0, plain.replace('\n', " ")))
    }

    fn current_section_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let section = self
            .index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)?;
        let mut parts: Vec<String> = Vec::new();
        for i in section.start_node_idx..=section.end_node_idx {
            if let Some(n) = self.index.nodes.get(i)
                && !n.selection_plain_text.is_empty()
            {
                // Constituent node text may contain `\n` (multi-line
                // paragraph or table) — collapse to single space per
                // modular_plan §"Section": no embedded newlines in
                // target:.
                parts.push(n.selection_plain_text.replace('\n', " "));
            }
        }
        Some((0, parts.join(" ")))
    }

    fn current_word_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node = self.index.nodes.get(node_idx)?;
        let range = node.word_ranges.get(unit_idx)?;
        let text = node.selection_plain_text.get(range.clone())?.to_string();
        Some((unit_idx, text))
    }

    fn current_line_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let unit_idx = self.selection_state.anchor.unit_idx;
        if let DocNode::ListItem { .. } = self.doc.nodes.get(node_idx)? {
            // ListItem at line unit: full item text, markers already
            // stripped by the index's selection_plain_text.
            let plain = self
                .index
                .nodes
                .get(node_idx)
                .map(|n| n.selection_plain_text.clone())?;
            Some((unit_idx, plain))
        } else {
            // Non-ListItem: source line verbatim.
            let (line, _) = self
                .index
                .nodes
                .get(node_idx)?
                .source_line_ranges
                .get(unit_idx)?
                .clone();
            let line_text = self.source_lines.get(line)?.clone();
            Some((unit_idx, line_text))
        }
    }

    /// Lookup variant of `current_target_capture` that takes an explicit
    /// `(node_idx, unit, unit_idx)` instead of reading from
    /// `selection_state.anchor`. Used by strike emit to render the target
    /// text for each saved (unit, unit_idx) strike entry.
    fn target_text_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<String> {
        let node = self.index.nodes.get(node_idx)?;
        match unit {
            SelectionUnit::Sentence => {
                let r = node.sentence_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.trim().to_string())
            }
            SelectionUnit::Word => {
                let r = node.word_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.to_string())
            }
            SelectionUnit::Line => {
                if let DocNode::ListItem { .. } = self.doc.nodes.get(node_idx)? {
                    Some(node.selection_plain_text.clone())
                } else {
                    let (line, _) = node.source_line_ranges.get(unit_idx)?.clone();
                    Some(self.source_lines.get(line)?.clone())
                }
            }
            SelectionUnit::Paragraph => Some(node.selection_plain_text.replace('\n', " ")),
            SelectionUnit::Section => {
                let section = self
                    .index
                    .sections
                    .iter()
                    .find(|s| s.start_node_idx == node_idx)?;
                let mut parts: Vec<String> = Vec::new();
                for i in section.start_node_idx..=section.end_node_idx {
                    if let Some(n) = self.index.nodes.get(i)
                        && !n.selection_plain_text.is_empty()
                    {
                        parts.push(n.selection_plain_text.replace('\n', " "));
                    }
                }
                Some(parts.join(" "))
            }
        }
    }

    // ── Annotations ───────────────────────────────────────────────────────────

    fn existing_change_for_cursor(&self) -> Option<usize> {
        let changes = self.changes.get(&self.selection_state.anchor.node_idx)?;
        // Sentence-keyed match only fires in Sentence mode; in any other
        // unit the unit_idx isn't a sentence index. The fallback returns
        // the most recent change on the node, which keeps `c` -> edit
        // working when the user is on a node that has any existing change.
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self.current_sentence_context().map(|(idx, _)| idx)
        {
            changes.iter().rposition(|c| c.sentence_index == Some(idx))
        } else {
            changes.len().checked_sub(1)
        }
    }

    fn existing_feedback_for_cursor(&self) -> Option<usize> {
        let feedbacks = self.feedbacks.get(&self.selection_state.anchor.node_idx)?;
        if self.selection_state.anchor.unit == SelectionUnit::Sentence
            && let Some(idx) = self.current_sentence_context().map(|(idx, _)| idx)
        {
            feedbacks
                .iter()
                .rposition(|f| f.sentence_index == Some(idx))
        } else {
            feedbacks.len().checked_sub(1)
        }
    }

    fn begin_change_or_edit(&mut self) {
        if let Some(change_idx) = self.existing_change_for_cursor()
            && let Some(change) = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| changes.get(change_idx))
        {
            self.change_buffer = change.change.clone();
            self.input_mode =
                InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Change;
        self.change_buffer.clear();
        self.status = "Change mode: type text and press Enter. Esc cancels.".to_string();
    }

    fn begin_feedback_or_edit(&mut self) {
        if let Some(feedback_idx) = self.existing_feedback_for_cursor()
            && let Some(feedback) = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| feedbacks.get(feedback_idx))
        {
            self.feedback_buffer = feedback.feedback.clone();
            self.input_mode =
                InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
            self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            return;
        }
        self.input_mode = InputMode::Feedback;
        self.feedback_buffer.clear();
        self.status = "Feedback mode: type text and press Enter. Esc cancels.".to_string();
    }

    fn begin_edit_annotation(&mut self) {
        match self.editable_annotation_at_cursor() {
            Some(EditableAnnotation::Change(change_idx)) => {
                let Some(change) = self
                    .changes
                    .get(&self.selection_state.anchor.node_idx)
                    .and_then(|changes| changes.get(change_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.change_buffer = change.change.clone();
                self.input_mode =
                    InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let Some(feedback) = self
                    .feedbacks
                    .get(&self.selection_state.anchor.node_idx)
                    .and_then(|feedbacks| feedbacks.get(feedback_idx))
                else {
                    self.status = "No change or feedback to edit on this node.".to_string();
                    return;
                };
                self.feedback_buffer = feedback.feedback.clone();
                self.input_mode =
                    InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
                self.status = "Edit mode: Enter saves, Esc cancels.".to_string();
            }
            None => {
                self.status = "No change or feedback to edit on this node.".to_string();
            }
        }
    }

    fn pick_editable_annotation<'a>(
        change: Option<(usize, &'a ChangeAnnotation)>,
        feedback: Option<(usize, &'a FeedbackAnnotation)>,
    ) -> Option<EditableAnnotation> {
        match (change, feedback) {
            (Some((change_idx, change)), Some((feedback_idx, feedback))) => {
                if change.created_at >= feedback.created_at {
                    Some(EditableAnnotation::Change(change_idx))
                } else {
                    Some(EditableAnnotation::Feedback(feedback_idx))
                }
            }
            (Some((change_idx, _)), None) => Some(EditableAnnotation::Change(change_idx)),
            (None, Some((feedback_idx, _))) => Some(EditableAnnotation::Feedback(feedback_idx)),
            (None, None) => None,
        }
    }

    fn editable_annotation_at_cursor(&self) -> Option<EditableAnnotation> {
        // Sentence-keyed match only fires in Sentence mode; in any other
        // unit the unit_idx is not a sentence index. Mirrors the same
        // gate applied in existing_change/feedback_for_cursor.
        let sentence_idx = if self.selection_state.anchor.unit == SelectionUnit::Sentence {
            self.current_sentence_context().map(|(idx, _)| idx)
        } else {
            None
        };

        let sentence_match = sentence_idx.and_then(|idx| {
            let change = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| {
                    changes
                        .iter()
                        .rposition(|c| c.sentence_index == Some(idx))
                        .map(|change_idx| (change_idx, &changes[change_idx]))
                });
            let feedback = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| {
                    feedbacks
                        .iter()
                        .rposition(|f| f.sentence_index == Some(idx))
                        .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
                });
            Self::pick_editable_annotation(change, feedback)
        });

        sentence_match.or_else(|| {
            let change = self
                .changes
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|changes| {
                    changes
                        .len()
                        .checked_sub(1)
                        .map(|change_idx| (change_idx, &changes[change_idx]))
                });
            let feedback = self
                .feedbacks
                .get(&self.selection_state.anchor.node_idx)
                .and_then(|feedbacks| {
                    feedbacks
                        .len()
                        .checked_sub(1)
                        .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
                });
            Self::pick_editable_annotation(change, feedback)
        })
    }

    fn remove_selected_annotation(&mut self) -> bool {
        let node_idx = self.selection_state.anchor.node_idx;
        match self.editable_annotation_at_cursor() {
            Some(EditableAnnotation::Change(change_idx)) => {
                let Some(changes) = self.changes.get_mut(&node_idx) else {
                    return false;
                };
                if change_idx >= changes.len() {
                    return false;
                }
                changes.remove(change_idx);
                if changes.is_empty() {
                    self.changes.remove(&node_idx);
                }
                self.status = format!("Removed change from node {}.", node_idx + 1);
                true
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let Some(feedbacks) = self.feedbacks.get_mut(&node_idx) else {
                    return false;
                };
                if feedback_idx >= feedbacks.len() {
                    return false;
                }
                feedbacks.remove(feedback_idx);
                if feedbacks.is_empty() {
                    self.feedbacks.remove(&node_idx);
                }
                self.status = format!("Removed feedback from node {}.", node_idx + 1);
                true
            }
            None => false,
        }
    }

    fn handle_edit_change_key(&mut self, key: KeyEvent, node_idx: usize, change_idx: usize) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
                self.status = "Edit cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.change_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Edit ignored — change cannot be empty.".to_string();
                } else if let Some(changes) = self.changes.get_mut(&node_idx)
                    && let Some(annotation) = changes.get_mut(change_idx)
                {
                    annotation.change = trimmed;
                    self.status = format!("Change updated on node {}.", node_idx + 1);
                }
                self.input_mode = InputMode::Normal;
                self.change_buffer.clear();
            }
            KeyCode::Backspace => {
                self.change_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.change_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn handle_edit_feedback_key(&mut self, key: KeyEvent, node_idx: usize, feedback_idx: usize) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
                self.status = "Edit cancelled.".to_string();
            }
            KeyCode::Enter => {
                let trimmed = self.feedback_buffer.trim().to_string();
                if trimmed.is_empty() {
                    self.status = "Edit ignored — feedback cannot be empty.".to_string();
                } else if let Some(feedbacks) = self.feedbacks.get_mut(&node_idx)
                    && let Some(annotation) = feedbacks.get_mut(feedback_idx)
                {
                    annotation.feedback = trimmed;
                    self.status = format!("Feedback updated on node {}.", node_idx + 1);
                }
                self.input_mode = InputMode::Normal;
                self.feedback_buffer.clear();
            }
            KeyCode::Backspace => {
                self.feedback_buffer.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.feedback_buffer.push(ch);
            }
            _ => {}
        }
    }

    fn toggle_strike(&mut self) {
        if self.remove_selected_annotation() {
            return;
        }

        let unit = self.selection_state.anchor.unit;
        let unit_idx = self.selection_state.anchor.unit_idx;
        let node_idx = self.selection_state.anchor.node_idx;

        // Verify the active anchor actually points at a real unit on this
        // node — otherwise an empty paragraph or out-of-range word index
        // would create a phantom strike.
        if self.current_target_capture().is_none() {
            self.status = format!(
                "Node {} has no {} target to strike.",
                node_idx + 1,
                unit.mode_str()
            );
            return;
        }

        let key = (unit, unit_idx);
        let entry = self.strikes.entry(node_idx).or_default();
        let unit_label = unit.mode_str();
        if entry.contains(&key) {
            entry.remove(&key);
            if entry.is_empty() {
                self.strikes.remove(&node_idx);
            }
            self.status = format!(
                "Removed strike from node {} ({unit_label} {}).",
                node_idx + 1,
                unit_idx + 1
            );
        } else {
            entry.insert(key);
            self.status = format!(
                "Struck node {} ({unit_label} {}).",
                node_idx + 1,
                unit_idx + 1
            );
        }
    }

    fn reveal_links_for_current_sentence(&mut self) -> bool {
        let urls = self.current_sentence_links();
        if urls.is_empty() {
            return false;
        }
        let count = urls.len();
        self.link_popup_urls = Some(urls);
        self.status = format!("Showing {count} link(s) from current sentence.");
        true
    }

    fn current_sentence_links(&self) -> Vec<String> {
        let Some(rn) = self
            .rendered_nodes
            .get(self.selection_state.anchor.node_idx)
        else {
            return Vec::new();
        };
        // In Sentence mode, scope to the current sentence's byte range.
        // In any other mode, fall back to "all links in the current node"
        // — the unit_idx is a Word/Line/Paragraph/Section index that
        // doesn't translate cleanly to sentence_ranges.
        let scope: Option<Range<usize>> =
            if self.selection_state.anchor.unit == SelectionUnit::Sentence {
                rn.sentence_ranges
                    .get(self.selection_state.anchor.unit_idx)
                    .cloned()
            } else {
                None
            };
        let mut urls = Vec::new();
        for link in &rn.links {
            let overlaps = scope
                .as_ref()
                .is_none_or(|r| link.end > r.start && link.start < r.end);
            if overlaps && !urls.iter().any(|u: &String| u == &link.url) {
                urls.push(link.url.clone());
            }
        }
        urls
    }

    fn annotation_counts(&self) -> (usize, usize, usize, usize) {
        let changes: usize = self.changes.values().map(|v| v.len()).sum();
        let feedbacks: usize = self.feedbacks.values().map(|v| v.len()).sum();
        let inserts: usize = self.inserts_before.values().map(|v| v.len()).sum::<usize>()
            + self.inserts_after.values().map(|v| v.len()).sum::<usize>();
        let strikes: usize = self.strikes.values().map(|v| v.len()).sum();
        (changes, feedbacks, inserts, strikes)
    }

    // ── Drawing ───────────────────────────────────────────────────────────────

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(FOOTER_HEIGHT)])
            .split(area);
        let line_area_inner_width = layout[0].width.saturating_sub(2) as usize;
        let wrapped_text_width = line_area_inner_width.saturating_sub(GUTTER_WIDTH).max(1);

        let filename = self
            .source_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("markdown");
        let (change_count, feedback_count, insert_count, strike_count) = self.annotation_counts();
        let block_title =
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
            };

        let list_block = Block::default()
            .borders(Borders::ALL)
            .title(block_title)
            .border_style(Style::default().fg(Color::Gray));
        let list_inner = list_block.inner(layout[0]);
        self.list_inner = list_inner;

        let mut node_heights: Vec<u16> = Vec::with_capacity(self.doc.node_count());

        let node_count = self.doc.node_count();
        let node_lines: Vec<Vec<Line<'static>>> = self
            .doc
            .nodes
            .iter()
            .enumerate()
            .map(|(node_idx, node)| {
                let (indicator, indicator_style) = self.node_indicator(node_idx);
                // Add a blank trailing line when the NEXT node is a block start.
                // This keeps the spacer at the END of the preceding item so that
                // navigating to any node always shows content as the first line.
                let add_spacer_after =
                    node_idx + 1 < node_count && self.doc.is_block_start(node_idx + 1);

                // Code blocks render line-by-line without sentence wrap logic.
                if let DocNode::CodeBlock {
                    source_lines: range,
                    ..
                } = node
                {
                    let raw = &self.source_lines[clamp_range(range, self.source_lines.len())];
                    let range_start = range.start;
                    let mut display_lines: Vec<Line> = raw
                        .iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let base_style = if line.trim_start().starts_with("```") {
                                Style::default().fg(Color::DarkGray)
                            } else {
                                Style::default().fg(Color::White).bg(Color::DarkGray)
                            };
                            let mut spans = vec![if i == 0 {
                                Span::styled(format!("{indicator} "), indicator_style)
                            } else {
                                Span::raw("  ")
                            }];
                            // Overlay highlight + strikes on this source
                            // line by mapping the active anchor's
                            // selection-view byte range (and each strike
                            // range) into bytes within `line`. Without
                            // this, code blocks rendered with no visible
                            // cursor — the special draw path used to
                            // bypass render_node_spans entirely so
                            // word-mode highlight on a fenced code block
                            // (e.g. YAML frontmatter folded as a
                            // CodeBlock) showed nothing.
                            self.push_codeblock_line_spans(
                                &mut spans,
                                node_idx,
                                range_start + i,
                                line,
                                base_style,
                            );
                            Line::from(spans)
                        })
                        .collect();
                    if add_spacer_after {
                        display_lines.push(Line::from(""));
                    }
                    let height = display_lines.len().max(1) as u16;
                    node_heights.push(height);
                    return display_lines;
                }

                let spans = self.render_node_spans(node_idx);
                let wrapped = wrap_styled_spans(spans, wrapped_text_width);

                let mut wrapped_lines: Vec<Line> = wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(seg_idx, mut seg)| {
                        let mut line_spans = Vec::new();
                        if seg_idx == 0 {
                            line_spans.push(Span::styled(format!("{indicator} "), indicator_style));
                        } else {
                            line_spans.push(Span::raw("  "));
                        }
                        line_spans.append(&mut seg);
                        Line::from(line_spans)
                    })
                    .collect();

                if add_spacer_after {
                    wrapped_lines.push(Line::from(""));
                }
                let height = wrapped_lines.len().max(1) as u16;
                node_heights.push(height);

                wrapped_lines
            })
            .collect();

        self.cached_node_heights = node_heights;
        self.adjust_scroll(list_inner.height);
        self.fill_partial_bottom(list_inner.height);

        // Render block border, then manually render lines so partial items are
        // clipped at the bottom rather than skipped entirely.
        frame.render_widget(list_block, layout[0]);
        let mut visible: Vec<Line<'static>> = Vec::new();
        let mut count = 0u16;
        'outer: for lines in node_lines.iter().skip(self.scroll_offset) {
            for line in lines {
                if count >= list_inner.height {
                    break 'outer;
                }
                visible.push(line.clone());
                count += 1;
            }
        }
        frame.render_widget(Paragraph::new(Text::from(visible)), list_inner);

        // Two-zone footer: persistent left mode indicator + transient right
        // zone (nav feedback / notification / hint). Mode indicator is never
        // truncated; right zone shrinks first under width pressure.
        let mode_text = format!(" mode: {}", self.mode_indicator());
        let mode_style = Style::default().fg(Color::Cyan);
        let hint_style = Style::default().fg(Color::DarkGray);
        // Right zone priority: transient nav_feedback (one-keypress
        // boundary message) > transient notification (clipboard result) >
        // persistent status (current mode / last action context) > help
        // hint. The status field accumulates input-mode prompts and
        // action-confirmation messages; without showing it the user never
        // sees mode prompts like "Change mode: type text and press Enter."
        let right_text = if let Some(fb) = &self.nav_feedback {
            (fb.clone(), Style::default().fg(Color::Yellow))
        } else if let Some(note) = &self.notification {
            (note.clone(), Style::default().fg(Color::Green))
        } else if !self.status.is_empty() {
            (self.status.clone(), Style::default().fg(Color::Gray))
        } else {
            ("? for help ".to_string(), hint_style)
        };
        // Account for terminal column width (not byte length) so user-supplied
        // text in the right zone — e.g. a search query containing CJK or
        // emoji — doesn't underestimate the gap and squish the footer when
        // the message contains wide-width characters.
        let total_width = layout[1].width as usize;
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
        frame.render_widget(Paragraph::new(footer_line), layout[1]);

        let popup_spec: Option<(&str, &str, &str, &str)> = match &self.input_mode {
            InputMode::Change => Some((
                " Change ",
                "Change mode: Enter save | Esc cancel",
                "Change> ",
                self.change_buffer.as_str(),
            )),
            InputMode::EditChange(..) => Some((
                " Edit Change ",
                "Edit mode: Enter save | Esc cancel",
                "Change> ",
                self.change_buffer.as_str(),
            )),
            InputMode::Feedback => Some((
                " Feedback ",
                "Feedback mode: Enter save | Esc cancel",
                "Feedback> ",
                self.feedback_buffer.as_str(),
            )),
            InputMode::EditFeedback(..) => Some((
                " Edit Feedback ",
                "Edit mode: Enter save | Esc cancel",
                "Feedback> ",
                self.feedback_buffer.as_str(),
            )),
            InputMode::InsertBefore => Some((
                " Insert Before ",
                "Insert before: Enter save | Esc cancel",
                "Before> ",
                self.insert_buffer.as_str(),
            )),
            InputMode::InsertAfter => Some((
                " Insert After ",
                "Insert after: Enter save | Esc cancel",
                "After> ",
                self.insert_buffer.as_str(),
            )),
            InputMode::Search => Some((
                " Search ",
                "Search: Enter jump | Esc cancel | n/N next/prev",
                "/",
                self.search_buffer.as_str(),
            )),
            InputMode::Normal => None,
        };
        if let Some((title, hint, prompt, buf)) = popup_spec {
            self.draw_input_popup(frame, list_inner, title, hint, prompt, buf);
        }

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

    fn draw_input_popup(
        &self,
        frame: &mut Frame,
        list_inner: Rect,
        title: &str,
        hint: &str,
        prompt: &str,
        buf: &str,
    ) {
        let heights = &self.cached_node_heights;
        if list_inner.width < 12
            || list_inner.height < 4
            || self.selection_state.anchor.node_idx >= heights.len()
        {
            return;
        }

        let list_offset = self.scroll_offset;
        if self.selection_state.anchor.node_idx < list_offset {
            return;
        }

        let selected_top: u16 = heights
            .iter()
            .skip(list_offset)
            .take(self.selection_state.anchor.node_idx - list_offset)
            .copied()
            .sum();
        let selected_height = heights[self.selection_state.anchor.node_idx].max(1);

        if selected_top >= list_inner.height {
            return;
        }

        let popup_width = list_inner.width.clamp(20, 80);
        let inner_width = popup_width.saturating_sub(2) as usize;

        let hint_height =
            wrap_styled_spans(vec![Span::raw(hint.to_owned())], inner_width).len() as u16;
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

    fn draw_help(frame: &mut Frame, area: Rect) {
        let help_lines = vec![
            Line::from(Span::styled(
                "  nav  next/prev",
                Style::default().fg(Color::Cyan),
            )),
            Line::from(
                "  i / u               cycle unit finer / coarser (section ↔ paragraph ↔ line ↔ sentence ↔ word)",
            ),
            Line::from("  Space / Backspace   synonyms for i / u"),
            Line::from("  j / k               next / prev anchor in active unit"),
            Line::from("  ↓ / ↑ / → / ←       synonyms for j / k"),
            Line::from("  ]/[                 next/prev annotation"),
            Line::from(""),
            Line::from("  I  AST view        U  links"),
            Line::from("  /  search          n/N  next/prev match"),
            Line::from("  c  change (literal)"),
            Line::from("  f  feedback (intent)"),
            Line::from("  b  a  insert before · after"),
            Line::from("  e  edit existing change/feedback"),
            Line::from("  x  clear annotation, or strike (sentence mode only)"),
            Line::from("  r  copy result to clipboard"),
            Line::from("  q  Q          quit · silent quit"),
            Line::from("  ? / Esc       help · close"),
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
        let popup_width = (content_width + 2).min(area.width);
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

    fn draw_ast_popup(&self, frame: &mut Frame, area: Rect) {
        let popup_width = (area.width * 4 / 5).max(40).min(area.width);
        let popup_height = (area.height * 4 / 5).max(6).min(area.height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(popup_width) / 2,
            y: area.y + area.height.saturating_sub(popup_height) / 2,
            width: popup_width,
            height: popup_height,
        };

        let lines: Vec<Line> = self
            .ast_lines
            .iter()
            .map(|l| Line::from(Span::raw(l.clone())))
            .collect();

        let total = self.ast_lines.len() as u16;
        let inner_height = popup_height.saturating_sub(2);
        let max_scroll = total.saturating_sub(inner_height);
        let scroll = self.ast_view_scroll.unwrap_or(0).min(max_scroll);

        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::default()
                        .title(format!(
                            " AST  [{}/{}]  j/k scroll · i/Esc close ",
                            scroll + 1,
                            total
                        ))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                )
                .scroll((scroll, 0)),
            popup,
        );
    }

    fn draw_link_popup(&self, frame: &mut Frame, area: Rect) {
        // Caller in draw() gates on link_popup_urls.is_some(), so the
        // None case here is unreachable; default to an empty slice if
        // it ever fires.
        let urls: &[String] = self.link_popup_urls.as_deref().unwrap_or(&[]);
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

    fn adjust_scroll(&mut self, inner_height: u16) {
        let heights = &self.cached_node_heights;
        if heights.is_empty() {
            return;
        }
        let n = heights.len();
        self.scroll_offset = self.scroll_offset.min(n.saturating_sub(1));

        if self.selection_state.anchor.node_idx < self.scroll_offset {
            self.scroll_offset = self.selection_state.anchor.node_idx;
            return;
        }

        let cursor_height = heights
            .get(self.selection_state.anchor.node_idx)
            .copied()
            .unwrap_or(1);
        let rows_before: u16 = heights
            .get(self.scroll_offset..self.selection_state.anchor.node_idx)
            .map_or(0, |s| s.iter().copied().sum());

        // Cursor is fully visible — nothing to do.
        if rows_before + cursor_height <= inner_height {
            return;
        }

        // Cursor extends past the bottom (or is entirely off-screen). Reposition so
        // the cursor's bottom aligns with the screen bottom, maximising the number of
        // cursor lines shown. If the cursor is taller than the screen, put it at top.
        let target_start = inner_height.saturating_sub(cursor_height);

        let mut new_offset = self.selection_state.anchor.node_idx;
        let mut cum: u16 = 0;
        for i in (0..self.selection_state.anchor.node_idx).rev() {
            let h = heights.get(i).copied().unwrap_or(0);
            if cum + h > target_start {
                break;
            }
            cum += h;
            new_offset = i;
        }
        self.scroll_offset = new_offset;
    }

    /// After cursor positioning, pull any item that is partially visible at the
    /// bottom fully into view — as long as the cursor node remains visible.
    ///
    /// This covers the case where the cursor is on node N and node N+1 (or later)
    /// is partially clipped at the bottom: we scroll forward enough to show the
    /// partial item fully, provided the cursor itself stays in view.
    fn fill_partial_bottom(&mut self, inner_height: u16) {
        let heights = &self.cached_node_heights;
        if heights.is_empty() {
            return;
        }

        // Find the first partially-visible item at the bottom of the current view.
        let mut cum: u16 = 0;
        let mut partial: Option<(usize, u16)> = None;
        for (i, &h) in heights.iter().enumerate().skip(self.scroll_offset) {
            if cum + h > inner_height {
                partial = Some((i, h));
                break;
            }
            cum += h;
        }

        let (partial_idx, partial_h) = match partial {
            Some(p) if p.1 <= inner_height => p, // only handle items that can fit fully
            _ => return,
        };

        if partial_idx <= self.selection_state.anchor.node_idx {
            return; // cursor-based logic already handles this
        }

        // How many rows do we need to free up above to show the partial item fully?
        // Currently the item starts at `cum`; we need it at `inner_height - partial_h`.
        let needed = cum.saturating_sub(inner_height - partial_h);
        if needed == 0 {
            return;
        }

        // Try to advance scroll_offset by `needed` rows, while keeping cursor visible.
        let cursor_h = heights
            .get(self.selection_state.anchor.node_idx)
            .copied()
            .unwrap_or(1);
        let mut skipped: u16 = 0;
        let mut new_offset = self.scroll_offset;
        for i in self.scroll_offset..partial_idx {
            let h = heights.get(i).copied().unwrap_or(0);
            if skipped + h > needed {
                break;
            }
            // Verify cursor stays visible after advancing past item i.
            let candidate = i + 1;
            if candidate > self.selection_state.anchor.node_idx {
                break; // would push cursor above offset
            }
            let rows_before_cursor: u16 = heights
                .get(candidate..self.selection_state.anchor.node_idx)
                .map_or(0, |s| s.iter().copied().sum());
            if rows_before_cursor + cursor_h > inner_height {
                break; // cursor would go off-screen
            }
            skipped += h;
            new_offset = candidate;
        }
        self.scroll_offset = new_offset;
    }

    fn node_indicator(&self, node_idx: usize) -> (&'static str, Style) {
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

    /// Compute the byte range in the rendered display plain text that the
    /// active selection unit should paint. Returns None when the active
    /// anchor doesn't resolve to a paintable range on this node (the
    /// section_highlight_range path covers Section selections).
    /// Active-anchor variant: maps `selection_state.anchor` to a display
    /// byte range. Thin wrapper over `unit_byte_range_in_display`.
    fn unit_highlight_for(
        &self,
        node_idx: usize,
        plain: &str,
        sentence_ranges: &[Range<usize>],
    ) -> Option<Range<usize>> {
        self.unit_byte_range_in_display(
            node_idx,
            self.selection_state.anchor.unit,
            self.selection_state.anchor.unit_idx,
            plain,
            sentence_ranges,
        )
    }

    /// Map any `(unit, unit_idx)` on `node_idx` to a byte range inside the
    /// node's display plain text, so strike rendering and live-anchor
    /// highlight can share the same mapping logic. Section returns None
    /// (sections paint whole-node ranges, handled by section_highlight_range
    /// in the caller).
    fn unit_byte_range_in_display(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
        plain: &str,
        sentence_ranges: &[Range<usize>],
    ) -> Option<Range<usize>> {
        match unit {
            SelectionUnit::Sentence => sentence_ranges.get(unit_idx).cloned(),
            SelectionUnit::Paragraph => Some(0..plain.len()),
            SelectionUnit::Line => {
                // Locate the line by its source line number from the index,
                // then find that source line's text inside display plain.
                // Re-segmenting display by `\n` and picking the unit_idx-th
                // segment drifts on fenced code blocks (where fence lines
                // are present in display but excluded from index lines).
                // Count which occurrence in source so two identical lines
                // map to the correct display position.
                let index_node = self.index.nodes.get(node_idx)?;
                let (line, _) = index_node.source_line_ranges.get(unit_idx)?;
                let line_text = self.source_lines.get(*line)?;
                if line_text.is_empty() {
                    return Some(0..plain.len());
                }
                let occurrence = index_node
                    .source_line_ranges
                    .iter()
                    .take(unit_idx)
                    .filter(|(l, _)| {
                        self.source_lines.get(*l).map(|s| s.as_str()) == Some(line_text.as_str())
                    })
                    .count();
                let pos = nth_occurrence(plain, line_text.as_str(), occurrence).unwrap_or(0);
                Some(pos..pos + line_text.len())
            }
            SelectionUnit::Word => {
                // Locate the index's word text in the rendered display plain
                // text. Re-segmenting display text with segment_words can
                // drift when markers (footnote refs, task markers, etc.)
                // appear in display but not in selection plain text — the
                // word index aligns to selection plain text, not display.
                // Count which occurrence the word is in selection plain
                // text so repeated words map to the right display position.
                let index_node = self.index.nodes.get(node_idx)?;
                let word_range = index_node.word_ranges.get(unit_idx)?;
                let word_text = index_node.selection_plain_text.get(word_range.clone())?;
                let occurrence = count_occurrences_before(
                    &index_node.selection_plain_text,
                    word_text,
                    word_range.start,
                );
                let pos = nth_occurrence(plain, word_text, occurrence)?;
                Some(pos..pos + word_text.len())
            }
            SelectionUnit::Section => None,
        }
    }

    /// Push styled span(s) for one source line of a code block, overlaying
    /// the active highlight and any strike ranges that intersect this line.
    /// `node_idx` identifies the code block; `source_line` is the absolute
    /// line index in `self.source_lines`; `line` is its raw text;
    /// `base_style` is the code-block paint (DarkGray fence vs
    /// White-on-DarkGray content). The active anchor and strike entries
    /// store byte ranges in selection_plain_text — we map each into bytes
    /// within `line` via the index's source_line_ranges table, then split
    /// the span at the overlap so the highlight paints precisely.
    fn push_codeblock_line_spans(
        &self,
        spans: &mut Vec<Span<'static>>,
        node_idx: usize,
        source_line: usize,
        line: &str,
        base_style: Style,
    ) {
        // Translate a (selection-view) byte range to a (line-local)
        // byte range when the active node's source_line_ranges has an
        // entry for this source line.
        let line_local = |range: Range<usize>| -> Option<Range<usize>> {
            let node = self.index.nodes.get(node_idx)?;
            let (_, line_range) = node
                .source_line_ranges
                .iter()
                .find(|(l, _)| *l == source_line)?;
            if range.end <= line_range.start || range.start >= line_range.end {
                return None;
            }
            let start = range.start.max(line_range.start) - line_range.start;
            let end = range.end.min(line_range.end) - line_range.start;
            // Don't paint a zero-width highlight on an empty intersection.
            if end <= start {
                return None;
            }
            Some(start..end)
        };

        // Active anchor → highlight range on this line. Section-mode
        // whole-node highlight is handled separately below.
        let highlight_local = if self
            .section_highlight_range
            .as_ref()
            .is_some_and(|r| r.contains(&node_idx))
        {
            Some(0..line.len())
        } else if node_idx == self.selection_state.anchor.node_idx {
            let unit = self.selection_state.anchor.unit;
            let unit_idx = self.selection_state.anchor.unit_idx;
            // Use the index's selection-view byte ranges directly so we
            // don't double-walk through unit_byte_range_in_display
            // (which is for paragraph-style display plain text).
            let node = self.index.nodes.get(node_idx);
            let active_range = node.and_then(|n| match unit {
                SelectionUnit::Word => n.word_ranges.get(unit_idx).cloned(),
                SelectionUnit::Sentence => n.sentence_ranges.get(unit_idx).cloned(),
                SelectionUnit::Line => n.source_line_ranges.get(unit_idx).map(|(_, r)| r.clone()),
                SelectionUnit::Paragraph => Some(0..n.selection_plain_text.len()),
                SelectionUnit::Section => None,
            });
            active_range.and_then(line_local)
        } else {
            None
        };

        // Strike ranges on this line.
        let strike_local: Vec<Range<usize>> = self
            .strikes
            .get(&node_idx)
            .map(|set| {
                set.iter()
                    .filter_map(|&(unit, idx)| {
                        let node = self.index.nodes.get(node_idx)?;
                        let r = match unit {
                            SelectionUnit::Word => node.word_ranges.get(idx).cloned()?,
                            SelectionUnit::Sentence => node.sentence_ranges.get(idx).cloned()?,
                            SelectionUnit::Line => {
                                node.source_line_ranges.get(idx).map(|(_, r)| r.clone())?
                            }
                            SelectionUnit::Paragraph => 0..node.selection_plain_text.len(),
                            SelectionUnit::Section => return None,
                        };
                        line_local(r)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Build segment boundaries.
        let mut bounds = vec![0, line.len()];
        if let Some(r) = &highlight_local {
            bounds.push(r.start);
            bounds.push(r.end);
        }
        for r in &strike_local {
            bounds.push(r.start);
            bounds.push(r.end);
        }
        bounds.sort_unstable();
        bounds.dedup();

        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let Some(text) = line.get(start..end) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let mut style = base_style;
            if highlight_local
                .as_ref()
                .is_some_and(|r| start < r.end && end > r.start)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }
            if strike_local.iter().any(|r| start < r.end && end > r.start) {
                style = style.patch(
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
                );
            }
            spans.push(Span::styled(text.to_string(), style));
        }

        if spans.is_empty() {
            spans.push(Span::styled(line.to_string(), base_style));
        }
    }

    fn render_node_spans(&self, node_idx: usize) -> Vec<Span<'static>> {
        let Some(rn) = self.rendered_nodes.get(node_idx) else {
            return vec![Span::styled(
                " ",
                Style::default().add_modifier(Modifier::DIM),
            )];
        };
        let plain = rn.plain.as_str();
        let plain_len = plain.len();

        if plain.is_empty() {
            return vec![Span::styled(
                " ",
                Style::default().add_modifier(Modifier::DIM),
            )];
        }

        let sentence_ranges = &rn.sentence_ranges;

        // Resolve every strike anchor on this node to a display byte
        // range. Empty when nothing is struck. Sentence-unit strikes use
        // the existing rn.sentence_ranges; Word/Line/Paragraph strikes
        // route through unit_byte_range_in_display so the painted span
        // matches what the user struck.
        let strike_ranges: Vec<Range<usize>> = self
            .strikes
            .get(&node_idx)
            .map(|set| {
                set.iter()
                    .filter_map(|&(unit, idx)| {
                        self.unit_byte_range_in_display(node_idx, unit, idx, plain, sentence_ranges)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Map span style → byte range segments.
        let mut seg: Vec<(usize, usize, Style)> = Vec::new();
        let mut offset = 0usize;
        for span in &rn.spans {
            let len = span.content.len();
            if len == 0 {
                continue;
            }
            let end = (offset + len).min(plain_len);
            if offset < end {
                seg.push((offset, end, span.style));
            }
            offset = end;
        }
        if seg.is_empty() {
            seg.push((0, plain_len, Style::default()));
        }

        let highlight = if self
            .section_highlight_range
            .as_ref()
            .is_some_and(|r| r.contains(&node_idx))
        {
            Some(0..plain_len)
        } else if node_idx == self.selection_state.anchor.node_idx {
            self.unit_highlight_for(node_idx, plain, sentence_ranges)
        } else {
            None
        };

        // Collect all split points. Include the active highlight boundaries so
        // sub-node units (Word/Line) can paint precisely rather than tinting an
        // entire pre-existing span chunk. Same for each strike range so
        // word-unit strikes paint just the word.
        let mut bounds = vec![0, plain_len];
        for &(s, e, _) in &seg {
            bounds.push(s);
            bounds.push(e);
        }
        for r in sentence_ranges {
            bounds.push(r.start.min(plain_len));
            bounds.push(r.end.min(plain_len));
        }
        if let Some(r) = &highlight {
            bounds.push(r.start.min(plain_len));
            bounds.push(r.end.min(plain_len));
        }
        for r in &strike_ranges {
            bounds.push(r.start.min(plain_len));
            bounds.push(r.end.min(plain_len));
        }
        bounds.sort_unstable();
        bounds.dedup();

        let mut spans = Vec::new();
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            if start >= end {
                continue;
            }
            let Some(text) = plain.get(start..end) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }

            let mut style = seg
                .iter()
                .find(|&&(s, e, _)| start >= s && start < e)
                .map(|&(_, _, sty)| sty)
                .unwrap_or_default();

            if highlight
                .as_ref()
                .is_some_and(|r| start < r.end && end > r.start)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }

            if strike_ranges.iter().any(|r| start < r.end && end > r.start) {
                style = style.patch(
                    Style::default()
                        .fg(Color::Red)
                        .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM),
                );
            }

            spans.push(Span::styled(text.to_string(), style));
        }

        if spans.is_empty() {
            spans.push(Span::raw(plain.to_string()));
        }
        spans
    }

    // ── Output ────────────────────────────────────────────────────────────────

    #[cfg(test)]
    fn to_output(&self) -> AgentOutput {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());

        let mut annotations = Vec::new();
        for node_idx in touched {
            let (source_line, line_text) = self.node_line_context(node_idx);

            let previous_line = source_line
                .checked_sub(1)
                .and_then(|i| self.source_lines.get(i))
                .cloned();
            let next_line = self.source_lines.get(source_line + 1).cloned();

            let changes = self
                .changes
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| ChangeOutput {
                    created_at: a.created_at,
                    sentence_index: a.sentence_index.map(|i| i + 1),
                    sentence_text: a.sentence_text,
                    change: a.change,
                })
                .collect();

            let feedbacks = self
                .feedbacks
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| FeedbackOutput {
                    created_at: a.created_at,
                    sentence_index: a.sentence_index.map(|i| i + 1),
                    sentence_text: a.sentence_text,
                    feedback: a.feedback,
                })
                .collect();

            let map_inserts = |bucket: Option<&Vec<InsertAnnotation>>| -> Vec<InsertOutput> {
                bucket
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| InsertOutput {
                        created_at: a.created_at,
                        sentence_index: a.sentence_index.map(|i| i + 1),
                        sentence_text: a.sentence_text,
                        text: a.text,
                    })
                    .collect()
            };
            let inserts_before = map_inserts(self.inserts_before.get(&node_idx));
            let inserts_after = map_inserts(self.inserts_after.get(&node_idx));

            let reactions = self
                .strikes
                .get(&node_idx)
                .map(|set| {
                    set.iter()
                        .map(|&(unit, idx)| {
                            let target_text = self
                                .target_text_for_unit(node_idx, unit, idx)
                                .unwrap_or_default();
                            ReactionOutput {
                                kind: "strike".to_string(),
                                target_unit: unit.mode_str().to_string(),
                                unit_index: idx + 1,
                                target_text,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            annotations.push(LineAnnotationOutput {
                line_number: source_line + 1,
                line_text: line_text.clone(),
                context: LineContext {
                    previous_line,
                    current_line: line_text,
                    next_line,
                },
                changes,
                feedbacks,
                inserts_before,
                inserts_after,
                reactions,
            });
        }

        AgentOutput {
            source_file: self.source_path.display().to_string(),
            generated_at: Utc::now().to_rfc3339(),
            keymap: KeymapOutput {
                mode_cycle_forward: "i".to_string(),
                mode_cycle_backward: "u".to_string(),
                unit_next: "j".to_string(),
                unit_prev: "k".to_string(),
                reveal_link: "U".to_string(),
                annotation_prev: "[".to_string(),
                annotation_next: "]".to_string(),
                help: "?".to_string(),
                change: "c".to_string(),
                feedback: "f".to_string(),
                insert_before: "b".to_string(),
                insert_after: "a".to_string(),
                strike: "x".to_string(),
                quit: "q".to_string(),
                quit_silent: "Q".to_string(),
            },
            annotations,
        }
    }

    pub fn to_human_output(&self) -> String {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());

        let mut out = String::new();
        let _ = writeln!(out, "FILE: {}", self.source_path.display());

        if touched.is_empty() {
            out.push_str("\nNo actions.\n");
            return out;
        }

        for node_idx in touched {
            let (source_line, line_text) = self.node_line_context(node_idx);
            let line_clean = clean_context(&line_text, EMIT_TARGET_MAX_CHARS);

            if let Some(changes) = self.changes.get(&node_idx) {
                for change in changes {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        "change",
                        "CHANGE",
                        change.target_unit,
                        change.sentence_index,
                        change.sentence_text.as_deref(),
                        &change.change,
                    );
                }
            }

            if let Some(feedbacks) = self.feedbacks.get(&node_idx) {
                for feedback in feedbacks {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        "revise-to-incorporate-feedback",
                        "FEEDBACK",
                        feedback.target_unit,
                        feedback.sentence_index,
                        feedback.sentence_text.as_deref(),
                        &feedback.feedback,
                    );
                }
            }

            for (action, bucket) in [
                ("insert-before", self.inserts_before.get(&node_idx)),
                ("insert-after", self.inserts_after.get(&node_idx)),
            ] {
                let Some(inserts) = bucket else { continue };
                for insert in inserts {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        action,
                        "INSERT",
                        insert.target_unit,
                        insert.sentence_index,
                        insert.sentence_text.as_deref(),
                        &insert.text,
                    );
                }
            }

            if let Some(strikes) = self.strikes.get(&node_idx) {
                for &(unit, unit_idx) in strikes {
                    // Target text comes from the selection plain text
                    // view per Req 11 — same source the index uses for
                    // navigation / sentence emit.
                    let raw_target = self
                        .target_text_for_unit(node_idx, unit, unit_idx)
                        .unwrap_or_else(|| line_clean.clone());
                    let target = clean_context(&raw_target, EMIT_TARGET_MAX_CHARS);
                    let strike_line =
                        self.where_for_annotation(unit, node_idx, Some(unit_idx), source_line);
                    Self::emit_action_header(&mut out, "delete this", strike_line);
                    self.emit_context_block(&mut out, strike_line, &target);
                }
            }
        }

        out
    }

    /// Append a single annotation block (ACTION / WHERE / CONTEXT /
    /// payload) to `out`. Centralizes the change / feedback / insert-* /
    /// emit shape so the block grows in one place. Strikes follow a
    /// different shape (sentence-keyed, no target_unit) so they stay
    /// inline.
    #[allow(clippy::too_many_arguments)]
    fn emit_annotation_block(
        &self,
        out: &mut String,
        node_idx: usize,
        node_first_line: usize,
        line_clean: &str,
        action: &str,
        payload_key: &str,
        target_unit: SelectionUnit,
        sentence_index: Option<usize>,
        sentence_text: Option<&str>,
        payload_text: &str,
    ) {
        let where_line =
            self.where_for_annotation(target_unit, node_idx, sentence_index, node_first_line);
        let target = sentence_text.map_or_else(
            || line_clean.to_owned(),
            |s| clean_context(s, EMIT_TARGET_MAX_CHARS),
        );
        Self::emit_action_header(out, action, where_line);
        self.emit_context_block(out, where_line, &target);
        let _ = writeln!(
            out,
            "{payload_key}: \"{}\"",
            clean_context(payload_text, EMIT_PAYLOAD_MAX_CHARS)
        );
    }

    /// Write `\nACTION: <name>\nWHERE: line N\n` — shared by every
    /// emit shape (changes / feedbacks / inserts / strikes).
    fn emit_action_header(out: &mut String, action: &str, where_line: usize) {
        out.push('\n');
        let _ = writeln!(out, "ACTION: {action}");
        let _ = writeln!(out, "WHERE: line {}", where_line + 1);
    }

    /// Write the CONTEXT block: `CONTEXT:\n  prev: "..." (if any)\n  target: "..."\n  next: "..." (if any)\n`.
    /// Called by every emit shape that writes a CONTEXT section.
    fn emit_context_block(&self, out: &mut String, where_line: usize, target: &str) {
        let (prev_clean_line, next_clean_line) = self.context_lines(where_line);
        out.push_str("CONTEXT:\n");
        if !prev_clean_line.is_empty() {
            let _ = writeln!(out, "  prev: \"{prev_clean_line}\"");
        }
        let _ = writeln!(out, "  target: \"{target}\"");
        if !next_clean_line.is_empty() {
            let _ = writeln!(out, "  next: \"{next_clean_line}\"");
        }
    }

    fn node_line_context(&self, node_idx: usize) -> (usize, String) {
        let source_line = self
            .doc
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(source_line)
            .cloned()
            .unwrap_or_default();
        (source_line, line_text)
    }

    /// Returns the source line where an annotation's selection text
    /// begins: per-line for Line annotations, per-word for Word,
    /// per-sentence for Sentence (computed from the rendered_nodes
    /// display plain text `\n` count), and the node's first line for
    /// Paragraph / Section (those emit their entire span anyway).
    fn where_for_annotation(
        &self,
        target_unit: SelectionUnit,
        node_idx: usize,
        sentence_index: Option<usize>,
        node_first_line: usize,
    ) -> usize {
        match target_unit {
            SelectionUnit::Line => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.index
                    .nodes
                    .get(node_idx)
                    .and_then(|n| n.source_line_ranges.get(unit_idx).map(|p| p.0))
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Word => {
                // Word emits at the source line where the word's bytes begin.
                let unit_idx = sentence_index.unwrap_or(0);
                self.word_source_line(node_idx, unit_idx)
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Sentence => {
                // Sentence inside a multi-source-line node: emit the line
                // where the sentence's text begins, not the node's first
                // line. Computed by counting `\n` characters in the
                // rendered display plain text up to the sentence range's
                // start (mirrors the strike-emit logic above).
                sentence_index
                    .and_then(|si| {
                        let rn = self.rendered_nodes.get(node_idx)?;
                        let r = rn.sentence_ranges.get(si)?;
                        Some(node_first_line + newlines_before_byte(&rn.plain, r.start))
                    })
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Paragraph | SelectionUnit::Section => node_first_line,
        }
    }

    /// Source line where a word's bytes begin within its node's selection
    /// plain text. Maps via the index's `source_line_ranges` table.
    fn word_source_line(&self, node_idx: usize, word_idx: usize) -> Option<usize> {
        let index_node = self.index.nodes.get(node_idx)?;
        let word_range = index_node.word_ranges.get(word_idx)?;
        let word_text = index_node.selection_plain_text.get(word_range.clone())?;
        let first_line = index_node.source_line_ranges.first().map_or_else(
            || {
                self.doc
                    .nodes
                    .get(node_idx)
                    .map_or(0, |n| n.source_start_line())
            },
            |(l, _)| *l,
        );
        // Find the same occurrence of the word in the rendered display
        // plain text — repeated words must map to the right occurrence,
        // not just the first match. Count occurrences in selection plain
        // text up to word_range.start, then locate the Nth occurrence in
        // display.
        let rn = self.rendered_nodes.get(node_idx)?;
        let occurrence = count_occurrences_before(
            &index_node.selection_plain_text,
            word_text,
            word_range.start,
        );
        let pos = nth_occurrence(&rn.plain, word_text, occurrence).unwrap_or(0);
        Some(first_line + newlines_before_byte(&rn.plain, pos))
    }

    fn context_lines(&self, source_line: usize) -> (String, String) {
        let prev = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .map_or("", String::as_str);
        let next = self
            .source_lines
            .get(source_line + 1)
            .map_or("", String::as_str);
        (
            clean_context(prev, EMIT_CONTEXT_MAX_CHARS),
            clean_context(next, EMIT_CONTEXT_MAX_CHARS),
        )
    }
}

// ── Clipboard ─────────────────────────────────────────────────────────────────

enum ClipboardOutcome {
    OsCommand,
    Osc52,
    Failed,
}

/// Count `\n` bytes in `plain[..byte]` — the number of source lines the
/// byte position is past inside the rendered display plain text.
fn newlines_before_byte(plain: &str, byte: usize) -> usize {
    plain
        .get(..byte)
        .map_or(0, |p| p.bytes().filter(|&b| b == b'\n').count())
}

/// Count occurrences of `needle` in `haystack[..before_byte]` (i.e. the
/// number of complete `needle` matches whose start position is strictly
/// before `before_byte`).
fn count_occurrences_before(haystack: &str, needle: &str, before_byte: usize) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let stop = before_byte.min(haystack.len());
    let mut count = 0usize;
    let mut cursor = 0usize;
    while cursor < stop {
        match haystack[cursor..stop].find(needle) {
            Some(off) => {
                count += 1;
                cursor += off + needle.len();
            }
            None => break,
        }
    }
    count
}

/// Return the byte offset of the `n`th occurrence (0-indexed) of `needle`
/// in `haystack`, if any.
fn nth_occurrence(haystack: &str, needle: &str, n: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut cursor = 0usize;
    for i in 0..=n {
        let off = haystack[cursor..].find(needle)?;
        let abs = cursor + off;
        if i == n {
            return Some(abs);
        }
        cursor = abs + needle.len();
    }
    None
}

fn copy_to_clipboard(text: &str) -> ClipboardOutcome {
    if try_osc52(text) {
        return ClipboardOutcome::Osc52;
    }
    if try_os_clipboard(text) {
        return ClipboardOutcome::OsCommand;
    }
    ClipboardOutcome::Failed
}

fn try_os_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(windows) {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (cmd, args) in candidates {
        let Ok(mut child) = Command::new(cmd).args(*args).stdin(Stdio::piped()).spawn() else {
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if child.wait().is_ok() {
            return true;
        }
    }
    false
}

fn try_osc52(text: &str) -> bool {
    use std::io::Write;
    let encoded = base64_encode(text.as_bytes());
    let seq = if std::env::var("TMUX").is_ok() {
        format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{encoded}\x07")
    };

    #[cfg(unix)]
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        return tty.write_all(seq.as_bytes()).is_ok() && tty.flush().is_ok();
    }

    let _ = std::io::stderr().write_all(seq.as_bytes());
    let _ = std::io::stderr().flush();
    false
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FILE_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn test_app(content: &str) -> App {
        let n = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rep_test_{n}.md"));
        std::fs::write(&path, content).unwrap();
        App::load(path).unwrap()
    }

    fn render(app: &mut App) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        terminal
    }

    fn key_char(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
    }

    fn row(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buf = terminal.backend().buffer();
        (0..buf.area.width)
            .map(|x| {
                buf.cell(ratatui::layout::Position::new(x, y))
                    .map_or(" ", |c| c.symbol())
            })
            .collect()
    }

    fn cell(terminal: &Terminal<TestBackend>, x: u16, y: u16) -> char {
        terminal
            .backend()
            .buffer()
            .cell(ratatui::layout::Position::new(x, y))
            .and_then(|c| c.symbol().chars().next())
            .unwrap_or(' ')
    }

    fn has_sentence_highlight(spans: &[Span<'_>]) -> bool {
        spans.iter().any(|s| s.style.bg == Some(Color::Blue))
    }

    fn highlighted_text(spans: &[Span<'_>]) -> String {
        spans
            .iter()
            .filter(|s| s.style.bg == Some(Color::Blue))
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    fn make_change(text: &str) -> ChangeAnnotation {
        ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(0),
            sentence_text: None,
            change: text.into(),
        }
    }

    fn make_feedback(text: &str) -> FeedbackAnnotation {
        FeedbackAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(0),
            sentence_text: None,
            feedback: text.into(),
        }
    }

    fn make_insert(text: &str) -> InsertAnnotation {
        InsertAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(0),
            sentence_text: None,
            text: text.into(),
        }
    }

    // ── Gutter indicators ─────────────────────────────────────────────────────

    #[test]
    fn gutter_shows_c_after_change() {
        let mut app = test_app("A sentence here.\n");
        app.changes.entry(0).or_default().push(make_change("x"));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), 'C', "expected change indicator 'C'");
    }

    #[test]
    fn gutter_shows_x_after_strike() {
        let mut app = test_app("A sentence here.\n");
        app.strikes
            .entry(0)
            .or_default()
            .insert((SelectionUnit::Sentence, 0));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), 'X', "expected strike indicator 'X'");
    }

    #[test]
    fn gutter_shows_f_after_feedback() {
        let mut app = test_app("A sentence here.\n");
        app.feedbacks.entry(0).or_default().push(make_feedback("x"));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), 'F', "expected feedback indicator 'F'");
    }

    #[test]
    fn gutter_shows_plus_after_insert_before() {
        let mut app = test_app("A sentence here.\n");
        app.inserts_before
            .entry(0)
            .or_default()
            .push(make_insert("new text"));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), '+', "expected insert indicator '+'");
    }

    #[test]
    fn gutter_shows_plus_after_insert_after() {
        let mut app = test_app("A sentence here.\n");
        app.inserts_after
            .entry(0)
            .or_default()
            .push(make_insert("new text"));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), '+', "expected insert indicator '+'");
    }

    #[test]
    fn gutter_shows_star_for_both() {
        let mut app = test_app("A sentence.\n");
        app.changes.entry(0).or_default().push(make_change("x"));
        app.strikes
            .entry(0)
            .or_default()
            .insert((SelectionUnit::Sentence, 0));
        let t = render(&mut app);
        assert_eq!(cell(&t, 1, 1), '*', "expected combined indicator '*'");
    }

    #[test]
    fn gutter_shows_star_for_two_changes_on_same_node() {
        let mut app = test_app("A sentence.\n");
        app.changes.entry(0).or_default().push(make_change("x"));
        app.changes.entry(0).or_default().push(make_change("y"));
        let t = render(&mut app);
        assert_eq!(
            cell(&t, 1, 1),
            '*',
            "expected '*' when multiple changes are stacked"
        );
    }

    #[test]
    fn gutter_shows_star_for_two_feedbacks_on_same_node() {
        let mut app = test_app("A sentence.\n");
        app.feedbacks.entry(0).or_default().push(make_feedback("x"));
        app.feedbacks.entry(0).or_default().push(make_feedback("y"));
        let t = render(&mut app);
        assert_eq!(
            cell(&t, 1, 1),
            '*',
            "expected '*' when multiple feedbacks are stacked"
        );
    }

    #[test]
    fn pressing_c_with_existing_change_enters_edit_mode() {
        let mut app = test_app("A sentence here.\n");
        app.changes
            .entry(0)
            .or_default()
            .push(make_change("prior change"));
        app.handle_key(key_char('c'));
        assert_eq!(app.input_mode, InputMode::EditChange(0, 0));
        assert_eq!(app.change_buffer, "prior change");
    }

    #[test]
    fn pressing_f_with_existing_feedback_enters_edit_mode() {
        let mut app = test_app("A sentence here.\n");
        app.feedbacks
            .entry(0)
            .or_default()
            .push(make_feedback("prior feedback"));
        app.handle_key(key_char('f'));
        assert_eq!(app.input_mode, InputMode::EditFeedback(0, 0));
        assert_eq!(app.feedback_buffer, "prior feedback");
    }

    #[test]
    fn pressing_c_without_existing_change_starts_new() {
        let mut app = test_app("A sentence here.\n");
        app.handle_key(key_char('c'));
        assert_eq!(app.input_mode, InputMode::Change);
        assert!(app.change_buffer.is_empty());
    }

    #[test]
    fn pressing_c_on_other_sentence_does_not_edit_neighbor() {
        let mut app = test_app("First sentence. Second sentence.\n");
        // change is on sentence 0
        app.changes
            .entry(0)
            .or_default()
            .push(make_change("for first"));
        // move cursor to sentence 1
        app.handle_key(key_char('j'));
        app.handle_key(key_char('c'));
        // should start a NEW change, not edit the one on sentence 0
        assert_eq!(app.input_mode, InputMode::Change);
        assert!(app.change_buffer.is_empty());
    }

    #[test]
    fn gutter_shows_space_when_unannotated() {
        let mut app = test_app("A sentence.\n");
        let t = render(&mut app);
        assert_eq!(
            cell(&t, 1, 1),
            ' ',
            "expected blank indicator for unannotated node"
        );
    }

    #[test]
    fn block_title_shows_filename() {
        let n = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("named_doc_{n}.md"));
        std::fs::write(&path, "line one\n").unwrap();
        let stem = path.file_name().unwrap().to_string_lossy().to_string();
        let mut app = App::load(path).unwrap();
        let t = render(&mut app);
        let top = row(&t, 0);
        assert!(top.contains(&stem), "filename missing from title: {top:?}");
    }

    #[test]
    fn block_title_shows_annotation_counts() {
        let mut app = test_app("A sentence.\n");
        app.changes.entry(0).or_default().push(make_change("x"));
        app.feedbacks.entry(0).or_default().push(make_feedback("y"));
        app.strikes
            .entry(0)
            .or_default()
            .insert((SelectionUnit::Sentence, 0));
        let t = render(&mut app);
        let top = row(&t, 0);
        assert!(top.contains("1C"), "change count missing: {top:?}");
        assert!(top.contains("1F"), "feedback count missing: {top:?}");
        assert!(top.contains("1X"), "strike count missing: {top:?}");
    }

    #[test]
    fn footer_shows_mode_indicator_and_status_or_hint() {
        // Right zone priority is nav_feedback > notification > status >
        // help hint. After test_app() the status carries the load
        // confirmation, so it pre-empts the hint. Force-clear status to
        // verify the help-hint fallback path.
        let mut app = test_app("line\n");
        app.status.clear();
        let t = render(&mut app);
        let hint_row = row(&t, 23);
        assert!(
            hint_row.contains("mode: sentence"),
            "mode indicator missing in left zone: {hint_row:?}"
        );
        assert!(
            hint_row.contains("? for help"),
            "help hint missing in right zone (with empty status): {hint_row:?}"
        );
    }

    #[test]
    fn footer_shows_status_in_right_zone() {
        let mut app = test_app("line\n");
        app.status = "Match 1/2 for \"foo\".".to_string();
        let t = render(&mut app);
        let hint_row = row(&t, 23);
        assert!(
            hint_row.contains("Match 1/2"),
            "status missing in right zone: {hint_row:?}"
        );
    }

    #[test]
    fn section_boundary_keypress_keeps_highlight_on_current_section() {
        // In Section mode at the only section, pressing j should set
        // "at end" feedback but keep the section highlight on the user's
        // current section — they're still selecting it.
        let mut app = test_app("# Only section\n\nbody.\n");
        // Cycle into Section mode (Backspace x3: Sentence -> Line ->
        // Paragraph -> Section).
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Section);
        assert!(
            app.section_highlight_range.is_some(),
            "highlight set on entry to Section mode"
        );
        // Push past the only section's end.
        app.handle_key(key_char('j'));
        assert_eq!(
            app.nav_feedback.as_deref(),
            Some("at end"),
            "boundary feedback expected"
        );
        assert!(
            app.section_highlight_range.is_some(),
            "highlight should NOT be cleared on a boundary in Section mode"
        );
    }

    #[test]
    fn section_nav_on_zero_section_doc_is_silent_no_op() {
        // Per modular_plan §"Empty / degenerate documents": a zero-anchor
        // unit (e.g., section nav with no headings + no top-level OL) is
        // an immediate Boundary with NO feedback string written.
        let mut app = test_app("Plain prose with no sections.\n");
        // Cycle backward into Section mode (clamp lands on PreHeading
        // section if the doc has any pre-content; here the prose IS the
        // pre-heading section, so clamp returns a valid Section anchor).
        // To exercise the truly-zero-anchor case, force-set an empty unit
        // and confirm move_active_unit doesn't write feedback.
        app.selection_state.anchor = SelectionAnchor::new(0, SelectionUnit::Word, 999);
        // Reach a definitively-empty unit by force-setting an anchor whose
        // unit doesn't have entries. Use Section on a doc-with-only-prose
        // — there IS one PreHeading entry, but words give us 6 entries
        // and pre-conditions are tricky to engineer. Instead, build
        // directly on a one-code-block doc which has zero sentence anchors.
        let mut app2 = test_app("```\nfn x() {}\n```");
        app2.selection_state.anchor = SelectionAnchor::new(0, SelectionUnit::Sentence, 0);
        app2.handle_key(key_char('j'));
        assert_eq!(
            app2.nav_feedback, None,
            "zero-anchor unit must not write feedback"
        );
    }

    #[test]
    fn footer_shows_boundary_feedback_after_overshoot() {
        let mut app = test_app("Only sentence.\n");
        app.handle_key(key_char('j'));
        let t = render(&mut app);
        let row23 = row(&t, 23);
        assert!(
            row23.contains("at end"),
            "expected 'at end' nav_feedback: {row23:?}"
        );
    }

    #[test]
    fn input_mode_space_and_backspace_are_literal_chars() {
        // In Change input mode, pressing Space appends ' ' to the buffer
        // (it does NOT cycle the selection unit). Backspace deletes the
        // last char (it does NOT reverse-cycle).
        let mut app = test_app("A sentence here.\n");
        app.handle_key(key_char('c'));
        assert_eq!(app.input_mode, InputMode::Change);
        app.handle_key(key_char('a'));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(key_char('b'));
        assert_eq!(app.change_buffer, "a b");
        // Backspace deletes 'b'.
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.change_buffer, "a ");
        // Active selection unit must still be the pre-input Sentence (not
        // re-anchored by Space/Backspace as if they were mode-cycle keys).
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
    }

    #[test]
    fn block_title_shows_insert_count() {
        let mut app = test_app("A sentence.\n");
        app.inserts_before
            .entry(0)
            .or_default()
            .push(make_insert("x"));
        app.inserts_after
            .entry(0)
            .or_default()
            .push(make_insert("y"));
        let t = render(&mut app);
        let top = row(&t, 0);
        assert!(top.contains("2I"), "insert count missing: {top:?}");
    }

    #[test]
    fn agent_output_includes_inserts_before_and_after() {
        let mut app = test_app("First. Second.\n");
        app.inserts_before
            .entry(0)
            .or_default()
            .push(make_insert("pre"));
        app.inserts_after
            .entry(0)
            .or_default()
            .push(make_insert("post"));
        let out = app.to_output();
        assert_eq!(out.annotations.len(), 1);
        assert_eq!(out.annotations[0].inserts_before.len(), 1);
        assert_eq!(out.annotations[0].inserts_after.len(), 1);
        assert_eq!(out.annotations[0].inserts_before[0].text, "pre");
        assert_eq!(out.annotations[0].inserts_after[0].text, "post");
        assert_eq!(out.keymap.insert_before, "b");
        assert_eq!(out.keymap.insert_after, "a");
    }

    #[test]
    fn b_key_enters_insert_before_mode_and_saves() {
        let mut app = test_app("A sentence.\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::InsertBefore);
        for ch in "hello".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
        let bucket = app.inserts_before.get(&0).expect("bucket should exist");
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].text, "hello");
    }

    #[test]
    fn a_key_enters_insert_after_mode_and_saves() {
        let mut app = test_app("A sentence.\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::InsertAfter);
        for ch in "post".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
        let bucket = app.inserts_after.get(&0).expect("bucket should exist");
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].text, "post");
    }

    #[test]
    fn insert_mode_esc_cancels_without_saving() {
        let mut app = test_app("A sentence.\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        for ch in "abc".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.inserts_before.is_empty());
    }

    // ── Search ────────────────────────────────────────────────────────────────

    fn type_search(app: &mut App, query: &str) {
        app.handle_key(key_char('/'));
        assert_eq!(app.input_mode, InputMode::Search);
        for ch in query.chars() {
            app.handle_key(key_char(ch));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn search_jumps_cursor_to_first_match() {
        let mut app = test_app("Alpha paragraph.\n\nBeta paragraph with target here.\n\nGamma.\n");
        type_search(&mut app, "target");
        assert_eq!(
            app.selection_state.anchor.node_idx, 1,
            "cursor should land on Beta node"
        );
        assert!(app.status.contains("Match 1/1"), "status: {}", app.status);
    }

    #[test]
    fn jump_to_annotation_resets_unit_to_sentence() {
        // Set up an annotation on node 1, cycle into Word mode on node 0,
        // then `]` should jump to the annotated node and reset unit.
        let mut app = test_app("First sentence.\n\nSecond paragraph.\n");
        app.changes.entry(1).or_default().push(make_change("x"));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        app.handle_key(key_char(']'));
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        assert_eq!(
            app.selection_state.anchor.unit,
            SelectionUnit::Sentence,
            "annotation jump must re-anchor to Sentence"
        );
    }

    #[test]
    fn c_in_word_mode_edits_most_recent_change_on_node() {
        // Existing change on node 0 sentence 0. From Word mode, pressing
        // `c` should still find the existing change and enter edit mode
        // (matched by node only, not by sentence_idx-as-word_idx).
        let mut app = test_app("First sentence here.\n");
        app.changes
            .entry(0)
            .or_default()
            .push(make_change("existing"));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        app.handle_key(key_char('c'));
        match app.input_mode {
            InputMode::EditChange(_, _) => {}
            other => panic!("expected EditChange, got {other:?}"),
        }
    }

    #[test]
    fn current_sentence_links_returns_all_node_links_in_word_mode() {
        // Two sentences each with one link. In Sentence mode the link list
        // is scoped to the current sentence; in Word mode (unit_idx is a
        // word index, not a sentence index) we fall back to "all links in
        // the node" rather than mis-index sentence_ranges.
        let mut app = test_app(
            "[First link](https://example.com) here. [Other link](https://other.com) there.\n",
        );
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        let links = app.current_sentence_links();
        // Both URLs surface in Word mode.
        assert!(links.iter().any(|u| u.contains("example.com")), "{links:?}");
        assert!(links.iter().any(|u| u.contains("other.com")), "{links:?}");
    }

    #[test]
    fn search_from_word_mode_resets_to_sentence_anchor() {
        // When the user is in Word mode and triggers a search, the match
        // table is sentence-keyed; the anchor must reset to Sentence so
        // unit_idx is interpreted correctly.
        let mut app = test_app("Alpha sentence.\n\nBeta paragraph with target here.\n");
        // Cycle into Word mode and step through a few words.
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(key_char('j'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        type_search(&mut app, "target");
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        assert_eq!(
            app.selection_state.anchor.unit,
            SelectionUnit::Sentence,
            "search must re-anchor to Sentence"
        );
    }

    #[test]
    fn search_no_match_leaves_cursor_in_place() {
        let mut app = test_app("Alpha.\n\nBeta.\n");
        let node_before = app.selection_state.anchor.node_idx;
        type_search(&mut app, "notfound");
        assert_eq!(app.selection_state.anchor.node_idx, node_before);
        assert!(app.status.contains("No matches"), "status: {}", app.status);
    }

    #[test]
    fn search_is_case_insensitive_by_default() {
        let mut app = test_app("First.\n\nthe TARGET lives here.\n");
        type_search(&mut app, "target");
        assert_eq!(app.selection_state.anchor.node_idx, 1);
    }

    #[test]
    fn search_is_case_sensitive_when_query_has_uppercase() {
        let mut app = test_app("First has Target.\n\nSecond has target.\n");
        type_search(&mut app, "Target");
        assert_eq!(
            app.selection_state.anchor.node_idx, 0,
            "should find capital 'Target' on first node"
        );
    }

    #[test]
    fn n_key_advances_to_next_match_and_wraps() {
        let mut app = test_app("foo one.\n\nfoo two.\n\nfoo three.\n");
        type_search(&mut app, "foo");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        app.handle_key(key_char('n'));
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        app.handle_key(key_char('n'));
        assert_eq!(app.selection_state.anchor.node_idx, 2);
        app.handle_key(key_char('n'));
        assert_eq!(
            app.selection_state.anchor.node_idx, 0,
            "n should wrap to first match"
        );
    }

    #[test]
    fn capital_n_goes_to_previous_match() {
        let mut app = test_app("foo one.\n\nfoo two.\n\nfoo three.\n");
        type_search(&mut app, "foo");
        app.handle_key(key_char('n'));
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
        assert_eq!(app.selection_state.anchor.node_idx, 0);
    }

    #[test]
    fn n_without_previous_search_sets_status() {
        let mut app = test_app("foo.\n");
        app.handle_key(key_char('n'));
        assert!(
            app.status.contains("No previous search"),
            "status: {}",
            app.status
        );
    }

    #[test]
    fn search_esc_cancels_without_running() {
        let mut app = test_app("foo.\n");
        app.handle_key(key_char('/'));
        app.handle_key(key_char('x'));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert!(app.last_search.is_none());
    }

    #[test]
    fn shift_slash_still_opens_help() {
        let mut app = test_app("foo.\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::SHIFT));
        assert!(app.show_help, "shift+/ should still open help");
        assert_eq!(app.input_mode, InputMode::Normal);
    }

    #[test]
    fn search_matches_within_same_node_multiple_sentences() {
        let mut app = test_app("The foo sentence. The bar sentence.\n");
        type_search(&mut app, "foo");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        assert_eq!(
            app.selection_state.anchor.unit_idx, 0,
            "should land on first sentence"
        );
    }

    #[test]
    fn search_counts_every_hit_including_duplicates_in_one_sentence() {
        let mut app = test_app("foo foo foo end.\n");
        type_search(&mut app, "foo");
        assert!(
            app.status.contains("Match 1/3"),
            "every occurrence should be counted: {}",
            app.status
        );
    }

    #[test]
    fn strike_in_word_mode_records_word_unit_strike() {
        // Per modular_plan §"Functional" Req 3, `delete this` works at
        // any unit. Word-mode `x` records a (Word, unit_idx) entry in
        // self.strikes so emit / render can target the word, not the
        // surrounding sentence.
        let mut app = test_app("alpha beta gamma\n");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        let word_idx = app.selection_state.anchor.unit_idx;
        app.handle_key(key_char('x'));
        let strikes = app
            .strikes
            .get(&0)
            .expect("strike entry created on first x");
        assert!(
            strikes.contains(&(SelectionUnit::Word, word_idx)),
            "expected (Word, {word_idx}) in strikes, got {strikes:?}"
        );
        // Toggle: a second `x` removes it.
        app.handle_key(key_char('x'));
        assert!(
            !app.strikes.contains_key(&0),
            "second x should remove the word-unit strike"
        );
    }

    #[test]
    fn arrow_keys_preserve_active_unit_in_word_mode() {
        // Modular_plan §"Key bindings": arrows are unit-agnostic synonyms
        // for j / k. Cycling into Word mode and pressing Right / Left
        // walks word anchors but keeps the unit at Word.
        let mut app = test_app("alpha beta gamma delta epsilon\n");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        let start = app.selection_state.anchor.unit_idx;
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(
            app.selection_state.anchor.unit,
            SelectionUnit::Word,
            "Right must not change unit"
        );
        assert_eq!(app.selection_state.anchor.unit_idx, start + 2);
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        assert_eq!(app.selection_state.anchor.unit_idx, start + 1);
    }

    #[test]
    fn change_status_reports_per_unit_source_line() {
        // Multi-line paragraph; cursor on sentence 1 (second sentence).
        // Status after `c X Enter` should mention line 2, not line 1.
        let mut app = test_app("First sentence.\nSecond sentence.\n");
        app.selection_state.anchor.unit_idx = 1;
        app.handle_key(key_char('c'));
        app.handle_key(key_char('X'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(
            app.status.contains("(line 2)"),
            "status should report sentence's source line: {}",
            app.status
        );
    }

    #[test]
    fn word_change_repeated_word_lands_on_correct_source_line() {
        // Multi-line paragraph with the same word on both lines.
        // Word 0 = "the" on line 1; word 3 = "the" on line 2.
        let mut app = test_app("the cat sat\nthe mat slept\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Word,
            sentence_index: Some(3),
            sentence_text: Some("the".into()),
            change: "Initial".into(),
        });
        let out = app.to_human_output();
        assert!(
            out.contains("WHERE: line 2\n"),
            "second `the` must map to line 2, not line 1: {out}"
        );
    }

    #[test]
    fn newlines_before_byte_basic() {
        assert_eq!(super::newlines_before_byte("a\nb\nc", 0), 0);
        assert_eq!(super::newlines_before_byte("a\nb\nc", 1), 0);
        assert_eq!(super::newlines_before_byte("a\nb\nc", 2), 1);
        assert_eq!(super::newlines_before_byte("a\nb\nc", 4), 2);
        assert_eq!(super::newlines_before_byte("a\nb\nc", 5), 2);
        // out-of-range byte is clamped silently — falls back to zero.
        assert_eq!(super::newlines_before_byte("abc", 999), 0);
    }

    #[test]
    fn count_occurrences_before_empty_needle_returns_zero() {
        assert_eq!(super::count_occurrences_before("a b c", "", 5), 0);
    }

    #[test]
    fn nth_occurrence_empty_needle_returns_none() {
        assert_eq!(super::nth_occurrence("a b c", "", 0), None);
    }

    #[test]
    fn count_occurrences_before_basic() {
        assert_eq!(super::count_occurrences_before("a b a c a", "a", 0), 0);
        assert_eq!(super::count_occurrences_before("a b a c a", "a", 1), 1);
        assert_eq!(super::count_occurrences_before("a b a c a", "a", 4), 1);
        assert_eq!(super::count_occurrences_before("a b a c a", "a", 5), 2);
        assert_eq!(super::count_occurrences_before("a b a c a", "a", 9), 3);
    }

    #[test]
    fn nth_occurrence_basic() {
        assert_eq!(super::nth_occurrence("a b a c a", "a", 0), Some(0));
        assert_eq!(super::nth_occurrence("a b a c a", "a", 1), Some(4));
        assert_eq!(super::nth_occurrence("a b a c a", "a", 2), Some(8));
        assert_eq!(super::nth_occurrence("a b a c a", "a", 3), None);
    }

    #[test]
    fn truncate_to_columns_handles_ascii() {
        assert_eq!(super::truncate_to_columns("abcdef", 3), "abc");
        assert_eq!(super::truncate_to_columns("abc", 5), "abc");
        assert_eq!(super::truncate_to_columns("", 5), "");
    }

    #[test]
    fn truncate_to_columns_does_not_split_a_wide_char_across_the_budget() {
        // CJK chars are 2 cols each. Budget of 3 columns fits one CJK +
        // one ASCII (total 3), but not two CJK (would be 4).
        assert_eq!(super::truncate_to_columns("日本語", 4), "日本");
        assert_eq!(super::truncate_to_columns("日本語", 3), "日");
        assert_eq!(super::truncate_to_columns("日本語", 1), "");
    }

    #[test]
    fn truncate_to_columns_handles_zero_width_combining_marks() {
        // "café" with combining acute (U+0301) — 4 columns, 5 chars.
        let s = "cafe\u{0301}";
        assert_eq!(super::truncate_to_columns(s, 4), s);
        assert_eq!(super::truncate_to_columns(s, 3), "caf");
    }

    #[test]
    fn truncate_to_columns_zero_budget_yields_empty_even_for_combining_mark() {
        // Edge case: a string starting with a zero-width char and a budget
        // of 0. Without the explicit guard the loop's "used + 0 > 0" check
        // is false, so the orphaned mark would slip through. Guard wins.
        assert_eq!(super::truncate_to_columns("\u{0301}abc", 0), "");
        assert_eq!(super::truncate_to_columns("abc", 0), "");
    }

    #[test]
    fn word_change_uses_word_source_line_in_multi_line_paragraph() {
        // Multi-line paragraph: word on second line should emit
        // `WHERE: line 2`, not the paragraph's first line.
        let mut app = test_app("First line.\nSecond line.\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Word,
            sentence_index: Some(2), // word_idx; "Second" is index 2 in
            // [First, line, Second, line]
            sentence_text: Some("Second".into()),
            change: "Initial".into(),
        });
        let out = app.to_human_output();
        assert!(
            out.contains("WHERE: line 2\n"),
            "should key on word's source line: {out}"
        );
    }

    #[test]
    fn sentence_change_uses_sentence_source_line_in_multi_line_paragraph() {
        // Multi-line paragraph: line 1 = "First sentence.", line 2 =
        // "Second sentence.". A change captured on sentence 1 must emit
        // `WHERE: line 2`, not the paragraph's first line.
        let mut app = test_app("First sentence.\nSecond sentence.\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(1),
            sentence_text: Some("Second sentence.".into()),
            change: "Rewrite second".into(),
        });
        let out = app.to_human_output();
        assert!(
            out.contains("WHERE: line 2\n"),
            "should key on sentence's begin line: {out}"
        );
    }

    #[test]
    fn strike_emit_uses_struck_sentence_source_line() {
        // Multi-line paragraph: line 1 = "First line.", line 2 = "Second
        // line.". Striking sentence index 1 must emit `WHERE: line 2`,
        // not the paragraph's first line.
        let mut app = test_app("First line.\nSecond line.\n");
        app.strikes
            .entry(0)
            .or_default()
            .insert((SelectionUnit::Sentence, 1));
        let out = app.to_human_output();
        assert!(out.contains("ACTION: delete this"), "{out}");
        assert!(
            out.contains("WHERE: line 2\n"),
            "should key on struck sentence's begin line, not node's first: {out}"
        );
        assert!(out.contains("\"Second line.\""), "{out}");
    }

    #[test]
    fn agent_output_sentence_index_is_one_based() {
        let mut app = test_app("First. Second.\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(1), // 0-based: second sentence
            sentence_text: Some("Second.".into()),
            change: "Fix this".into(),
        });
        let out = app.to_output();
        assert_eq!(out.annotations.len(), 1);
        assert_eq!(
            out.annotations[0].changes[0].sentence_index,
            Some(2),
            "agent output sentence_index must be 1-based"
        );
    }

    #[test]
    fn edit_key_enters_feedback_edit_mode_when_feedback_exists() {
        let mut app = test_app("A sentence here.\n");
        app.feedbacks
            .entry(0)
            .or_default()
            .push(make_feedback("make this clearer"));

        app.handle_key(key_char('e'));

        assert_eq!(app.input_mode, InputMode::EditFeedback(0, 0));
        assert_eq!(app.feedback_buffer, "make this clearer");
    }

    #[test]
    fn x_key_removes_change_before_striking() {
        let mut app = test_app("A sentence here.\n");
        app.changes
            .entry(0)
            .or_default()
            .push(make_change("rewrite"));

        app.handle_key(key_char('x'));
        assert!(
            !app.changes.contains_key(&0),
            "first x should remove the existing change"
        );
        assert!(
            !app.strikes.contains_key(&0),
            "first x should not strike when removing a change"
        );

        app.handle_key(key_char('x'));
        assert!(
            app.strikes
                .get(&0)
                .is_some_and(|set| set.contains(&(SelectionUnit::Sentence, 0))),
            "second x should strike once no change/feedback remains"
        );
    }

    #[test]
    fn x_key_removes_feedback_before_striking() {
        let mut app = test_app("A sentence here.\n");
        app.feedbacks
            .entry(0)
            .or_default()
            .push(make_feedback("make this clearer"));

        app.handle_key(key_char('x'));
        assert!(
            !app.feedbacks.contains_key(&0),
            "first x should remove the existing feedback"
        );
        assert!(
            !app.strikes.contains_key(&0),
            "first x should not strike when removing feedback"
        );
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    // ── Sentence context and highlight ────────────────────────────────────────

    #[test]
    fn sentence_context_for_soft_wrapped_paragraph() {
        // The two source lines are one paragraph; sentences are correctly split
        // on the joined text, not on individual lines.
        let app = test_app(
            "  - Stabilize commands/flags (future --output,\n    --stdin, --version)\n  - Next item.\n",
        );
        let (_, context) = app.current_sentence_context().expect("sentence context");
        assert!(context.contains("Stabilize commands/flags"), "{context}");
        assert!(context.contains("stdin"), "{context}");
        assert!(context.contains("version)"), "{context}");
        assert!(!context.contains("Next item"), "context leaked: {context}");
    }

    #[test]
    fn sentence_context_single_sentence_stops_at_node_boundary() {
        let app = test_app("First sentence ends.\nSecond sentence starts here.\n");
        let (_, context) = app.current_sentence_context().expect("sentence context");
        // Paragraph is joined → two sentences; cursor on sentence 0.
        assert!(context.contains("First sentence ends."), "{context}");
        assert!(!context.contains("Second sentence"), "{context}");
    }

    #[test]
    fn sentence_highlight_covers_full_node_when_cursor_on_it() {
        let mut app = test_app("Hello world. Goodbye world.\n");
        app.selection_state.anchor.node_idx = 0;
        app.selection_state.anchor.unit_idx = 0;
        let spans = app.render_node_spans(0);
        assert!(
            has_sentence_highlight(&spans),
            "first sentence should be highlighted"
        );
    }

    #[test]
    fn sentence_highlight_absent_on_other_nodes() {
        let mut app = test_app("Para one.\n\nPara two.\n");
        app.selection_state.anchor.node_idx = 0;
        app.selection_state.anchor.unit_idx = 0;
        let spans = app.render_node_spans(1);
        assert!(
            !has_sentence_highlight(&spans),
            "node 1 should not be highlighted"
        );
    }

    #[test]
    fn word_mode_highlight_tracks_each_word_on_j_navigation() {
        let mut app = test_app("alpha beta gamma\n");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);

        let spans0 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans0), "alpha");

        app.handle_key(key_char('j'));
        let spans1 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans1), "beta");

        app.handle_key(key_char('j'));
        let spans2 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans2), "gamma");
    }

    #[test]
    fn word_mode_highlight_tracks_each_word_with_smart_apostrophe() {
        let mut app = test_app("we’re in an early period\n");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);

        let spans0 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans0), "we");

        app.handle_key(key_char('j'));
        let spans1 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans1), "re");

        app.handle_key(key_char('j'));
        let spans2 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans2), "in");
    }

    #[test]
    fn i_and_u_keys_cycle_unit_finer_and_coarser() {
        // i = "in" / finer (synonym for Space). u = "up" / coarser
        // (synonym for Backspace). One i step from default Sentence
        // lands on Word; subsequent u step returns to Sentence.
        let mut app = test_app("Plain prose paragraph here.\n");
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
        app.handle_key(key_char('i'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        app.handle_key(key_char('u'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
        // Space and Backspace still work as synonyms.
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
    }

    #[test]
    fn capital_i_opens_ast_view_lowercase_does_not() {
        // Capital `I` opens the AST popup; lowercase `i` is the
        // mode-cycle key. Verifies the key migration is complete.
        let mut app = test_app("Body here.\n");
        app.handle_key(key_char('i'));
        assert!(
            app.ast_view_scroll.is_none(),
            "lowercase i must not open AST view"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE));
        assert!(
            app.ast_view_scroll.is_some(),
            "capital I must open AST view"
        );
    }

    #[test]
    fn word_mode_highlight_paints_inside_code_block_yaml_frontmatter() {
        // Regression: the special code-block draw branch in App::draw used
        // to bypass render_node_spans entirely, so word-mode highlight on a
        // code block (including YAML frontmatter folded as CodeBlock(yaml))
        // showed nothing on screen even though the anchor advanced.
        //
        // Render the full frame to a TestBackend and scan the output for a
        // cell with the highlight bg color (Color::Blue). The frontmatter
        // must paint the active anchor on every j step.
        let mut app = test_app("---\ntitle: Hello\n---\n\nBody.\n");
        // Cycle into Word mode (Space x1) — anchor is now on the first
        // word in the YAML CodeBlock.
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        assert_eq!(app.selection_state.anchor.node_idx, 0);

        let terminal = render(&mut app);
        let buf = terminal.backend().buffer();
        // Hunt for any cell with bg=Blue, the highlight color used by
        // both render_node_spans and the new code-block overlay.
        let mut highlighted = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = buf
                    .cell(ratatui::layout::Position::new(x, y))
                    .expect("cell");
                if cell.bg == Color::Blue {
                    highlighted.push_str(cell.symbol());
                }
            }
        }
        assert!(
            !highlighted.is_empty(),
            "expected a Blue-bg highlight cell on the YAML frontmatter word; \
             scanned buffer found none"
        );
        // The active anchor on Space-cycle is the first word; the
        // frontmatter content is `title: Hello`, so word 0 is "title".
        // Allow a substring match so we don't overspecify.
        assert!(
            highlighted.contains("title") || highlighted.contains("Hello"),
            "highlight cells didn't include a frontmatter word; got {highlighted:?}"
        );
    }

    #[test]
    fn sentence_ranges_applied_to_newline_joined_plain_are_byte_valid() {
        // Ensure cursor_sentence=0 always lands on a range that begins at byte 0
        // (i.e. covers "Option 1 ..."), not a later sentence mid-paragraph.
        let content = "\
  Option 1 — Lean (~60 lines)
    Triggers: push/PR to main
    Rust:     stable only
    Cache:    Swatinem/rust-cache@v2
    Tradeoff: No MSRV, no audit. Fast (~30-60s).
";
        let app = test_app(content);
        let rn = &app.rendered_nodes[0];
        assert!(
            !rn.sentence_ranges.is_empty(),
            "should have at least one sentence range"
        );
        // The first sentence range must start at byte 0 so that cursor_sentence=0
        // highlights the beginning of the node ("Option 1 ..."), not a later sentence.
        assert_eq!(
            rn.sentence_ranges[0].start, 0,
            "sentence 0 must start at byte 0 (i.e. cover 'Option 1'), got ranges: {:?}",
            rn.sentence_ranges
        );
    }

    #[test]
    fn multi_line_paragraph_first_rendered_line_matches_source_first_line() {
        // "Option 1" style paragraph: first source line on its own, continuation
        // lines indented. The first visible line of the rendered node must be the
        // first source line, not a merged blob of all lines.
        let content = "\
  Option 1 — Lean (~60 lines)
    Triggers: push/PR to main
    Rust:     stable only
    Cache:    Swatinem/rust-cache@v2
    Tradeoff: No MSRV, no audit. Fast (~30-60s).
";
        let app = test_app(content);
        assert_eq!(app.doc.node_count(), 1);
        let rn = &app.rendered_nodes[0];
        // The plain text must start with the first source line, not a joined blob.
        assert!(
            rn.plain.starts_with("Option 1"),
            "first rendered line should start with 'Option 1', got: {:?}",
            &rn.plain[..rn.plain.len().min(60)]
        );
        // The per-line rendering inserts '\n' between source lines, not ' '.
        let first_line: &str = rn.plain.split('\n').next().unwrap_or("");
        assert_eq!(
            first_line, "Option 1 — Lean (~60 lines)",
            "first line should be the literal first source line"
        );
    }

    // ── move_sentence boundary conditions ─────────────────────────────────────

    #[test]
    fn cycling_out_of_section_mode_clears_section_highlight_range() {
        // Cycle into Section mode (Backspace x3: Sentence -> Line ->
        // Paragraph -> Section). mode_cycle sets section_highlight_range
        // for Section anchors and clears it for any other unit on the
        // way out.
        let mut app = test_app("Intro\n\n# A\ntext\n");
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Section);
        assert!(
            app.section_highlight_range.is_some(),
            "highlight set on Section-mode entry"
        );
        // Forward cycle Section -> Paragraph; expect highlight cleared.
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Paragraph);
        assert!(
            app.section_highlight_range.is_none(),
            "highlight cleared after cycling out of Section mode"
        );
    }

    #[test]
    fn jump_to_annotation_from_section_mode_clears_section_highlight() {
        // In Section mode, []/[ jump to annotated nodes via clamp_sentence
        // which forces unit -> Sentence. Without the refresh inside
        // clamp_sentence the previous Section-mode highlight stuck around
        // and the renderer kept painting the section span.
        let mut app = test_app("# A\n\nfirst.\n\n# B\n\nsecond.\n");
        app.changes.entry(2).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(0),
            sentence_text: Some("first.".into()),
            change: "x".into(),
        });
        // Enter Section mode (Backspace x3 from Sentence default).
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Section);
        assert!(app.section_highlight_range.is_some());
        // `]` jumps forward to the next annotated node; clamp_sentence runs.
        app.handle_key(key_char(']'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
        assert!(
            app.section_highlight_range.is_none(),
            "section highlight must clear when clamp_sentence forces unit -> Sentence"
        );
    }

    // ── current_sentence_links ────────────────────────────────────────────────

    #[test]
    fn current_sentence_links_isolates_links_per_sentence() {
        let mut app = test_app(
            "[Click here](https://example.com) for info. [Other link](https://other.com) elsewhere.\n",
        );
        // sentence 0: "Click here for info."  — contains first link
        // sentence 1: "Other link elsewhere." — contains second link
        app.selection_state.anchor.unit_idx = 0;
        let links = app.current_sentence_links();
        assert_eq!(
            links.len(),
            1,
            "sentence 0 should have exactly one link: {links:?}"
        );
        assert!(links[0].contains("example.com"), "{links:?}");

        app.selection_state.anchor.unit_idx = 1;
        let links = app.current_sentence_links();
        assert_eq!(
            links.len(),
            1,
            "sentence 1 should have exactly one link: {links:?}"
        );
        assert!(links[0].contains("other.com"), "{links:?}");
    }

    // ── move_section / move_block boundary conditions ─────────────────────────

    #[test]
    fn section_mode_silent_noop_in_heading_less_doc() {
        // Per modular_plan §"Section unit": "A document whose
        // pre-heading region is empty or contains only thematic breaks /
        // empty headings / wordless nodes has no section 0; section nav
        // goes straight to the first heading." A prose-only document
        // has no first heading either — so the section table is empty
        // and Section-mode entry is a silent no-op (clamp can't find a
        // target). Cursor stays in whatever unit it was before.
        let mut app = test_app("Just a paragraph. No headings.");
        let start_node = app.selection_state.anchor.node_idx;
        let start_unit = app.selection_state.anchor.unit;
        // Try to cycle into Section. Backspace 3x from default Sentence:
        // Sentence -> Line -> Paragraph -> (no Section anchor available,
        // clamp returns the Paragraph anchor unchanged).
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
        assert_ne!(
            app.selection_state.anchor.unit,
            SelectionUnit::Section,
            "with no sections in the doc, Section mode must not activate"
        );
        assert_eq!(app.selection_state.anchor.node_idx, start_node);
        // The mode-cycle still made progress — we landed on a coarser
        // unit, just not Section. Don't assume which one (Paragraph vs
        // staying put depends on whether Paragraph anchors exist).
        let _ = start_unit;
    }

    #[test]
    fn section_nav_sets_highlight_range_to_section_boundary() {
        // nodes: [Para(intro), Heading(A), Para(text), Heading(B), Para(final)]
        // Cycle into Section mode (lands on PreHeading at node 0), then
        // step j twice to walk Section A then Section B; assert each
        // section_highlight_range matches the section table's span.
        let mut app = test_app("Intro\n\n# A\ntext\n\n# B\nfinal\n");
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        // Section 0 (PreHeading): nodes [0..0].
        let range = app
            .section_highlight_range
            .clone()
            .expect("highlight set on Section entry");
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 1);

        app.handle_key(key_char('j')); // → Heading A section
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        let range = app.section_highlight_range.clone().expect("set");
        assert_eq!(range.start, 1);
        assert_eq!(range.end, 3, "section A spans nodes 1..3");

        app.handle_key(key_char('j')); // → Heading B section
        assert_eq!(app.selection_state.anchor.node_idx, 3);
        let range = app.section_highlight_range.clone().expect("set");
        assert_eq!(range.start, 3);
        assert_eq!(range.end, 5, "last section spans to end of doc");
    }

    #[test]
    fn tall_item_at_bottom_renders_partial_not_blank() {
        // 10 list items (no inter-item spacers) + 1 tall paragraph.
        // Layout: height=15, footer=1, outer block=13, border top+bottom → inner=11.
        // But we need inner=12, so use height=15 → outer=14 → inner=12.
        //
        // With inner_height=12:
        //   Nodes 0-8 (list items, no spacer): 9 rows.
        //   Node 9 (last list item + trailing spacer before paragraph): 2 rows.
        //   Total: 11 rows → 1 row left at inner row 11 = terminal row 12.
        // That row must show "tall line 0", not be blank.
        let mut content = String::new();
        for i in 0..10 {
            let _ = writeln!(content, "- Item {i}");
        }
        content.push('\n'); // blank line separates list from following paragraph
        for j in 0..12 {
            let _ = writeln!(content, "tall line {j}");
        }

        let mut app = test_app(&content);
        // height=15: footer at row 14, outer block rows 0-13, inner rows 1-12 (height=12).
        let mut terminal = Terminal::new(TestBackend::new(40, 15)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();

        // Inner row 11 = terminal row 12.
        let buf = terminal.backend().buffer();
        let row12: String = (0..40)
            .map(|x| {
                buf.cell(ratatui::layout::Position::new(x, 12))
                    .map_or(" ", |c| c.symbol())
            })
            .collect();
        assert!(
            row12.contains("tall line"),
            "partial tall item should render at bottom, got: {row12:?}"
        );
    }

    #[test]
    fn navigating_to_tall_node_bottom_aligns_it() {
        // When the cursor moves to a node taller than the available space,
        // adjust_scroll should bottom-align the node so the last visible lines
        // are at the bottom of the screen — not just show the first line.
        //
        // Layout: 5-line terminal, footer=1, outer=3 (border top+bottom → inner=1).
        // Use a 7-line terminal to get inner=4.
        // Node 0: "before" (1 line, + spacer = 2 rows since next is block start)
        // Node 1: tall paragraph with 6 lines (> inner_height of 4).
        //
        // After navigating to node 1, adjust_scroll must bottom-align it:
        // target_start = max(0, 4-6) = 0 → cursor at top of inner area.
        // The first 4 lines of the tall node fill the screen.
        // Terminal row 1 (inner row 0) should show "tall line 0".
        let content = "before\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\ntall line 5\n";
        let mut app = test_app(content);
        // height=7: footer row 6, outer block rows 0-5, inner rows 1-4 (height=4).
        let mut terminal = Terminal::new(TestBackend::new(40, 7)).unwrap();

        // Navigate to node 1 (the tall paragraph).
        app.move_node(1);
        terminal.draw(|f| app.draw(f)).unwrap();

        let buf = terminal.backend().buffer();
        let inner_rows: Vec<String> = (1..=4)
            .map(|y| {
                (0..40)
                    .map(|x| {
                        buf.cell(ratatui::layout::Position::new(x, y))
                            .map_or(" ", |c| c.symbol())
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert!(
            inner_rows[0].contains("tall line 0"),
            "tall node should be top-aligned (bottom-aligned with 0 context): got {inner_rows:?}"
        );
        assert!(
            inner_rows[3].contains("tall line 3"),
            "last visible inner row should show tall line 3: got {inner_rows:?}"
        );
    }

    #[test]
    fn move_sentence_to_tall_node_bottom_aligns_it() {
        // Same layout as navigating_to_tall_node_bottom_aligns_it, but navigate
        // there via move_sentence (j key) rather than move_node (Down arrow).
        // adjust_scroll fires on every draw() regardless of how cursor moved.
        let content = "before\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\ntall line 5\n";
        let mut app = test_app(content);
        let mut terminal = Terminal::new(TestBackend::new(40, 7)).unwrap();

        // move_sentence forward from node 0's last sentence → lands on node 1.
        app.handle_key(key_char('j'));
        terminal.draw(|f| app.draw(f)).unwrap();

        let buf = terminal.backend().buffer();
        let inner_rows: Vec<String> = (1..=4)
            .map(|y| {
                (0..40)
                    .map(|x| {
                        buf.cell(ratatui::layout::Position::new(x, y))
                            .map_or(" ", |c| c.symbol())
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert_eq!(
            app.selection_state.anchor.node_idx, 1,
            "cursor should be on the tall node"
        );
        assert!(
            inner_rows[0].contains("tall line 0"),
            "move_sentence to tall node must also bottom-align it: got {inner_rows:?}"
        );
        assert!(
            inner_rows[3].contains("tall line 3"),
            "last inner row should show tall line 3: got {inner_rows:?}"
        );
    }

    #[test]
    fn fill_partial_bottom_reveals_more_of_next_node() {
        // Cursor is on node 1 (short paragraph); node 2 is a 5-line paragraph.
        // Without fill_partial_bottom, scroll_offset=0 and only 3 lines of node 2
        // fit on screen. fill_partial_bottom should skip node 0 (scroll_offset→1)
        // so all 5 lines of node 2 become visible.
        //
        // Layout: height=10 → footer at row 9, borders rows 0 & 8, inner rows 1-7 (height=7).
        //   Node 0: "short A"      → 1 content row + 1 spacer = 2 rows
        //   Node 1: "short B..."   → 1 content row + 1 spacer = 2 rows  (cursor)
        //   Node 2: 5-line para    → 5 rows (last node, no spacer)
        //   Total: 9 rows > inner_height=7; node 2 partially hidden without scrolling.
        //
        // After fill_partial_bottom: scroll_offset=1 (node 0 scrolled off).
        //   Node 1: inner rows 0-1, Node 2: inner rows 2-6 (terminal rows 3-7).
        //   Terminal row 7 should show "tall line 4" (the last of 5 node-2 lines).
        let content = "short A\n\nshort B. Next. Third.\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\n";
        let mut app = test_app(content);
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();

        app.move_node(1); // cursor on "short B" node
        terminal.draw(|f| app.draw(f)).unwrap();

        let buf = terminal.backend().buffer();
        let last_inner_row: String = (0..40)
            .map(|x| {
                buf.cell(ratatui::layout::Position::new(x, 7))
                    .map_or(" ", |c| c.symbol())
            })
            .collect::<String>()
            .trim_end()
            .to_string();

        assert!(
            last_inner_row.contains("tall line 4"),
            "fill_partial_bottom should reveal all 5 lines of next node; last inner row got: {last_inner_row:?}"
        );
    }
}
