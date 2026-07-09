use super::*;
use ratatui::{Terminal, backend::TestBackend};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[path = "tests/annotations.rs"]
mod annotations;
#[path = "tests/mouse.rs"]
mod mouse;
#[path = "tests/quit.rs"]
mod quit;
#[path = "tests/search.rs"]
mod search;
#[path = "tests/selection.rs"]
mod selection;
#[path = "tests/viewport.rs"]
mod viewport;

static FILE_SEQ: AtomicUsize = AtomicUsize::new(0);

fn test_app(content: &str) -> App {
    let n = FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rep_test_{n}.md"));
    std::fs::write(&path, content).expect("test precondition: write temp markdown fixture");
    App::load(path).expect("test precondition: load temp markdown fixture")
}

fn render(app: &mut App) -> Terminal<TestBackend> {
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");
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

#[test]
fn load_initializes_app_state_from_first_content_node() {
    let app = test_app("---\n\n# Heading\n\nBody text.\n");

    assert_eq!(app.current_anchor(), (1, "Sentence", 0));
    assert_eq!(
        app.status,
        "Loaded file. Press q to quit and print annotations."
    );
    assert!(!app.should_quit);
    assert!(!app.silent_quit);
    assert!(!app.quit_confirm_pending);
    assert!(app.link_popup_urls.is_none());
    assert!(!app.show_help);
    assert!(app.ast_view_scroll.is_none());
    assert_eq!(app.scroll_offset, 0);
    assert!(app.render_cache.node_heights.is_empty());
    assert!(app.ast_lines.iter().any(|line| line.contains("Root")));
}

#[test]
fn load_reports_missing_markdown_path() {
    let path = std::env::temp_dir().join(format!(
        "rep_missing_{}.md",
        FILE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));

    let err = App::load(path.clone()).unwrap_err();

    assert!(err.to_string().contains("failed to read markdown file"));
    assert!(err.to_string().contains(path.to_string_lossy().as_ref()));
}

#[test]
fn base64_encode_matches_standard_padding_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
}

#[test]
fn key_cue_formats_keys_like_social_demo_overlay() {
    let shift_space = format_key_cue(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
    let ctrl_c = format_key_cue(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    let tab = format_key_cue(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let enter = format_key_cue(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(shift_space, "Shift Space");
    assert_eq!(ctrl_c, "^ C");
    assert_eq!(tab, "Tab");
    assert_eq!(enter, "Enter");
}
