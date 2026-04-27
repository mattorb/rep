use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::document::{DocNode, Document};
use crate::markdown::{MarkdownLinkRange, render_markdown_line};
use crate::output::clean_context;
use crate::selection::index::SelectionIndex;
use crate::selection::model::{SelectionAnchor, SelectionState, SelectionUnit};
#[cfg(test)]
use crate::output::{
    AgentOutput, ChangeOutput, FeedbackOutput, InsertOutput, KeymapOutput, LineAnnotationOutput,
    LineContext, ReactionOutput,
};
use crate::ui::wrap_styled_spans;

const FOOTER_HEIGHT: u16 = 1;
const GUTTER_WIDTH: usize = 2;

// ── Annotation types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
                let sentence_ranges = sentence_ranges_from_plain(&plain);
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

/// Convenience wrapper that delegates to the canonical segmenter in
/// `selection::segment`. Kept as a local alias to minimize the diff at call
/// sites; remove in phase 6 when sentence-related app tests migrate out.
fn sentence_ranges_from_plain(plain: &str) -> Vec<Range<usize>> {
    crate::selection::segment::segment_sentences(plain)
}

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
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(0);
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

// ── App ───────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct App {
    source_path: PathBuf,
    /// Original source lines — used only for output context (prev/next line).
    source_lines: Vec<String>,
    doc: Document,
    rendered_nodes: Vec<RenderedNode>,
    /// Owned eager selection index built once at load time per Req 11.
    /// Read by phase 3a's navigator extraction; stub-referenced today.
    #[allow(dead_code)]
    pub(crate) index: SelectionIndex,
    /// Canonical selection state — `(node_idx, unit, unit_idx)`. Replaces the
    /// pre-phase-1 `cursor_node` + `cursor_sentence` pair.
    pub(crate) selection_state: SelectionState,
    /// When set by J/K, highlights every node from the section start through the node
    /// before the next section boundary. Cleared on the next j/k/h/l press.
    section_highlight_range: Option<Range<usize>>,
    /// Annotations keyed by node index.
    changes: BTreeMap<usize, Vec<ChangeAnnotation>>,
    feedbacks: BTreeMap<usize, Vec<FeedbackAnnotation>>,
    inserts_before: BTreeMap<usize, Vec<InsertAnnotation>>,
    inserts_after: BTreeMap<usize, Vec<InsertAnnotation>>,
    strikes: BTreeMap<usize, BTreeSet<usize>>,
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
    show_link_popup: bool,
    link_popup_urls: Vec<String>,
    show_help: bool,
    show_ast: bool,
    ast_scroll: u16,
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
        let ast_text = markdown::to_mdast(&raw, &markdown::ParseOptions::default())
            .map(|node| format!("{node:#?}"))
            .unwrap_or_else(|_| "Failed to parse AST".to_string());
        let ast_lines: Vec<String> = ast_text.lines().map(ToOwned::to_owned).collect();
        let doc = Document::parse(&raw);
        let rendered_nodes = build_rendered_nodes(&doc, &source_lines);
        let index = SelectionIndex::build(&doc, &source_lines);

        let initial_node = doc.next_node_with_sentences(0).unwrap_or(0);
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
            show_link_popup: false,
            link_popup_urls: Vec::new(),
            show_help: false,
            show_ast: false,
            ast_scroll: 0,
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
    pub fn current_anchor(&self) -> (usize, &'static str, usize) {
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
                self.handle_edit_change_key(key, node_idx, change_idx)
            }
            InputMode::EditFeedback(node_idx, feedback_idx) => {
                self.handle_edit_feedback_key(key, node_idx, feedback_idx)
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

        if self.show_ast {
            match key.code {
                KeyCode::Esc | KeyCode::Char('i') => {
                    self.show_ast = false;
                    self.status = "Closed AST view.".to_string();
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.ast_scroll = self.ast_scroll.saturating_add(3);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.ast_scroll = self.ast_scroll.saturating_sub(3);
                }
                _ => {}
            }
            return;
        }

        if self.show_link_popup {
            match key.code {
                KeyCode::Esc | KeyCode::Char('u') => {
                    self.show_link_popup = false;
                    self.link_popup_urls.clear();
                    self.status = "Closed link popup.".to_string();
                }
                _ => {
                    self.show_link_popup = false;
                    self.link_popup_urls.clear();
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
            KeyCode::Char('i') => {
                self.show_ast = true;
                self.ast_scroll = 0;
                self.status = "AST view. j/k scroll, i or Esc close.".to_string();
            }
            KeyCode::Char('u') if !self.reveal_links_for_current_sentence() => {
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
                    bucket.entry(self.selection_state.anchor.node_idx).or_default().push(annotation);
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
                cursor = abs + needle.len().max(1);
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
        let current = (self.selection_state.anchor.node_idx, self.selection_state.anchor.unit_idx);
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
        let current = (self.selection_state.anchor.node_idx, self.selection_state.anchor.unit_idx);
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

    fn apply_search_target(&mut self, query: &str, matches: &[(usize, usize)], target_idx: usize) {
        let (ni, si) = matches[target_idx];
        self.selection_state.anchor.node_idx = ni;
        self.selection_state.anchor.unit_idx = si;
        self.section_highlight_range = None;
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

    /// ↓/↑: move through every content node (nodes that have a highlightable sentence).
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
                self.doc.next_node_with_sentences(target.saturating_add(1))
            } else {
                self.doc.prev_node_with_sentences(target)
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
        self.section_highlight_range = None;
        self.clamp_sentence();
        self.status = format!("Node {}/{}", self.selection_state.anchor.node_idx + 1, self.doc.node_count());
    }

    /// j / k / Down / Up / Right / Left — move by the currently active
    /// selection unit. Pure delegate to `selection::navigator::next/prev`.
    /// On `Boundary`, set `nav_feedback` ("at end" / "at start") for one
    /// keypress in the right zone of the footer.
    fn move_active_unit(&mut self, forward: bool) {
        if self.doc.node_count() == 0 {
            return;
        }
        self.section_highlight_range = None;
        let outcome = if forward {
            crate::selection::navigator::next(&self.index, self.selection_state.anchor)
        } else {
            crate::selection::navigator::prev(&self.index, self.selection_state.anchor)
        };
        match outcome {
            crate::selection::model::NavOutcome::Moved(a) => {
                self.selection_state.anchor = a;
                if a.unit == SelectionUnit::Section {
                    let end = self
                        .index
                        .sections
                        .iter()
                        .find(|s| s.start_node_idx == a.node_idx)
                        .map(|s| s.end_node_idx + 1)
                        .unwrap_or_else(|| self.doc.node_count());
                    self.section_highlight_range = Some(a.node_idx..end);
                }
            }
            crate::selection::model::NavOutcome::Boundary => {
                self.nav_feedback = Some(
                    if forward { "at end" } else { "at start" }.to_string(),
                );
            }
        }
    }

    /// Space (forward) / Backspace (reverse) — cycle the active selection
    /// unit. Re-anchors via `navigator::clamp` per the pinned rules.
    fn mode_cycle(&mut self, forward: bool) {
        let order = [
            SelectionUnit::Section,
            SelectionUnit::Paragraph,
            SelectionUnit::Line,
            SelectionUnit::Sentence,
            SelectionUnit::Word,
        ];
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
        let new_anchor = crate::selection::navigator::clamp(
            &self.index,
            self.selection_state.anchor,
            target,
        );
        self.selection_state.anchor = new_anchor;
        // Section mode also lights up the section span highlight.
        if new_anchor.unit == SelectionUnit::Section {
            let end = self
                .index
                .sections
                .iter()
                .find(|s| s.start_node_idx == new_anchor.node_idx)
                .map(|s| s.end_node_idx + 1)
                .unwrap_or_else(|| self.doc.node_count());
            self.section_highlight_range = Some(new_anchor.node_idx..end);
        } else {
            self.section_highlight_range = None;
        }
    }

    /// Stable string for the mode indicator in the left zone of the footer.
    pub fn mode_indicator(&self) -> &'static str {
        match self.selection_state.anchor.unit {
            SelectionUnit::Section => "section",
            SelectionUnit::Paragraph => "paragraph",
            SelectionUnit::Line => "line",
            SelectionUnit::Sentence => "sentence",
            SelectionUnit::Word => "word",
        }
    }

    /// Phase-3a-compat shim still consumed by some legacy app tests; phase 6
    /// retires the move_sentence name with the test migration.
    fn move_sentence(&mut self, forward: bool) {
        let saved_unit = self.selection_state.anchor.unit;
        self.selection_state.anchor.unit = SelectionUnit::Sentence;
        self.move_active_unit(forward);
        self.selection_state.anchor.unit = saved_unit;
    }

    /// J/K: jump to the next/prev section heading or top-level ordered list item.
    /// Phase 3a: thin wrapper around `selection::navigator::next/prev` with
    /// `SelectionUnit::Section`; section_highlight_range is derived from the
    /// section table.
    fn move_section(&mut self, forward: bool) {
        // Build a Section anchor from the current selection. If currently in
        // sentence mode, find the section that contains the current node.
        let current_node = self.selection_state.anchor.node_idx;
        let containing_section_start = self
            .index
            .sections
            .iter()
            .find(|s| s.start_node_idx <= current_node && current_node <= s.end_node_idx)
            .map(|s| s.start_node_idx);
        let section_anchor = SelectionAnchor::new(
            containing_section_start.unwrap_or(current_node),
            SelectionUnit::Section,
            0,
        );
        let outcome = if forward {
            crate::selection::navigator::next(&self.index, section_anchor)
        } else {
            crate::selection::navigator::prev(&self.index, section_anchor)
        };
        match outcome {
            crate::selection::model::NavOutcome::Moved(a) => {
                self.selection_state.anchor.node_idx = a.node_idx;
                self.selection_state.anchor.unit_idx = 0;
                // Look up the section's end to compute the highlight range.
                let end = self
                    .index
                    .sections
                    .iter()
                    .find(|s| s.start_node_idx == a.node_idx)
                    .map(|s| s.end_node_idx + 1)
                    .unwrap_or_else(|| self.doc.node_count());
                self.section_highlight_range = Some(a.node_idx..end);
                self.status = format!("Section at node {}.", a.node_idx + 1);
            }
            crate::selection::model::NavOutcome::Boundary => {
                self.status = if forward {
                    "Already at the last section.".to_string()
                } else {
                    "Already at the first section.".to_string()
                };
            }
        }
    }

    /// H/L: jump to the next/prev content block (entire lists count as one block).
    fn move_block(&mut self, forward: bool) {
        let target = if forward {
            self.doc.next_block(self.selection_state.anchor.node_idx.saturating_add(1))
        } else {
            self.doc.prev_block(self.selection_state.anchor.node_idx)
        };

        match target {
            Some(idx) => {
                self.selection_state.anchor.node_idx = idx;
                self.selection_state.anchor.unit_idx = 0;
                let end = self.doc.block_end(idx);
                self.section_highlight_range = Some(idx..end + 1);
                self.status = format!("Block at node {}.", idx + 1);
            }
            None => {
                self.status = if forward {
                    "Already at the last block.".to_string()
                } else {
                    "Already at the first block.".to_string()
                };
            }
        }
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

    fn clamp_sentence(&mut self) {
        let total = self
            .rendered_nodes
            .get(self.selection_state.anchor.node_idx)
            .map(|rn| rn.sentence_ranges.len())
            .unwrap_or(0);
        if total == 0 {
            self.selection_state.anchor.unit_idx = 0;
        } else {
            self.selection_state.anchor.unit_idx = self.selection_state.anchor.unit_idx.min(total - 1);
        }
    }

    fn current_source_line(&self) -> usize {
        self.doc
            .nodes
            .get(self.selection_state.anchor.node_idx)
            .map(|n| n.source_start_line())
            .unwrap_or(0)
    }

    fn current_sentence_context(&self) -> Option<(usize, String)> {
        let rn = self.rendered_nodes.get(self.selection_state.anchor.node_idx)?;
        let range = rn.sentence_ranges.get(self.selection_state.anchor.unit_idx)?;
        let text = rn.plain.get(range.clone())?.trim().to_string();
        Some((self.selection_state.anchor.unit_idx, text))
    }

    /// Capture the (unit_idx, target_text) snapshot used when storing an
    /// annotation. Routing by `selection_state.anchor.unit`. For phase 4:
    ///   - Sentence: legacy current_sentence_context (rendered display
    ///     plain text slice for the current sentence).
    ///   - Line: per-unit text — for ListItem the full item text
    ///     (markers stripped, soft-wrapped lines space-joined); for any
    ///     other node the source line verbatim.
    ///   - Paragraph / Section / Word: fall through to the sentence
    ///     capture today; phase 5 adds Word; full Paragraph / Section
    ///     emit lands later.
    fn current_target_capture(&self) -> Option<(usize, String)> {
        match self.selection_state.anchor.unit {
            SelectionUnit::Line => self.current_line_capture(),
            _ => self.current_sentence_context(),
        }
    }

    fn current_line_capture(&self) -> Option<(usize, String)> {
        let node_idx = self.selection_state.anchor.node_idx;
        let unit_idx = self.selection_state.anchor.unit_idx;
        match self.doc.nodes.get(node_idx)? {
            DocNode::ListItem { .. } => {
                // ListItem at line unit: full item text, markers already
                // stripped by the index's selection_plain_text.
                let plain = self
                    .index
                    .nodes
                    .get(node_idx)
                    .map(|n| n.selection_plain_text.clone())?;
                Some((unit_idx, plain))
            }
            _ => {
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
    }

    // ── Annotations ───────────────────────────────────────────────────────────

    fn existing_change_for_cursor(&self) -> Option<usize> {
        let sentence_idx = self.current_sentence_context().map(|(idx, _)| idx);
        let changes = self.changes.get(&self.selection_state.anchor.node_idx)?;
        if let Some(idx) = sentence_idx {
            changes.iter().rposition(|c| c.sentence_index == Some(idx))
        } else {
            changes.len().checked_sub(1)
        }
    }

    fn existing_feedback_for_cursor(&self) -> Option<usize> {
        let sentence_idx = self.current_sentence_context().map(|(idx, _)| idx);
        let feedbacks = self.feedbacks.get(&self.selection_state.anchor.node_idx)?;
        if let Some(idx) = sentence_idx {
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
            self.input_mode = InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
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
            self.input_mode = InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
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
                self.input_mode = InputMode::EditChange(self.selection_state.anchor.node_idx, change_idx);
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
                self.input_mode = InputMode::EditFeedback(self.selection_state.anchor.node_idx, feedback_idx);
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
        let sentence_idx = self.current_sentence_context().map(|(idx, _)| idx);

        let sentence_match = sentence_idx.and_then(|idx| {
            let change = self.changes.get(&self.selection_state.anchor.node_idx).and_then(|changes| {
                changes
                    .iter()
                    .rposition(|c| c.sentence_index == Some(idx))
                    .map(|change_idx| (change_idx, &changes[change_idx]))
            });
            let feedback = self.feedbacks.get(&self.selection_state.anchor.node_idx).and_then(|feedbacks| {
                feedbacks
                    .iter()
                    .rposition(|f| f.sentence_index == Some(idx))
                    .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
            });
            Self::pick_editable_annotation(change, feedback)
        });

        sentence_match.or_else(|| {
            let change = self.changes.get(&self.selection_state.anchor.node_idx).and_then(|changes| {
                changes
                    .len()
                    .checked_sub(1)
                    .map(|change_idx| (change_idx, &changes[change_idx]))
            });
            let feedback = self.feedbacks.get(&self.selection_state.anchor.node_idx).and_then(|feedbacks| {
                feedbacks
                    .len()
                    .checked_sub(1)
                    .map(|feedback_idx| (feedback_idx, &feedbacks[feedback_idx]))
            });
            Self::pick_editable_annotation(change, feedback)
        })
    }

    fn remove_selected_annotation(&mut self) -> bool {
        match self.editable_annotation_at_cursor() {
            Some(EditableAnnotation::Change(change_idx)) => {
                let mut removed = false;
                let mut empty = false;
                if let Some(changes) = self.changes.get_mut(&self.selection_state.anchor.node_idx)
                    && change_idx < changes.len()
                {
                    changes.remove(change_idx);
                    removed = true;
                    empty = changes.is_empty();
                }
                if removed {
                    if empty {
                        self.changes.remove(&self.selection_state.anchor.node_idx);
                    }
                    self.status = format!("Removed change from node {}.", self.selection_state.anchor.node_idx + 1);
                }
                removed
            }
            Some(EditableAnnotation::Feedback(feedback_idx)) => {
                let mut removed = false;
                let mut empty = false;
                if let Some(feedbacks) = self.feedbacks.get_mut(&self.selection_state.anchor.node_idx)
                    && feedback_idx < feedbacks.len()
                {
                    feedbacks.remove(feedback_idx);
                    removed = true;
                    empty = feedbacks.is_empty();
                }
                if removed {
                    if empty {
                        self.feedbacks.remove(&self.selection_state.anchor.node_idx);
                    }
                    self.status = format!("Removed feedback from node {}.", self.selection_state.anchor.node_idx + 1);
                }
                removed
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

        let Some((sentence_idx, _)) = self.current_sentence_context() else {
            self.status = format!("Node {} has no sentence to strike.", self.selection_state.anchor.node_idx + 1);
            return;
        };

        let entry = self.strikes.entry(self.selection_state.anchor.node_idx).or_default();
        if entry.contains(&sentence_idx) {
            entry.remove(&sentence_idx);
            let node = self.selection_state.anchor.node_idx;
            if entry.is_empty() {
                self.strikes.remove(&node);
            }
            self.status = format!(
                "Removed strike from node {}, sentence {}.",
                self.selection_state.anchor.node_idx + 1,
                sentence_idx + 1
            );
        } else {
            entry.insert(sentence_idx);
            self.status = format!(
                "Struck node {}, sentence {}.",
                self.selection_state.anchor.node_idx + 1,
                sentence_idx + 1
            );
        }
    }

    fn reveal_links_for_current_sentence(&mut self) -> bool {
        let urls = self.current_sentence_links();
        if urls.is_empty() {
            return false;
        }
        self.link_popup_urls = urls;
        self.show_link_popup = true;
        self.status = format!(
            "Showing {} link(s) from current sentence.",
            self.link_popup_urls.len()
        );
        true
    }

    fn current_sentence_links(&self) -> Vec<String> {
        let Some(rn) = self.rendered_nodes.get(self.selection_state.anchor.node_idx) else {
            return Vec::new();
        };
        let Some(range) = rn.sentence_ranges.get(self.selection_state.anchor.unit_idx) else {
            return Vec::new();
        };
        let mut urls = Vec::new();
        for link in &rn.links {
            let overlaps = link.end > range.start && link.start < range.end;
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
                    let mut display_lines: Vec<Line> = raw
                        .iter()
                        .enumerate()
                        .map(|(i, line)| {
                            let style = if line.trim_start().starts_with("```") {
                                Style::default().fg(Color::DarkGray)
                            } else {
                                Style::default().fg(Color::White).bg(Color::DarkGray)
                            };
                            let mut spans = vec![if i == 0 {
                                Span::styled(format!("{indicator} "), indicator_style)
                            } else {
                                Span::raw("  ")
                            }];
                            spans.push(Span::styled(line.clone(), style));
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
        let right_text = if let Some(fb) = &self.nav_feedback {
            (fb.clone(), Style::default().fg(Color::Yellow))
        } else if let Some(note) = &self.notification {
            (note.clone(), Style::default().fg(Color::Green))
        } else {
            ("? for help ".to_string(), hint_style)
        };
        let total_width = layout[1].width as usize;
        let mode_w = mode_text.len();
        let right_w_target = right_text.0.len().min(total_width.saturating_sub(mode_w + 1));
        let right_str: String = right_text.0.chars().take(right_w_target).collect();
        let gap = total_width.saturating_sub(mode_w + right_str.len());
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

        if self.show_link_popup {
            self.draw_link_popup(frame, area);
        }

        if self.show_help {
            self.draw_help(frame, area);
        }

        if self.show_ast {
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
        if list_inner.width < 12 || list_inner.height < 4 || self.selection_state.anchor.node_idx >= heights.len() {
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

    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        let help_lines = vec![
            Line::from(Span::styled(
                "  nav  next/prev",
                Style::default().fg(Color::Cyan),
            )),
            Line::from("  j/k  line         h/l  sentence"),
            Line::from("  J/K  section      H/L  paragraph"),
            Line::from("  ]/[  annotation"),
            Line::from(""),
            Line::from("  i  AST view        u  links"),
            Line::from("  /  search          n/N  next/prev match"),
            Line::from("  c  change (literal)"),
            Line::from("  f  feedback (intent)"),
            Line::from("  b  a  insert before · after"),
            Line::from("  e  x  r  edit · clear/strike · copy result"),
            Line::from("  q  Q          quit · silent quit"),
            Line::from("  ? / Esc       help · close"),
        ];

        let content_width: u16 = help_lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.chars().count())
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
        let scroll = self.ast_scroll.min(max_scroll);

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
        let popup_width = area.width.saturating_sub(10).clamp(40, 100);
        let max_height = area.height.saturating_sub(6).max(6);
        let desired_height = (self.link_popup_urls.len() as u16)
            .saturating_add(5)
            .clamp(6, max_height);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(popup_width) / 2,
            y: area.y + area.height.saturating_sub(desired_height) / 2,
            width: popup_width,
            height: desired_height,
        };

        let mut lines = Vec::new();
        lines.push(Line::from("Links in current sentence:"));
        lines.push(Line::from(""));
        for (idx, url) in self.link_popup_urls.iter().enumerate() {
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

        let cursor_height = heights.get(self.selection_state.anchor.node_idx).copied().unwrap_or(1);
        let rows_before: u16 = heights
            .get(self.scroll_offset..self.selection_state.anchor.node_idx)
            .map(|s| s.iter().copied().sum())
            .unwrap_or(0);

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
        let cursor_h = heights.get(self.selection_state.anchor.node_idx).copied().unwrap_or(1);
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
                .map(|s| s.iter().copied().sum())
                .unwrap_or(0);
            if rows_before_cursor + cursor_h > inner_height {
                break; // cursor would go off-screen
            }
            skipped += h;
            new_offset = candidate;
        }
        self.scroll_offset = new_offset;
    }

    fn node_indicator(&self, node_idx: usize) -> (&'static str, Style) {
        let change_count = self.changes.get(&node_idx).map(|v| v.len()).unwrap_or(0);
        let feedback_count = self.feedbacks.get(&node_idx).map(|v| v.len()).unwrap_or(0);
        let insert_count = self
            .inserts_before
            .get(&node_idx)
            .map(|v| v.len())
            .unwrap_or(0)
            + self
                .inserts_after
                .get(&node_idx)
                .map(|v| v.len())
                .unwrap_or(0);
        let strike_count = self.strikes.get(&node_idx).map(|v| v.len()).unwrap_or(0);

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
        let strikes = self.strikes.get(&node_idx);

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

        // Collect all split points.
        let mut bounds = vec![0, plain_len];
        for &(s, e, _) in &seg {
            bounds.push(s);
            bounds.push(e);
        }
        for r in sentence_ranges {
            bounds.push(r.start.min(plain_len));
            bounds.push(r.end.min(plain_len));
        }
        bounds.sort_unstable();
        bounds.dedup();

        let highlight = if self
            .section_highlight_range
            .as_ref()
            .is_some_and(|r| r.contains(&node_idx))
        {
            Some(0..plain_len)
        } else if node_idx == self.selection_state.anchor.node_idx {
            sentence_ranges.get(self.selection_state.anchor.unit_idx).cloned()
        } else {
            None
        };

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

            let sentence_idx = sentence_ranges
                .iter()
                .position(|r| start >= r.start && start < r.end);

            if highlight
                .as_ref()
                .map(|r| start < r.end && end > r.start)
                .unwrap_or(false)
            {
                style = style.patch(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                );
            }

            if sentence_idx
                .map(|idx| strikes.map(|s| s.contains(&idx)).unwrap_or(false))
                .unwrap_or(false)
            {
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
    pub fn to_output(&self) -> AgentOutput {
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
                        .map(|&sidx| {
                            let sentence_text = self
                                .rendered_nodes
                                .get(node_idx)
                                .and_then(|rn| rn.sentence_ranges.get(sidx))
                                .and_then(|r| self.rendered_nodes[node_idx].plain.get(r.clone()))
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            ReactionOutput {
                                kind: "strike".to_string(),
                                sentence_index: sidx + 1,
                                sentence_text,
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
                line_prev: "k".to_string(),
                line_next: "j".to_string(),
                sentence_prev: "h".to_string(),
                sentence_next: "l".to_string(),
                reveal_link: "u".to_string(),
                section_prev: "K".to_string(),
                section_next: "J".to_string(),
                paragraph_prev: "H".to_string(),
                paragraph_next: "L".to_string(),
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
        out.push_str(&format!("FILE: {}\n", self.source_path.display()));

        if touched.is_empty() {
            out.push_str("\nNo actions.\n");
            return out;
        }

        for node_idx in touched {
            let (source_line, line_text) = self.node_line_context(node_idx);

            let prev = source_line
                .checked_sub(1)
                .and_then(|i| self.source_lines.get(i))
                .map(String::as_str)
                .unwrap_or("");
            let next = self
                .source_lines
                .get(source_line + 1)
                .map(String::as_str)
                .unwrap_or("");

            let prev_clean = clean_context(prev, 140);
            let line_clean = clean_context(&line_text, 180);
            let next_clean = clean_context(next, 140);

            if let Some(changes) = self.changes.get(&node_idx) {
                for change in changes {
                    let (where_line, sentence_suffix) = self.where_for_annotation(
                        change.target_unit,
                        node_idx,
                        change.sentence_index,
                        source_line,
                    );
                    let target = change
                        .sentence_text
                        .as_deref()
                        .map(|s| clean_context(s, 180))
                        .unwrap_or_else(|| line_clean.clone());
                    let (prev_clean_line, next_clean_line) =
                        self.context_lines(where_line);
                    out.push('\n');
                    out.push_str("ACTION: change\n");
                    out.push_str(&format!(
                        "WHERE: line {}{}\n",
                        where_line + 1,
                        sentence_suffix
                    ));
                    out.push_str("CONTEXT:\n");
                    if !prev_clean_line.is_empty() {
                        out.push_str(&format!("  prev: \"{prev_clean_line}\"\n"));
                    }
                    out.push_str(&format!("  target: \"{target}\"\n"));
                    if !next_clean_line.is_empty() {
                        out.push_str(&format!("  next: \"{next_clean_line}\"\n"));
                    }
                    out.push_str(&format!(
                        "CHANGE: \"{}\"\n",
                        clean_context(&change.change, 220)
                    ));
                }
            }

            if let Some(feedbacks) = self.feedbacks.get(&node_idx) {
                for feedback in feedbacks {
                    let (where_line, sentence_suffix) = self.where_for_annotation(
                        feedback.target_unit,
                        node_idx,
                        feedback.sentence_index,
                        source_line,
                    );
                    let target = feedback
                        .sentence_text
                        .as_deref()
                        .map(|s| clean_context(s, 180))
                        .unwrap_or_else(|| line_clean.clone());
                    let (prev_clean_line, next_clean_line) =
                        self.context_lines(where_line);
                    out.push('\n');
                    out.push_str("ACTION: revise-to-incorporate-feedback\n");
                    out.push_str(&format!(
                        "WHERE: line {}{}\n",
                        where_line + 1,
                        sentence_suffix
                    ));
                    out.push_str("CONTEXT:\n");
                    if !prev_clean_line.is_empty() {
                        out.push_str(&format!("  prev: \"{prev_clean_line}\"\n"));
                    }
                    out.push_str(&format!("  target: \"{target}\"\n"));
                    if !next_clean_line.is_empty() {
                        out.push_str(&format!("  next: \"{next_clean_line}\"\n"));
                    }
                    out.push_str(&format!(
                        "FEEDBACK: \"{}\"\n",
                        clean_context(&feedback.feedback, 220)
                    ));
                }
            }

            for (action, bucket) in [
                ("insert-before", self.inserts_before.get(&node_idx)),
                ("insert-after", self.inserts_after.get(&node_idx)),
            ] {
                let Some(inserts) = bucket else { continue };
                for insert in inserts {
                    let (where_line, sentence_suffix) = self.where_for_annotation(
                        insert.target_unit,
                        node_idx,
                        insert.sentence_index,
                        source_line,
                    );
                    let target = insert
                        .sentence_text
                        .as_deref()
                        .map(|s| clean_context(s, 180))
                        .unwrap_or_else(|| line_clean.clone());
                    let (prev_clean_line, next_clean_line) =
                        self.context_lines(where_line);
                    out.push('\n');
                    out.push_str(&format!("ACTION: {action}\n"));
                    out.push_str(&format!(
                        "WHERE: line {}{}\n",
                        where_line + 1,
                        sentence_suffix
                    ));
                    out.push_str("CONTEXT:\n");
                    if !prev_clean_line.is_empty() {
                        out.push_str(&format!("  prev: \"{prev_clean_line}\"\n"));
                    }
                    out.push_str(&format!("  target: \"{target}\"\n"));
                    if !next_clean_line.is_empty() {
                        out.push_str(&format!("  next: \"{next_clean_line}\"\n"));
                    }
                    out.push_str(&format!(
                        "INSERT: \"{}\"\n",
                        clean_context(&insert.text, 220)
                    ));
                }
            }

            if let Some(strikes) = self.strikes.get(&node_idx) {
                for &sentence_idx in strikes {
                    let sentence_text = self
                        .rendered_nodes
                        .get(node_idx)
                        .and_then(|rn| rn.sentence_ranges.get(sentence_idx))
                        .and_then(|r| self.rendered_nodes[node_idx].plain.get(r.clone()))
                        .map(|s| clean_context(s, 180))
                        .unwrap_or_else(|| line_clean.clone());
                    out.push('\n');
                    out.push_str("ACTION: delete this\n");
                    out.push_str(&format!(
                        "WHERE: line {}, sentence {}\n",
                        source_line + 1,
                        sentence_idx + 1
                    ));
                    out.push_str("CONTEXT:\n");
                    if !prev_clean.is_empty() {
                        out.push_str(&format!("  prev: \"{prev_clean}\"\n"));
                    }
                    out.push_str(&format!("  target: \"{sentence_text}\"\n"));
                    if !next_clean.is_empty() {
                        out.push_str(&format!("  next: \"{next_clean}\"\n"));
                    }
                }
            }
        }

        out
    }

    fn node_line_context(&self, node_idx: usize) -> (usize, String) {
        let source_line = self
            .doc
            .nodes
            .get(node_idx)
            .map(|n| n.source_start_line())
            .unwrap_or(0);
        let line_text = self
            .source_lines
            .get(source_line)
            .cloned()
            .unwrap_or_default();
        (source_line, line_text)
    }

    /// Returns `(where_line: usize, sentence_suffix: String)` for an
    /// annotation. For Sentence-unit annotations, `sentence_suffix` carries
    /// the legacy `, sentence M` suffix; phase 5 strips it. For Line-unit
    /// annotations, the where-line is the specific source line and the
    /// suffix is empty. Other units fall through to sentence semantics for
    /// now.
    fn where_for_annotation(
        &self,
        target_unit: SelectionUnit,
        node_idx: usize,
        sentence_index: Option<usize>,
        node_first_line: usize,
    ) -> (usize, String) {
        match target_unit {
            SelectionUnit::Line => {
                let unit_idx = sentence_index.unwrap_or(0);
                let where_line = self
                    .index
                    .nodes
                    .get(node_idx)
                    .and_then(|n| n.source_line_ranges.get(unit_idx).map(|p| p.0))
                    .unwrap_or(node_first_line);
                (where_line, String::new())
            }
            SelectionUnit::Sentence => {
                let suffix = sentence_index
                    .map(|i| format!(", sentence {}", i + 1))
                    .unwrap_or_default();
                (node_first_line, suffix)
            }
            _ => {
                let suffix = sentence_index
                    .map(|i| format!(", sentence {}", i + 1))
                    .unwrap_or_default();
                (node_first_line, suffix)
            }
        }
    }

    fn context_lines(&self, source_line: usize) -> (String, String) {
        let prev = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .map(String::as_str)
            .unwrap_or("");
        let next = self
            .source_lines
            .get(source_line + 1)
            .map(String::as_str)
            .unwrap_or("");
        (clean_context(prev, 140), clean_context(next, 140))
    }
}

// ── Clipboard ─────────────────────────────────────────────────────────────────

enum ClipboardOutcome {
    OsCommand,
    Osc52,
    Failed,
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
                    .map(|c| c.symbol())
                    .unwrap_or(" ")
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
        app.strikes.entry(0).or_default().insert(0);
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
        app.strikes.entry(0).or_default().insert(0);
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
        app.strikes.entry(0).or_default().insert(0);
        let t = render(&mut app);
        let top = row(&t, 0);
        assert!(top.contains("1C"), "change count missing: {top:?}");
        assert!(top.contains("1F"), "feedback count missing: {top:?}");
        assert!(top.contains("1X"), "strike count missing: {top:?}");
    }

    #[test]
    fn footer_shows_mode_indicator_and_help_hint() {
        let mut app = test_app("line\n");
        let t = render(&mut app);
        let hint_row = row(&t, 23);
        assert!(
            hint_row.contains("mode: sentence"),
            "mode indicator missing in left zone: {hint_row:?}"
        );
        assert!(
            hint_row.contains("? for help"),
            "help hint missing in right zone: {hint_row:?}"
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
    fn human_output_emits_insert_before_action() {
        let mut app = test_app("A sentence here.\n");
        app.inserts_before
            .entry(0)
            .or_default()
            .push(InsertAnnotation {
                created_at: "2026-01-01T00:00:00Z".into(),
                target_unit: SelectionUnit::Sentence,
                sentence_index: Some(0),
                sentence_text: Some("A sentence here.".into()),
                text: "Prologue line.".into(),
            });
        let out = app.to_human_output();
        assert!(out.contains("ACTION: insert-before"), "{out}");
        assert!(out.contains("INSERT: \"Prologue line.\""), "{out}");
        assert!(out.contains("target: \"A sentence here.\""), "{out}");
    }

    #[test]
    fn human_output_emits_insert_after_action() {
        let mut app = test_app("A sentence here.\n");
        app.inserts_after
            .entry(0)
            .or_default()
            .push(InsertAnnotation {
                created_at: "2026-01-01T00:00:00Z".into(),
                target_unit: SelectionUnit::Sentence,
                sentence_index: Some(0),
                sentence_text: Some("A sentence here.".into()),
                text: "Followup line.".into(),
            });
        let out = app.to_human_output();
        assert!(out.contains("ACTION: insert-after"), "{out}");
        assert!(out.contains("INSERT: \"Followup line.\""), "{out}");
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
        assert_eq!(app.selection_state.anchor.node_idx, 1, "cursor should land on Beta node");
        assert!(app.status.contains("Match 1/1"), "status: {}", app.status);
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
        assert_eq!(app.selection_state.anchor.node_idx, 0, "n should wrap to first match");
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
        assert_eq!(app.selection_state.anchor.unit_idx, 0, "should land on first sentence");
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
    fn human_output_emits_revise_action_for_feedback() {
        let mut app = test_app("A sentence here.\n");
        app.feedbacks
            .entry(0)
            .or_default()
            .push(make_feedback("make this clearer"));
        let out = app.to_human_output();
        assert!(
            out.contains("ACTION: revise-to-incorporate-feedback"),
            "{out}"
        );
        assert!(out.contains("FEEDBACK: \"make this clearer\""), "{out}");
    }

    #[test]
    fn human_output_change_format_with_sentence_text_as_target() {
        let mut app = test_app("Alpha. Beta.\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(0),
            sentence_text: Some("Alpha.".into()),
            change: "Rewrite alpha".into(),
        });
        let out = app.to_human_output();
        assert!(out.contains("ACTION: change"), "{out}");
        assert!(out.contains("CHANGE: \"Rewrite alpha\""), "{out}");
        assert!(out.contains("target: \"Alpha.\""), "{out}");
    }

    #[test]
    fn human_output_strike_emits_delete_action_and_extracts_sentence_text() {
        let mut app = test_app("Strike this. Keep this.\n");
        app.strikes.entry(0).or_default().insert(0);
        let out = app.to_human_output();
        assert!(out.contains("ACTION: delete this"), "{out}");
        assert!(out.contains("WHERE: line 1, sentence 1"), "{out}");
        assert!(out.contains("\"Strike this.\""), "{out}");
    }

    #[test]
    fn human_output_sentence_index_in_where_is_one_based() {
        // sentence_index stored 0-based; WHERE must show 1-based for consumers
        let mut app = test_app("First. Second. Third.\n");
        app.changes.entry(0).or_default().push(ChangeAnnotation {
            created_at: "2026-01-01T00:00:00Z".into(),
            target_unit: SelectionUnit::Sentence,
            sentence_index: Some(2), // 0-based: third sentence
            sentence_text: Some("Third.".into()),
            change: "Fix third".into(),
        });
        let out = app.to_human_output();
        assert!(out.contains("sentence 3"), "should be 1-based: {out}");
        assert!(
            !out.contains("sentence 2"),
            "should not show 0-based offset: {out}"
        );
    }

    #[test]
    fn human_output_with_no_annotations_contains_no_actions() {
        let app = test_app("Some text.\n");
        let out = app.to_human_output();
        assert!(out.contains("No actions."), "{out}");
        assert!(!out.contains("ACTION:"), "{out}");
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
            app.changes.get(&0).is_none(),
            "first x should remove the existing change"
        );
        assert!(
            app.strikes.get(&0).is_none(),
            "first x should not strike when removing a change"
        );

        app.handle_key(key_char('x'));
        assert!(
            app.strikes.get(&0).is_some_and(|set| set.contains(&0)),
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
            app.feedbacks.get(&0).is_none(),
            "first x should remove the existing feedback"
        );
        assert!(
            app.strikes.get(&0).is_none(),
            "first x should not strike when removing feedback"
        );
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    #[test]
    fn sentence_navigation_crosses_node_boundary() {
        // Blank lines don't exist as nodes — moving 'l' from last sentence of
        // node 0 should land on first sentence of node 1.
        let mut app = test_app("First sentence.\n\nSecond sentence.\n");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        assert_eq!(app.selection_state.anchor.unit_idx, 0);
        app.move_sentence(true);
        assert_eq!(app.selection_state.anchor.node_idx, 1, "should cross to next node");
        assert_eq!(app.selection_state.anchor.unit_idx, 0);
    }

    #[test]
    fn node_navigation_moves_through_every_node() {
        // "one", blank, blank, "two", blank, "three" → 3 Paragraph nodes.
        let mut app = test_app("one\n\n\ntwo\n\nthree\n");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        app.move_node(1);
        assert_eq!(app.selection_state.anchor.node_idx, 1, "should move to next node");
        app.move_node(1);
        assert_eq!(app.selection_state.anchor.node_idx, 2, "should move to next node");
        app.move_node(-1);
        assert_eq!(app.selection_state.anchor.node_idx, 1, "should move back");
    }

    #[test]
    fn block_navigation_jumps_between_content_nodes() {
        // Title / Para / ListItem / Tail → nodes 0..3
        let mut app =
            test_app("Title\n\nPara one line one\nline two\n\n- list item\nwrapped\n\nTail\n");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        app.move_block(true);
        assert_eq!(app.selection_state.anchor.node_idx, 1, "should jump to next content node");
        app.move_block(true);
        assert_eq!(app.selection_state.anchor.node_idx, 2, "should jump to list item");
        app.move_block(true);
        assert_eq!(app.selection_state.anchor.node_idx, 3, "should jump to tail");
        app.move_block(false);
        assert_eq!(app.selection_state.anchor.node_idx, 2, "should jump back");
    }

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

    // ── sentence_ranges_from_plain: byte-range values ────────────────────────

    #[test]
    fn sentence_ranges_from_plain_byte_ranges_slice_correctly() {
        let text = "First sentence. Second sentence.";
        let ranges = sentence_ranges_from_plain(text);
        assert_eq!(ranges.len(), 2, "{ranges:?}");
        assert_eq!(&text[ranges[0].clone()], "First sentence.");
        assert_eq!(&text[ranges[1].clone()], "Second sentence.");
    }

    // ── sentence_ranges_from_plain: hard-wrap continuation ────────────────────

    #[test]
    fn hard_wrap_lowercase_continuation_is_not_split() {
        let ranges = sentence_ranges_from_plain("First line\ncontinuation here");
        assert_eq!(
            ranges.len(),
            1,
            "lowercase after newline must not split: {ranges:?}"
        );
    }

    #[test]
    fn hard_wrap_uppercase_after_newline_does_split() {
        let ranges = sentence_ranges_from_plain("First line\nSecond line");
        assert_eq!(
            ranges.len(),
            2,
            "uppercase after newline must split: {ranges:?}"
        );
    }

    #[test]
    fn hard_wrap_indented_lowercase_continuation_no_split() {
        let ranges = sentence_ranges_from_plain("Some text\n  continued here");
        assert_eq!(
            ranges.len(),
            1,
            "indented lowercase continuation must not split: {ranges:?}"
        );
    }

    #[test]
    fn hard_wrap_indented_uppercase_splits() {
        let ranges = sentence_ranges_from_plain("Some text\n  Capitalized here");
        assert_eq!(ranges.len(), 2, "indented uppercase must split: {ranges:?}");
    }

    // ── move_sentence boundary conditions ─────────────────────────────────────

    #[test]
    fn move_sentence_forward_at_last_sentence_stays_put() {
        let mut app = test_app("Single sentence.");
        app.move_sentence(true);
        assert_eq!(app.selection_state.anchor.node_idx, 0, "no next node — cursor must stay");
        assert_eq!(app.selection_state.anchor.unit_idx, 0);
    }

    #[test]
    fn move_sentence_backward_at_first_sentence_stays_put() {
        let mut app = test_app("Single sentence.");
        app.move_sentence(false);
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        assert_eq!(app.selection_state.anchor.unit_idx, 0);
    }

    #[test]
    fn move_sentence_forward_stays_on_last_sentence_of_multi_sentence_node() {
        let mut app = test_app("One. Two. Three.");
        app.selection_state.anchor.unit_idx = 2;
        app.move_sentence(true);
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        assert_eq!(app.selection_state.anchor.unit_idx, 2, "should stay on last sentence");
    }

    #[test]
    fn move_sentence_backward_crosses_to_last_sentence_of_previous_node() {
        // node 0 has 2 sentences; node 1 has 1.
        let mut app = test_app("First. Second.\n\nThird.\n");
        app.move_sentence(true); // → node 0, sentence 1
        app.move_sentence(true); // → node 1, sentence 0
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        app.move_sentence(false); // ← should land on last sentence of node 0
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        assert_eq!(
            app.selection_state.anchor.unit_idx, 1,
            "must land on last sentence of previous node"
        );
    }

    #[test]
    fn move_sentence_clears_section_highlight_range() {
        let mut app = test_app("Intro\n\n# A\ntext\n");
        app.move_section(true);
        assert!(
            app.section_highlight_range.is_some(),
            "should be set after move_section"
        );
        app.move_sentence(true);
        assert!(
            app.section_highlight_range.is_none(),
            "should be cleared after move_sentence"
        );
    }

    #[test]
    fn move_node_clamps_cursor_sentence_to_destination_node_length() {
        // node 0: 3 sentences; node 1: 1 sentence
        let mut app = test_app("One. Two. Three.\n\nSingle.\n");
        app.selection_state.anchor.unit_idx = 2;
        app.move_node(1);
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        assert_eq!(
            app.selection_state.anchor.unit_idx, 0,
            "cursor_sentence must clamp to last valid index"
        );
    }

    #[test]
    fn move_node_skips_thematic_break() {
        // nodes: [Para, ThematicBreak, Para]
        let mut app = test_app("First paragraph.\n\n---\n\nSecond paragraph.\n");
        assert_eq!(app.selection_state.anchor.node_idx, 0);
        app.move_node(1);
        assert_eq!(app.selection_state.anchor.node_idx, 2, "should skip ThematicBreak at node 1");
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
    fn move_section_in_doc_with_no_sections_stays_put() {
        let mut app = test_app("Just a paragraph. No headings.");
        let start = app.selection_state.anchor.node_idx;
        app.move_section(true);
        assert_eq!(
            app.selection_state.anchor.node_idx, start,
            "no section found — cursor must not move"
        );
        app.move_section(false);
        assert_eq!(app.selection_state.anchor.node_idx, start);
    }

    #[test]
    fn move_block_forward_at_last_block_stays_put() {
        let mut app = test_app("Only one block.");
        let start = app.selection_state.anchor.node_idx;
        app.move_block(true);
        assert_eq!(
            app.selection_state.anchor.node_idx, start,
            "already at last block — must not move"
        );
    }

    #[test]
    fn move_section_sets_highlight_range_to_section_boundary() {
        // nodes: [Para(intro), Heading(A), Para(text), Heading(B), Para(final)]
        let mut app = test_app("Intro\n\n# A\ntext\n\n# B\nfinal\n");
        app.move_section(true); // jumps to Heading A = node 1
        assert_eq!(app.selection_state.anchor.node_idx, 1);
        let range = app
            .section_highlight_range
            .clone()
            .expect("highlight should be set");
        assert_eq!(range.start, 1);
        assert_eq!(range.end, 3, "section A spans nodes 1..3");

        app.move_section(true); // jumps to Heading B = node 3
        assert_eq!(app.selection_state.anchor.node_idx, 3);
        let range = app
            .section_highlight_range
            .clone()
            .expect("highlight should be set");
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
            content.push_str(&format!("- Item {i}\n"));
        }
        content.push('\n'); // blank line separates list from following paragraph
        for j in 0..12 {
            content.push_str(&format!("tall line {j}\n"));
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
                    .map(|c| c.symbol())
                    .unwrap_or(" ")
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
                            .map(|c| c.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert!(
            inner_rows[0].contains("tall line 0"),
            "tall node should be top-aligned (bottom-aligned with 0 context): got {:?}",
            inner_rows
        );
        assert!(
            inner_rows[3].contains("tall line 3"),
            "last visible inner row should show tall line 3: got {:?}",
            inner_rows
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
        app.move_sentence(true);
        terminal.draw(|f| app.draw(f)).unwrap();

        let buf = terminal.backend().buffer();
        let inner_rows: Vec<String> = (1..=4)
            .map(|y| {
                (0..40)
                    .map(|x| {
                        buf.cell(ratatui::layout::Position::new(x, y))
                            .map(|c| c.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();

        assert_eq!(app.selection_state.anchor.node_idx, 1, "cursor should be on the tall node");
        assert!(
            inner_rows[0].contains("tall line 0"),
            "move_sentence to tall node must also bottom-align it: got {:?}",
            inner_rows
        );
        assert!(
            inner_rows[3].contains("tall line 3"),
            "last inner row should show tall line 3: got {:?}",
            inner_rows
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
                    .map(|c| c.symbol())
                    .unwrap_or(" ")
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
