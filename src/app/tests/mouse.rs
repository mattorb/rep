use super::*;

fn render_then_click(app: &mut App, row: u16, col: u16) {
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");
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
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

    let para_idx = app
        .view
        .rendered_nodes()
        .iter()
        .position(|rn| rn.plain.starts_with("alpha bravo"))
        .expect("test invariant: wrapped paragraph rendered");
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
    let target_byte = plain
        .find(target_word)
        .expect("test invariant: target word exists in wrapped paragraph");
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
    eprintln!("clicking '{target_word}' at row={click_row} col={click_col} (row_idx={row_idx})");

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
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");
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
    let target_byte = plain
        .find(target_word)
        .expect("test invariant: smart-apostrophe target word exists");
    let apos_byte = plain[target_byte..]
        .find('\u{2019}')
        .expect("test invariant: target word contains smart apostrophe")
        + target_byte;
    let row_idx = app
        .view
        .visible_rows()
        .iter()
        .position(|rm| {
            rm.as_ref()
                .is_some_and(|m| m.byte_range.contains(&apos_byte))
        })
        .expect("test invariant: apostrophe byte is visible");
    let map = app.view.visible_rows()[row_idx]
        .clone()
        .expect("test invariant: visible row has mapping");
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
    let selected = &app.view.rendered_nodes()[anchor.node_idx].plain
        [app.view.rendered_nodes()[anchor.node_idx].display_word_ranges[anchor.unit_idx].clone()];
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
    let mut terminal =
        Terminal::new(TestBackend::new(80, 24)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");
    let para_idx = 0;
    let plain = app.view.rendered_nodes()[para_idx].plain.clone();
    // Byte just past "charlie" — the space between charlie and delta.
    let charlie_end = plain
        .find("charlie")
        .expect("test invariant: fixture contains charlie")
        + "charlie".len();
    let row_idx = app
        .view
        .visible_rows()
        .iter()
        .position(|rm| {
            rm.as_ref()
                .is_some_and(|m| m.byte_range.contains(&charlie_end))
        })
        .expect("test invariant: trailing space after charlie is visible");
    let map = app.view.visible_rows()[row_idx]
        .clone()
        .expect("test invariant: visible row has mapping");
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
    let selected = &app.view.rendered_nodes()[anchor.node_idx].plain
        [app.view.rendered_nodes()[anchor.node_idx].display_word_ranges[anchor.unit_idx].clone()];
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
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/app/click-words-real-markdown.md");
    let mut app = App::load(path).expect("test precondition: load markdown fixture");
    let mut terminal =
        Terminal::new(TestBackend::new(80, 40)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

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
