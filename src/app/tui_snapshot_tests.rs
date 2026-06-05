use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::App;
use crate::test_support::{render_app_to_string, snapshot_app};
use std::path::PathBuf;

fn test_app(name: &str, content: &str) -> App {
    snapshot_app(name, content)
}

fn render(app: &mut App) -> String {
    render_app_to_string(app, 72, 22)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn shift_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

fn type_chars(app: &mut App, text: &str) {
    for ch in text.chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
}

fn snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);
    settings.set_snapshot_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("tui_snapshots"),
    );
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

#[test]
fn feedback_input_state_snapshot() {
    let mut app = test_app(
        "feedback_input",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('f')));
    type_chars(&mut app, "Clarify the release risk");
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("feedback_input", render(&mut app));
    });
}

#[test]
fn insert_before_input_state_snapshot() {
    let mut app = test_app(
        "insert_before_input",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('b')));
    type_chars(&mut app, "Add a rollback note");
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("insert_before_input", render(&mut app));
    });
}

#[test]
fn insert_after_input_state_snapshot() {
    let mut app = test_app(
        "insert_after_input",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('a')));
    type_chars(&mut app, "Follow with install verification");
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("insert_after_input", render(&mut app));
    });
}

#[test]
fn search_input_state_snapshot() {
    let mut app = test_app(
        "search_input",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('/')));
    type_chars(&mut app, "binary");
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("search_input", render(&mut app));
    });
}

#[test]
fn search_no_match_state_snapshot() {
    let mut app = test_app(
        "search_no_match",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('/')));
    type_chars(&mut app, "notfound");
    app.handle_key(key(KeyCode::Enter));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("search_no_match", render(&mut app));
    });
}

#[test]
fn quit_confirmation_state_snapshot() {
    let mut app = test_app(
        "quit_confirmation",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('q')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("quit_confirmation", render(&mut app));
    });
}

#[test]
fn ast_popup_state_snapshot() {
    let mut app = test_app(
        "ast_popup",
        "# Release Plan\n\nShip the binary installer.\n",
    );
    app.handle_key(key(KeyCode::Char('I')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("ast_popup", render(&mut app));
    });
}

#[test]
fn link_popup_state_snapshot() {
    let mut app = test_app(
        "link_popup",
        "# Release Plan\n\nSee [the installer](https://example.com/install) before shipping.\n",
    );
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char('O')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("link_popup", render(&mut app));
    });
}

#[test]
fn annotated_gutter_change_state_snapshot() {
    let mut app = test_app("gutter_change", "Ship the binary installer.\n");
    app.handle_key(key(KeyCode::Char('c')));
    type_chars(&mut app, "Include rollback");
    app.handle_key(key(KeyCode::Enter));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("gutter_change", render(&mut app));
    });
}

#[test]
fn annotated_gutter_feedback_state_snapshot() {
    let mut app = test_app("gutter_feedback", "Ship the binary installer.\n");
    app.handle_key(key(KeyCode::Char('f')));
    type_chars(&mut app, "Clarify risk");
    app.handle_key(key(KeyCode::Enter));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("gutter_feedback", render(&mut app));
    });
}

#[test]
fn annotated_gutter_insert_state_snapshot() {
    let mut app = test_app("gutter_insert", "Ship the binary installer.\n");
    app.handle_key(key(KeyCode::Char('b')));
    type_chars(&mut app, "Add prerequisite");
    app.handle_key(key(KeyCode::Enter));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("gutter_insert", render(&mut app));
    });
}

#[test]
fn annotated_gutter_strike_state_snapshot() {
    let mut app = test_app("gutter_strike", "Ship the binary installer.\n");
    app.handle_key(key(KeyCode::Char('x')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("gutter_strike", render(&mut app));
    });
}

#[test]
fn help_via_shift_slash_state_snapshot() {
    let mut app = test_app("help_shift_slash", "# Release Plan\n\nShip it.\n");
    app.handle_key(shift_key(KeyCode::Char('/')));
    snapshot_settings().bind(|| {
        insta::assert_snapshot!("help_shift_slash", render(&mut app));
    });
}
