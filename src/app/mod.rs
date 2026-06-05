use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::fmt::Write;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[cfg(test)]
use crate::document::DocNode;
use crate::document_view::{
    CodeBlockLineStyleRequest, DisplaySpanStyleRequest, DocumentView, SourceActionContext,
};
use crate::output::{
    EmitAction, EmitActionContext, EmitChange, EmitFeedback, EmitInsert, EmitKeymap,
    EmitLineAnnotation, EmitLineContext, EmitModel, EmitPayload, EmitReaction, clean_context,
    render_human_output,
};
use crate::selection::model::{SelectionAnchor, SelectionState, SelectionUnit};
use crate::ui::wrap_styled_spans;

mod input;
mod output;
mod render;
mod state;

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
    created_at: String,
    target_unit: SelectionUnit,
    sentence_index: Option<usize>,
    sentence_text: Option<String>,
    text: String,
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
    /// Parsed source and derived document views.
    view: DocumentView,
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
    /// True after `q` is pressed but before the user has confirmed or
    /// cancelled. While pending, `handle_normal_key` only accepts keys
    /// that resolve the prompt (y/Y/q/Enter to confirm, n/N/Esc to
    /// cancel); other keys are ignored so the user can re-read the
    /// prompt without losing it. `Q` and Ctrl-C bypass confirmation
    /// entirely (already-deliberate exits).
    quit_confirm_pending: bool,
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
    /// Timestamp + position of the most recent left-click, used to
    /// bump the click count on rapid same-cell clicks (single → double
    /// → triple). Reset on cell change or after the timeout window.
    last_click: Option<LastClick>,
}

#[derive(Debug, Clone, Copy)]
struct LastClick {
    at: Instant,
    row: u16,
    col: u16,
    /// 1 = single, 2 = double, 3 = triple. Saturates at 3 — a fourth
    /// rapid click on the same cell drops back to 1.
    count: u8,
}

const CLICK_DOUBLE_INTERVAL: Duration = Duration::from_millis(500);

impl App {
    pub fn load(path: PathBuf) -> Result<Self> {
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read markdown file: {}", path.display()))?;

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
        let view = DocumentView::parse(&raw)?;

        let initial_node = view.next_content_node(0).unwrap_or(0);
        let selection_state = SelectionState::new(SelectionAnchor::new(
            initial_node,
            SelectionUnit::Sentence,
            0,
        ));

        Ok(Self {
            source_path: path,
            view,
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
            quit_confirm_pending: false,
            link_popup_urls: None,
            show_help: false,
            ast_view_scroll: None,
            ast_lines,
            notification: None,
            nav_feedback: None,
            scroll_offset: 0,
            list_inner: Rect::default(),
            cached_node_heights: Vec::new(),
            last_click: None,
        })
    }

