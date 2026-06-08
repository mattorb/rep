use super::*;

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
    std::fs::write(&path, "line one\n").expect("test precondition: write named temp fixture");
    let stem = path
        .file_name()
        .expect("test precondition: temp path has a file name")
        .to_string_lossy()
        .to_string();
    let mut app = App::load(path).expect("test precondition: load named temp fixture");
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
