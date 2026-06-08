use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
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

#[cfg(test)]
use crate::document::DocNode;
use crate::document_view::{
    CodeBlockStyleRequest, DisplaySpanStyleRequest, DocumentView, SourceActionContext,
};
use crate::output::{
    EmitAction, EmitActionContext, EmitChange, EmitFeedback, EmitInsert, EmitKeymap,
    EmitLineAnnotation, EmitLineContext, EmitModel, EmitPayload, EmitReaction, clean_context,
    keybinding_doc_rows, render_human_output,
};
use crate::selection::model::{SelectionAnchor, SelectionState, SelectionUnit};
use crate::ui::wrap_styled_spans;

mod input;
mod output;
mod render;
mod state;

use self::state::{
    CLICK_DOUBLE_INTERVAL, ChangeAnnotation, EditableAnnotation, FeedbackAnnotation, InputMode,
    InsertAnnotation, LastClick,
};

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
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "emit_matrix_tests.rs"]
mod emit_matrix_tests;

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod transcript_tests;

#[cfg(test)]
#[path = "tui_snapshot_tests.rs"]
mod tui_snapshot_tests;