    /// Returns the current selection in the canonical
    /// `(node_idx, unit, unit_idx)` shape, used by the transcript harness.
    #[cfg(test)]
    pub const fn current_anchor(&self) -> (usize, &'static str, usize) {
        let a = &self.selection_state.anchor;
        (a.node_idx, a.unit.as_str(), a.unit_idx)
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
        if child.wait().is_ok_and(|status| status.success()) {
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
    fn emit_model_actions_capture_human_output_fields() {
        let mut app = test_app("Intro.\nTarget sentence.\nAfter.\n");
        app.selection_state.anchor.unit_idx = 1;
        app.handle_key(key_char('c'));
        for ch in "tighten wording".chars() {
            app.handle_key(key_char(ch));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let model = app.to_output();
        assert_eq!(model.actions.len(), 1);
        let action = &model.actions[0];
        assert_eq!(action.action, "change");
        assert_eq!(action.where_line, 2);
        assert_eq!(action.context.previous_line.as_deref(), Some("Intro."));
        assert_eq!(action.context.target, "Target sentence.");
        assert_eq!(action.context.next_line.as_deref(), Some("After."));
        let payload = action.payload.as_ref().expect("change payload");
        assert_eq!(payload.key, "CHANGE");
        assert_eq!(payload.text, "tighten wording");
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
        let (_, context) = app
            .view
            .sentence_context(app.selection_state.anchor)
            .expect("sentence context");
        assert!(context.contains("Stabilize commands/flags"), "{context}");
        assert!(context.contains("stdin"), "{context}");
        assert!(context.contains("version)"), "{context}");
        assert!(!context.contains("Next item"), "context leaked: {context}");
    }

    #[test]
    fn sentence_context_single_sentence_stops_at_node_boundary() {
        let app = test_app("First sentence ends.\nSecond sentence starts here.\n");
        let (_, context) = app
            .view
            .sentence_context(app.selection_state.anchor)
            .expect("sentence context");
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
        // segment_words treats typographic apostrophe (U+2019) as
        // internal-word continuation, the same way it treats ASCII `'`,
        // so contractions stay whole on display plain too. This keeps
        // display word indices aligned with selection-plain word
        // indices (mouse-click resolution depends on that alignment).
        let mut app = test_app("we’re in an early period\n");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);

        let spans0 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans0), "we’re");

        app.handle_key(key_char('j'));
        let spans1 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans1), "in");

        app.handle_key(key_char('j'));
        let spans2 = app.render_node_spans(0);
        assert_eq!(highlighted_text(&spans2), "an");
    }

    #[test]
    fn i_and_o_keys_cycle_unit_finer_and_coarser() {
        // i = "in" / finer; o = "out" / coarser. They stop at the
        // ends instead of wrapping around.
        let mut app = test_app("# Heading\n\nPlain prose paragraph here.\n");
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
        app.handle_key(key_char('i'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Word);
        app.handle_key(key_char('i'));
        assert_eq!(
            app.selection_state.anchor.unit,
            SelectionUnit::Word,
            "i should not wrap from Word back to Section"
        );
        app.handle_key(key_char('o'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Sentence);
        app.handle_key(key_char('o'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Line);
        app.handle_key(key_char('o'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Paragraph);
        app.handle_key(key_char('o'));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Section);
        app.handle_key(key_char('o'));
        assert_eq!(
            app.selection_state.anchor.unit,
            SelectionUnit::Section,
            "o should not wrap from Section back to Word"
        );
        // Space and Backspace still wrap.
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Paragraph);
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.selection_state.anchor.unit, SelectionUnit::Section);
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
        let rn = &app.view.rendered_nodes()[0];
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
        assert_eq!(app.view.document().node_count(), 1);
        let rn = &app.view.rendered_nodes()[0];
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

    #[test]
    fn line_mode_emphasis_paragraph_highlights_full_display() {
        // Source line carries straight apostrophes and emphasis markers.
        // pulldown-cmark applies smart-punctuation (straight → typographic)
        // and strips emphasis markers, so display plain has different
        // bytes than source. The previous `nth_occurrence(display,
        // source_line)`-then-`pos..pos+source_line.len()` mapping
        // truncated the tail of the display when `display.len() !=
        // source_line.len()`. The fix is to track per-line display
        // ranges at render time on `RenderedNode.line_ranges`.
        let body = "I'm chilling on a couch during my son's piano lesson — *heavy* (a higher specc'd mac) is doing the actual work and is working the plan.";
        let mut app = test_app(&format!("# t\n\n{body}\n"));
        let para_idx = app
            .view
            .rendered_nodes()
            .iter()
            .position(|rn| rn.plain.contains("working the plan."))
            .expect("paragraph node");
        app.selection_state.anchor = SelectionAnchor {
            node_idx: para_idx,
            unit: SelectionUnit::Line,
            unit_idx: 0,
        };
        let rn = &app.view.rendered_nodes()[para_idx];
        let plain = rn.plain.as_str();
        let range = app
            .view
            .display_range_for_unit(para_idx, SelectionUnit::Line, 0)
            .expect("line range");
        assert_eq!(
            range,
            0..plain.len(),
            "single-line paragraph should highlight whole display plain (display: {plain:?})"
        );

        // Verify the rendered spans actually paint the highlight all the
        // way to the end of the line — the byte-range fix should
        // translate into visible coverage.
        let spans = app.render_node_spans(para_idx);
        let highlighted: String = spans
            .iter()
            .filter(|s| s.style.bg == Some(Color::Blue))
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            highlighted.ends_with("working the plan."),
            "highlight must cover trailing text, got highlighted={highlighted:?}"
        );
    }

    #[test]
    fn q_requires_confirmation_then_y_quits() {
        let mut app = test_app("Body.\n");
        app.handle_key(key_char('q'));
        assert!(
            !app.should_quit,
            "first q must arm the confirmation, not quit"
        );
        assert!(app.quit_confirm_pending);
        assert!(app.status.contains("Are you sure?"));

        app.handle_key(key_char('y'));
        assert!(app.should_quit, "y after q must confirm the quit");
        assert!(
            !app.silent_quit,
            "regular q-quit emits results — silent_quit must stay false"
        );
    }

    #[test]
    fn q_then_n_cancels_quit() {
        let mut app = test_app("Body.\n");
        app.handle_key(key_char('q'));
        app.handle_key(key_char('n'));
        assert!(!app.should_quit, "n must cancel the quit");
        assert!(!app.quit_confirm_pending, "n must clear the pending flag");
        assert!(app.status.contains("cancelled") || app.status.contains("Cancel"));
    }

    #[test]
    fn q_then_q_double_tap_confirms() {
        let mut app = test_app("Body.\n");
        app.handle_key(key_char('q'));
        app.handle_key(key_char('q'));
        assert!(app.should_quit, "q-q double tap should confirm the quit");
    }

    #[test]
    fn unrelated_key_during_confirm_is_ignored() {
        let mut app = test_app("Body.\n");
        app.handle_key(key_char('q'));
        app.handle_key(key_char('j'));
        assert!(!app.should_quit, "unrelated key must not confirm quit");
        assert!(
            app.quit_confirm_pending,
            "unrelated key must leave the prompt pending"
        );
    }

    #[test]
    fn capital_q_quits_silently_without_confirmation() {
        let mut app = test_app("Body.\n");
        app.handle_key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::NONE));
        assert!(app.should_quit, "Q must quit immediately");
        assert!(app.silent_quit, "Q must set silent_quit");
        assert!(!app.quit_confirm_pending, "Q must not arm the confirmation");
    }

    // ── mouse-click selection ───────────────────────────────────────

    fn render_then_click(app: &mut App, row: u16, col: u16) {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row,
            column: col,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn single_click_selects_word_at_cursor() {
        // Multi-word paragraph, single click on "fox".
        // After draw: list block border at row 0, inner row 0 = terminal row 1.
        // Gutter is 2 cols (indicator + space), so col 2 = first text byte.
        let mut app = test_app("alpha bravo charlie delta echo.\n");
        // "alpha bravo " = 12 bytes; "charlie" starts at col 14 (gutter + 12).
        // Click on the 'h' in "charlie" at col 15.
        render_then_click(&mut app, 1, 15);
        let anchor = app.selection_state.anchor;
        assert_eq!(anchor.unit, SelectionUnit::Word, "single click → Word");
        // Word index of "charlie" should be 2 (0=alpha, 1=bravo, 2=charlie).
        let rn = &app.view.rendered_nodes()[anchor.node_idx];
        let word_text = rn
            .display_word_ranges
            .get(anchor.unit_idx)
            .and_then(|r| rn.plain.get(r.clone()))
            .unwrap_or("");
        assert_eq!(
            word_text, "charlie",
            "single click should land on the clicked word"
        );
    }

    #[test]
    fn double_click_selects_sentence() {
        // Two-sentence paragraph. Click twice in the first sentence.
        let mut app = test_app("Alpha bravo. Charlie delta echo fox.\n");
        render_then_click(&mut app, 1, 4);
        render_then_click(&mut app, 1, 4);
        let anchor = app.selection_state.anchor;
        assert_eq!(
            anchor.unit,
            SelectionUnit::Sentence,
            "double click → Sentence"
        );
        assert_eq!(anchor.unit_idx, 0, "click on first sentence");
    }

    #[test]
    fn triple_click_selects_paragraph() {
        let mut app = test_app("Alpha. Bravo. Charlie.\n");
        render_then_click(&mut app, 1, 4);
        render_then_click(&mut app, 1, 4);
        render_then_click(&mut app, 1, 4);
        let anchor = app.selection_state.anchor;
        assert_eq!(anchor.unit, SelectionUnit::Paragraph);
        assert_eq!(anchor.unit_idx, 0);
    }

    #[test]
    fn double_click_in_codeblock_selects_line() {
        // Fenced code block — sentence semantics don't apply, so double
        // click should land on the clicked line, not a sentence.
        let mut app = test_app("```\nfirst line\nsecond line\nthird line\n```\n");
        // Code block fence starts at row 1; "second line" is on inner row 2.
        render_then_click(&mut app, 3, 5);
        render_then_click(&mut app, 3, 5);
        let anchor = app.selection_state.anchor;
        assert_eq!(
            anchor.unit,
            SelectionUnit::Line,
            "double click in code → Line"
        );
        // Index lines exclude fence lines, so "first" = 0, "second" = 1.
        assert_eq!(
            anchor.unit_idx, 1,
            "should land on the second non-fence line"
        );
    }

    #[test]
    fn click_count_resets_on_position_change() {
        let mut app = test_app("Alpha. Bravo. Charlie.\n");
        render_then_click(&mut app, 1, 4);
        // Different cell — should reset to single click.
        render_then_click(&mut app, 1, 12);
        let anchor = app.selection_state.anchor;
        assert_eq!(
            anchor.unit,
            SelectionUnit::Word,
            "different cell must restart at single-click"
        );
    }

    #[test]
    fn click_outside_list_inner_is_noop() {
        let mut app = test_app("Body.\n");
        let before = app.selection_state.anchor;
        // Click on the footer row (row 23 in an 80x24 backend).
        render_then_click(&mut app, 23, 10);
        let after = app.selection_state.anchor;
        assert_eq!(
            before, after,
            "click outside list_inner must not move selection"
        );
    }

    #[test]
    fn click_during_input_mode_is_swallowed() {
        let mut app = test_app("alpha bravo charlie.\n");
        // Enter Change input mode.
        app.handle_key(key_char('c'));
        let before = app.selection_state.anchor;
        render_then_click(&mut app, 1, 8);
        let after = app.selection_state.anchor;
        assert_eq!(
            before, after,
            "click in input mode must not move the underlying selection"
        );
    }

    #[test]
    fn single_click_on_wrapped_line_picks_clicked_word() {
        // Reproduce the bug where multi-line wrapped paragraphs misalign
        // the click → word mapping. Build a paragraph long enough to
        // wrap onto multiple rows in an 80-col backend and click a
        // known word on the second wrapped row.
        let body = "alpha bravo charlie delta echo foxtrot golf hotel india juliet \
                    kilo lima mike november oscar papa quebec romeo sierra tango \
                    uniform victor whiskey xray yankee zulu.";
        let mut app = test_app(&format!("{body}\n"));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();

        let para_idx = app
            .view
            .rendered_nodes()
            .iter()
            .position(|rn| rn.plain.starts_with("alpha bravo"))
            .unwrap();
        let plain = app.view.rendered_nodes()[para_idx].plain.clone();
        // Print every visible row's byte range and rendered text so we
        // can see if there's drift on later rows.
        for (i, rm) in app.view.visible_rows().iter().enumerate() {
            if let Some(m) = rm.as_ref() {
                let txt = plain.get(m.byte_range.clone()).unwrap_or("");
                eprintln!(
                    "row {i}: node={} range={:?} text={:?}",
                    m.node_idx, m.byte_range, txt
                );
            } else {
                eprintln!("row {i}: None");
            }
        }
        // Pick a word we expect to land on a later wrapped row; adjust
        // here based on how the backend wrapped it.
        let target_word = "victor";
        let target_byte = plain.find(target_word).unwrap();
        let (row_idx, row_map) = app
            .view
            .visible_rows()
            .iter()
            .enumerate()
            .find_map(|(i, rm)| {
                rm.as_ref()
                    .filter(|m| m.byte_range.contains(&target_byte))
                    .map(|m| (i, m.clone()))
            })
            .unwrap_or_else(|| panic!("{target_word} byte {target_byte} not found on any row"));
        let row_text = &plain[row_map.byte_range.clone()];
        let target_offset_in_row = target_byte - row_map.byte_range.start;
        let target_col_in_row: usize = row_text[..target_offset_in_row]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        let inner = app.list_inner;
        let click_col = inner.x + row_map.gutter_cols + (target_col_in_row as u16) + 1;
        let click_row = inner.y + (row_idx as u16);
        eprintln!(
            "clicking '{target_word}' at row={click_row} col={click_col} (row_idx={row_idx})"
        );

        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: click_row,
            column: click_col,
            modifiers: KeyModifiers::NONE,
        });
        let anchor = app.selection_state.anchor;
        assert_eq!(anchor.unit, SelectionUnit::Word);
        let rn = &app.view.rendered_nodes()[anchor.node_idx];
        let selected = &rn.plain[rn.display_word_ranges[anchor.unit_idx].clone()];
        assert_eq!(
            selected, target_word,
            "click on {target_word:?} must select it"
        );
    }

    #[test]
    fn click_on_word_with_smart_apostrophe_picks_whole_contraction() {
        let body = "I'm chilling on a couch during my son's piano lesson.";
        let mut app = test_app(&format!("{body}\n"));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let para_idx = 0;
        let plain = app.view.rendered_nodes()[para_idx].plain.clone();
        let display_words: Vec<&str> = app.view.rendered_nodes()[para_idx]
            .display_word_ranges
            .iter()
            .map(|r| &plain[r.clone()])
            .collect();
        assert!(
            display_words.contains(&"I\u{2019}m"),
            "display words should include I'm (typographic), got {display_words:?}"
        );
        assert!(
            display_words.contains(&"son\u{2019}s"),
            "display words should include son's (typographic), got {display_words:?}"
        );
        // Click on the apostrophe of "son's" — used to land on the "s"
        // suffix or further off; should now resolve to the whole word.
        let target_word = "son\u{2019}s";
        let target_byte = plain.find(target_word).unwrap();
        let apos_byte = plain[target_byte..].find('\u{2019}').unwrap() + target_byte;
        let row_idx = app
            .view
            .visible_rows()
            .iter()
            .position(|rm| {
                rm.as_ref()
                    .is_some_and(|m| m.byte_range.contains(&apos_byte))
            })
            .unwrap();
        let map = app.view.visible_rows()[row_idx].clone().unwrap();
        let row_text = &plain[map.byte_range.clone()];
        let in_row = apos_byte - map.byte_range.start;
        let col_in_row: usize = row_text[..in_row]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        let inner = app.list_inner;
        let click_col = inner.x + map.gutter_cols + (col_in_row as u16);
        let click_row = inner.y + (row_idx as u16);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: click_row,
            column: click_col,
            modifiers: KeyModifiers::NONE,
        });
        let anchor = app.selection_state.anchor;
        assert_eq!(anchor.unit, SelectionUnit::Word);
        let selected = &app.view.rendered_nodes()[anchor.node_idx].plain[app.view.rendered_nodes()
            [anchor.node_idx]
            .display_word_ranges[anchor.unit_idx]
            .clone()];
        assert_eq!(
            selected, target_word,
            "click on the smart apostrophe of son's must select the whole contraction"
        );
    }

    #[test]
    fn click_on_trailing_space_does_not_jump_to_last_word() {
        // Regression: a click on the space between two words used to
        // fall through `position(...).or_else(|| len-1)` and select the
        // LAST word in the paragraph — off by however many words came
        // after the click. The fix walks ranges with
        // partition_point and returns the closest preceding word.
        let mut app =
            test_app("alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima.\n");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let para_idx = 0;
        let plain = app.view.rendered_nodes()[para_idx].plain.clone();
        // Byte just past "charlie" — the space between charlie and delta.
        let charlie_end = plain.find("charlie").unwrap() + "charlie".len();
        let row_idx = app
            .view
            .visible_rows()
            .iter()
            .position(|rm| {
                rm.as_ref()
                    .is_some_and(|m| m.byte_range.contains(&charlie_end))
            })
            .unwrap();
        let map = app.view.visible_rows()[row_idx].clone().unwrap();
        let row_text = &plain[map.byte_range.clone()];
        let in_row = charlie_end - map.byte_range.start;
        let col_in_row: usize = row_text[..in_row]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum();
        let inner = app.list_inner;
        let click_col = inner.x + map.gutter_cols + (col_in_row as u16);
        let click_row = inner.y + (row_idx as u16);
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: click_row,
            column: click_col,
            modifiers: KeyModifiers::NONE,
        });
        let anchor = app.selection_state.anchor;
        let selected = &app.view.rendered_nodes()[anchor.node_idx].plain[app.view.rendered_nodes()
            [anchor.node_idx]
            .display_word_ranges[anchor.unit_idx]
            .clone()];
        assert_eq!(
            selected, "charlie",
            "click in the space after 'charlie' should still pick 'charlie' \
             (was selecting last word due to len-1 fallback)"
        );
    }

    #[test]
    fn click_words_in_real_markdown_file() {
        // Walk every visible word in a real wrapped document and assert
        // that a click on each word's column resolves to that word.
        let path = std::path::PathBuf::from(
            "/Users/admin/dev/projects/mattorb.com/src/content/posts/macbook-neo-ai-remote-control.md",
        );
        if !path.exists() {
            eprintln!("skipping: {} not present", path.display());
            return;
        }
        let mut app = App::load(path).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();

        let snapshot = app.view.visible_rows().to_vec();
        let mut mismatches: Vec<String> = Vec::new();
        for (row_idx, rm) in snapshot.iter().enumerate() {
            let Some(map) = rm else { continue };
            let node_idx = map.node_idx;
            let Some(rn) = app.view.rendered_nodes().get(node_idx) else {
                continue;
            };
            if !matches!(
                app.view.document().nodes.get(node_idx),
                Some(DocNode::Paragraph { .. })
            ) {
                continue;
            }
            if map.byte_range.is_empty() {
                continue;
            }
            let plain = rn.plain.clone();
            let row_text = plain[map.byte_range.clone()].to_string();
            let display_word_ranges = rn.display_word_ranges.clone();
            for word_range in &display_word_ranges {
                if word_range.start < map.byte_range.start || word_range.end > map.byte_range.end {
                    continue;
                }
                // Click on the LAST byte of the word and on the byte
                // just past it (the trailing space). Both should
                // resolve to this same word — the trailing-space case
                // is the click pattern that produced "off by N words"
                // before the find_unit_at fix.
                let word_end_offset = word_range.end - map.byte_range.start - 1;
                let after_word_offset = (word_range.end - map.byte_range.start).min(row_text.len());
                for in_row in [word_end_offset, after_word_offset] {
                    if in_row > row_text.len() {
                        continue;
                    }
                    let col_in_row: usize = row_text
                        .get(..in_row)
                        .unwrap_or("")
                        .chars()
                        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                        .sum();
                    let inner = app.list_inner;
                    let click_col = inner.x + map.gutter_cols + (col_in_row as u16);
                    let click_row = inner.y + (row_idx as u16);
                    app.last_click = None;
                    app.handle_mouse(MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        row: click_row,
                        column: click_col,
                        modifiers: KeyModifiers::NONE,
                    });
                    let anchor = app.selection_state.anchor;
                    let actual = app.view.rendered_nodes()[anchor.node_idx]
                        .plain
                        .get(
                            app.view.rendered_nodes()[anchor.node_idx].display_word_ranges
                                [anchor.unit_idx]
                                .clone(),
                        )
                        .unwrap_or("")
                        .to_string();
                    let expected = &plain[word_range.clone()];
                    if actual != expected {
                        mismatches.push(format!(
                            "row {row_idx} col {click_col} (in_row={in_row}) expected={expected:?} \
                             got={actual:?} row_byte={:?} row_text={row_text:?}",
                            map.byte_range
                        ));
                        if mismatches.len() >= 12 {
                            break;
                        }
                    }
                }
                if mismatches.len() >= 12 {
                    break;
                }
            }
            if mismatches.len() >= 12 {
                break;
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} mismatches:\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }

    #[test]
    fn click_count_resets_after_timeout() {
        // Manually drive bump_click_count to verify the time-based reset
        // without sleeping for half a second in the test.
        let mut app = test_app("Body.\n");
        app.last_click = Some(LastClick {
            at: Instant::now() - Duration::from_secs(1),
            row: 5,
            col: 10,
            count: 1,
        });
        let n = app.bump_click_count(5, 10);
        assert_eq!(n, 1, "stale prior click must restart the count at 1");
    }
}

#[cfg(test)]
#[path = "emit_matrix_tests.rs"]
mod emit_matrix_tests;

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod transcript_tests;

#[cfg(test)]
#[path = "tui_snapshot_tests.rs"]
mod tui_snapshot_tests;
