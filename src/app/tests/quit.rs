use super::*;

#[test]
fn q_requires_confirmation_then_y_quits() {
    let mut app = test_app("Body.\n");
    app.handle_key(key_char('q'));
    assert!(
        !app.should_quit,
        "first q must arm the confirmation, not quit"
    );
    assert!(app.quit_confirm_pending);

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
