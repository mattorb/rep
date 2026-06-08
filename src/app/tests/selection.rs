use super::*;

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
        .target_capture(app.selection_state.anchor)
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
        .target_capture(app.selection_state.anchor)
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
