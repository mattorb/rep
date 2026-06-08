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
