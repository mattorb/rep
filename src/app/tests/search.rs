use super::*;

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
