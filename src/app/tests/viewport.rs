use super::*;

#[test]
fn tall_item_at_bottom_renders_partial_not_blank() {
    // 10 list items (no inter-item spacers) + 1 tall paragraph.
    // Layout: height=15, footer=1, outer block=13, border top+bottom → inner=11.
    // But we need inner=12, so use height=15 → outer=14 → inner=12.
    //
    // With inner_height=12:
    //   Nodes 0-8 (list items, no spacer): 9 rows.
    //   Node 9 (last list item + trailing spacer before paragraph): 2 rows.
    //   Total: 11 rows → 1 row left at inner row 11 = terminal row 12.
    // That row must show "tall line 0", not be blank.
    let mut content = String::new();
    for i in 0..10 {
        let _ = writeln!(content, "- Item {i}");
    }
    content.push('\n'); // blank line separates list from following paragraph
    for j in 0..12 {
        let _ = writeln!(content, "tall line {j}");
    }

    let mut app = test_app(&content);
    // height=15: footer at row 14, outer block rows 0-13, inner rows 1-12 (height=12).
    let mut terminal =
        Terminal::new(TestBackend::new(40, 15)).expect("test precondition: create terminal");
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

    // Inner row 11 = terminal row 12.
    let buf = terminal.backend().buffer();
    let row12: String = (0..40)
        .map(|x| {
            buf.cell(ratatui::layout::Position::new(x, 12))
                .map_or(" ", |c| c.symbol())
        })
        .collect();
    assert!(
        row12.contains("tall line"),
        "partial tall item should render at bottom, got: {row12:?}"
    );
}

