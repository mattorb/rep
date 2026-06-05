use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use rep::app::App;
use std::path::PathBuf;

fn test_app(name: &str, content: &str) -> App {
    let path = std::env::temp_dir().join(format!("rep_snapshot_{name}.md"));
    std::fs::write(&path, content).unwrap();
    App::load(path).unwrap()
}

fn render(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(72, 22)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(
                buffer
                    .cell(ratatui::layout::Position::new(x, y))
                    .map_or(" ", |cell| cell.symbol()),
            );
        }
        out.push('\n');
    }
    out
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_snapshot_path(PathBuf::from("fixtures/tui_snapshots"));
    settings
}

#[test]
fn normal_state_snapshot() {
    let mut app = test_app(
        "normal",
        "# Release Plan\n\nShip the binary installer.\n\n- Add checks.\n",
    );
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("normal", render(&mut app));
    });
}

#[test]
fn help_state_snapshot() {
    let mut app = test_app("help", "# Release Plan\n\nShip the binary installer.\n");
    app.handle_key(key(KeyCode::Char('?')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("help", render(&mut app));
    });
}

#[test]
fn change_input_state_snapshot() {
    let mut app = test_app(
        "change_input",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('c')));
    for ch in "Include the agent skill".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("change_input", render(&mut app));
    });
}