#[test]
fn navigating_to_tall_node_bottom_aligns_it() {
    // When the cursor moves to a node taller than the available space,
    // adjust_scroll should bottom-align the node so the last visible lines
    // are at the bottom of the screen — not just show the first line.
    //
    // Layout: 5-line terminal, footer=1, outer=3 (border top+bottom → inner=1).
    // Use a 7-line terminal to get inner=4.
    // Node 0: "before" (1 line, + spacer = 2 rows since next is block start)
    // Node 1: tall paragraph with 6 lines (> inner_height of 4).
    //
    // After navigating to node 1, adjust_scroll must bottom-align it:
    // target_start = max(0, 4-6) = 0 → cursor at top of inner area.
    // The first 4 lines of the tall node fill the screen.
    // Terminal row 1 (inner row 0) should show "tall line 0".
    let content =
        "before\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\ntall line 5\n";
    let mut app = test_app(content);
    // height=7: footer row 6, outer block rows 0-5, inner rows 1-4 (height=4).
    let mut terminal =
        Terminal::new(TestBackend::new(40, 7)).expect("test precondition: create terminal");

    // Navigate to node 1 (the tall paragraph).
    app.move_node(1);
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

    let buf = terminal.backend().buffer();
    let inner_rows: Vec<String> = (1..=4)
        .map(|y| {
            (0..40)
                .map(|x| {
                    buf.cell(ratatui::layout::Position::new(x, y))
                        .map_or(" ", |c| c.symbol())
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();

    assert!(
        inner_rows[0].contains("tall line 0"),
        "tall node should be top-aligned (bottom-aligned with 0 context): got {inner_rows:?}"
    );
    assert!(
        inner_rows[3].contains("tall line 3"),
        "last visible inner row should show tall line 3: got {inner_rows:?}"
    );
}

#[test]
fn move_sentence_to_tall_node_bottom_aligns_it() {
    // Same layout as navigating_to_tall_node_bottom_aligns_it, but navigate
    // there via move_sentence (j key) rather than move_node (Down arrow).
    // adjust_scroll fires on every draw() regardless of how cursor moved.
    let content =
        "before\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\ntall line 5\n";
    let mut app = test_app(content);
    let mut terminal =
        Terminal::new(TestBackend::new(40, 7)).expect("test precondition: create terminal");

    // move_sentence forward from node 0's last sentence → lands on node 1.
    app.handle_key(key_char('j'));
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

    let buf = terminal.backend().buffer();
    let inner_rows: Vec<String> = (1..=4)
        .map(|y| {
            (0..40)
                .map(|x| {
                    buf.cell(ratatui::layout::Position::new(x, y))
                        .map_or(" ", |c| c.symbol())
                })
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();

    assert_eq!(
        app.selection_state.anchor.node_idx, 1,
        "cursor should be on the tall node"
    );
    assert!(
        inner_rows[0].contains("tall line 0"),
        "move_sentence to tall node must also bottom-align it: got {inner_rows:?}"
    );
    assert!(
        inner_rows[3].contains("tall line 3"),
        "last inner row should show tall line 3: got {inner_rows:?}"
    );
}

#[test]
fn fill_partial_bottom_reveals_more_of_next_node() {
    // Cursor is on node 1 (short paragraph); node 2 is a 5-line paragraph.
    // Without fill_partial_bottom, scroll_offset=0 and only 3 lines of node 2
    // fit on screen. fill_partial_bottom should skip node 0 (scroll_offset→1)
    // so all 5 lines of node 2 become visible.
    //
    // Layout: height=10 → footer at row 9, borders rows 0 & 8, inner rows 1-7 (height=7).
    //   Node 0: "short A"      → 1 content row + 1 spacer = 2 rows
    //   Node 1: "short B..."   → 1 content row + 1 spacer = 2 rows  (cursor)
    //   Node 2: 5-line para    → 5 rows (last node, no spacer)
    //   Total: 9 rows > inner_height=7; node 2 partially hidden without scrolling.
    //
    // After fill_partial_bottom: scroll_offset=1 (node 0 scrolled off).
    //   Node 1: inner rows 0-1, Node 2: inner rows 2-6 (terminal rows 3-7).
    //   Terminal row 7 should show "tall line 4" (the last of 5 node-2 lines).
    let content = "short A\n\nshort B. Next. Third.\n\ntall line 0\ntall line 1\ntall line 2\ntall line 3\ntall line 4\n";
    let mut app = test_app(content);
    let mut terminal =
        Terminal::new(TestBackend::new(40, 10)).expect("test precondition: create terminal");

    app.move_node(1); // cursor on "short B" node
    terminal
        .draw(|f| app.draw(f))
        .expect("test precondition: draw app");

    let buf = terminal.backend().buffer();
    let last_inner_row: String = (0..40)
        .map(|x| {
            buf.cell(ratatui::layout::Position::new(x, 7))
                .map_or(" ", |c| c.symbol())
        })
        .collect::<String>()
        .trim_end()
        .to_string();

    assert!(
        last_inner_row.contains("tall line 4"),
        "fill_partial_bottom should reveal all 5 lines of next node; last inner row got: {last_inner_row:?}"
    );
}

#[test]
fn line_mode_emphasis_paragraph_highlights_full_display() {
    // Source line carries straight apostrophes and emphasis markers.
    // pulldown-cmark applies smart-punctuation (straight → typographic)
    // and strips emphasis markers, so display plain has different
    // bytes than source. The previous `nth_occurrence(display,
    // source_line)`-then-`pos..pos+source_line.len()` mapping
    // truncated the tail of the display when `display.len() !=
    // source_line.len()`. The fix is to track per-line display
    // ranges at render time on `RenderedNode.line_ranges`.
    let body = "I'm chilling on a couch during my son's piano lesson — *heavy* (a higher specc'd mac) is doing the actual work and is working the plan.";
    let mut app = test_app(&format!("# t\n\n{body}\n"));
    let para_idx = app
        .view
        .rendered_nodes()
        .iter()
        .position(|rn| rn.plain.contains("working the plan."))
        .expect("paragraph node");
    app.selection_state.anchor = SelectionAnchor {
        node_idx: para_idx,
        unit: SelectionUnit::Line,
        unit_idx: 0,
    };
    // Verify the rendered spans paint the highlight all the way to
    // the end of the line.
    let spans = app.render_node_spans(para_idx);
    let highlighted: String = spans
        .iter()
        .filter(|s| s.style.bg == Some(Color::Blue))
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        highlighted.ends_with("working the plan."),
        "highlight must cover trailing text, got highlighted={highlighted:?}"
    );
}
