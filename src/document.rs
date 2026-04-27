use std::ops::Range;

use markdown::mdast;

// ── Data model ───────────────────────────────────────────────────────────────

/// A parsed document: a flat, indexed sequence of content nodes.
///
/// Paragraphs have their soft-wrapped lines joined into a single string before
/// sentence splitting, so sentence boundaries are correct even when the source
/// file hard-wraps at 80 columns.
#[derive(Debug, Clone)]
pub struct Document {
    pub nodes: Vec<DocNode>,
}

#[derive(Debug, Clone)]
pub enum DocNode {
    Heading {
        #[cfg_attr(not(test), allow(dead_code))]
        level: u8,
        text: String,
        source_line: usize,
    },
    Paragraph {
        text: String,
        source_lines: Range<usize>,
    },
    ListItem {
        depth: usize,
        ordered: bool,
        #[cfg_attr(not(test), allow(dead_code))]
        prefix: String,
        /// Identifies which root-level list this item belongs to; all items in
        /// the same outermost list (including nested) share one id.
        list_id: usize,
        text: String,
        source_lines: Range<usize>,
    },
    CodeBlock {
        #[cfg_attr(not(test), allow(dead_code))]
        language: Option<String>,
        content: String,
        source_lines: Range<usize>,
    },
    ThematicBreak {
        source_line: usize,
    },
}

impl DocNode {
    /// Whether the node has selectable content. Used to decide where the
    /// cursor lands at load time and where same-unit traversal jumps to.
    pub fn has_content(&self) -> bool {
        match self {
            Self::Paragraph { text, .. }
            | Self::ListItem { text, .. }
            | Self::Heading { text, .. } => !text.is_empty(),
            Self::CodeBlock { content, .. } => !content.is_empty(),
            Self::ThematicBreak { .. } => false,
        }
    }

    #[cfg(test)]
    fn is_heading(&self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    pub fn source_start_line(&self) -> usize {
        match self {
            Self::Heading { source_line, .. } | Self::ThematicBreak { source_line } => *source_line,
            Self::Paragraph { source_lines, .. }
            | Self::ListItem { source_lines, .. }
            | Self::CodeBlock { source_lines, .. } => source_lines.start,
        }
    }
}

// ── Parser ────────────────────────────────────────────────────────────────────

impl Document {
    pub fn parse(content: &str) -> Self {
        let options = markdown::ParseOptions::gfm();
        let Ok(ast) = markdown::to_mdast(content, &options) else {
            return Self { nodes: Vec::new() };
        };

        let mut nodes = Vec::new();
        let mut list_counter = 0usize;
        if let mdast::Node::Root(root) = ast {
            for child in &root.children {
                collect_nodes(child, 0, None, &mut list_counter, &mut nodes);
            }
        }

        Self { nodes }
    }
}

fn collect_nodes(
    node: &mdast::Node,
    depth: usize,
    outer_list_id: Option<usize>,
    list_counter: &mut usize,
    out: &mut Vec<DocNode>,
) {
    match node {
        mdast::Node::Heading(h) => {
            let text = extract_plain_text(node);
            let source_line = h.position.as_ref().map_or(0, |p| p.start.line - 1);
            out.push(DocNode::Heading {
                level: h.depth,
                text,
                source_line,
            });
        }
        mdast::Node::Paragraph(p) => {
            let raw = extract_plain_text(node);
            let text = normalize_whitespace(&raw);
            let source_lines = p
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::Paragraph { text, source_lines });
        }
        mdast::Node::List(list) => {
            // All items that share the same outermost list get the same list_id.
            let this_list_id = outer_list_id.unwrap_or_else(|| {
                *list_counter += 1;
                *list_counter
            });
            for (i, item) in list.children.iter().enumerate() {
                if let mdast::Node::ListItem(li) = item {
                    let ordered = list.ordered;
                    let prefix = if ordered {
                        let n = list.start.unwrap_or(1) as usize + i;
                        format!("{n}. ")
                    } else {
                        "• ".to_owned()
                    };
                    let raw = extract_list_item_text(li);
                    let text = normalize_whitespace(&raw);
                    let source_lines = li
                        .position
                        .as_ref()
                        .map(|pos| (pos.start.line - 1)..pos.end.line)
                        .unwrap_or(0..1);
                    out.push(DocNode::ListItem {
                        depth,
                        ordered,
                        prefix,
                        list_id: this_list_id,
                        text,
                        source_lines,
                    });
                    // Recurse into nested lists, passing the same outer list_id.
                    for child in &li.children {
                        if matches!(child, mdast::Node::List(_)) {
                            collect_nodes(child, depth + 1, Some(this_list_id), list_counter, out);
                        }
                    }
                }
            }
        }
        mdast::Node::Code(code) => {
            let source_lines = code
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::CodeBlock {
                language: code.lang.clone(),
                content: code.value.clone(),
                source_lines,
            });
        }
        mdast::Node::ThematicBreak(tb) => {
            let source_line = tb.position.as_ref().map_or(0, |p| p.start.line - 1);
            out.push(DocNode::ThematicBreak { source_line });
        }
        mdast::Node::Blockquote(bq) => {
            for child in &bq.children {
                collect_nodes(child, depth, None, list_counter, out);
            }
        }
        mdast::Node::Table(t) => {
            // GFM table: whole table = one Paragraph node. Per-row text is cells
            // joined by a single space (each cell trimmed); rows joined by `\n`.
            // The header-separator line is encoded as `Table.align`, not as a
            // child node, so excluding it is automatic.
            let mut rows: Vec<String> = Vec::new();
            for child in &t.children {
                if let mdast::Node::TableRow(row) = child {
                    let cells: Vec<String> = row
                        .children
                        .iter()
                        .filter_map(|c| {
                            if let mdast::Node::TableCell(_) = c {
                                Some(
                                    extract_plain_text(c)
                                        .split_whitespace()
                                        .collect::<Vec<_>>()
                                        .join(" "),
                                )
                            } else {
                                None
                            }
                        })
                        .collect();
                    rows.push(cells.join(" "));
                }
            }
            let text = rows.join("\n");
            let source_lines = t
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::Paragraph { text, source_lines });
        }
        mdast::Node::FootnoteDefinition(fd) => {
            // Footnote definition body becomes a Paragraph. The body text is the
            // children's plain text, normalized.
            let raw = extract_plain_text(node);
            let text = normalize_whitespace(&raw);
            let source_lines = fd
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::Paragraph { text, source_lines });
        }
        mdast::Node::Html(h) => {
            // Block-level HTML folds into the existing CodeBlock variant per
            // modular_plan.md (HTML-as-CodeBlock-variant rule, no new enum arm).
            let source_lines = h
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::CodeBlock {
                language: None,
                content: h.value.clone(),
                source_lines,
            });
        }
        _ => {}
    }
}

/// Extract text from a ListItem's non-list children (skip nested List nodes).
fn extract_list_item_text(li: &mdast::ListItem) -> String {
    li.children
        .iter()
        .filter(|child| !matches!(child, mdast::Node::List(_)))
        .map(extract_plain_text)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Recursively extract plain text from any node, collapsing inline markup.
fn extract_plain_text(node: &mdast::Node) -> String {
    match node {
        mdast::Node::Text(t) => t.value.clone(),
        mdast::Node::InlineCode(c) => c.value.clone(),
        mdast::Node::Break(_) => " ".to_owned(),
        _ => node
            .children()
            .map(|ch| ch.iter().map(extract_plain_text).collect::<String>())
            .unwrap_or_default(),
    }
}

/// Collapse internal newlines and runs of whitespace to single spaces.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ── Navigation helpers ────────────────────────────────────────────────────────

impl Document {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// First node at or after `from` that has any selectable content.
    /// Used by `App::load` to pick the initial cursor and by the mouse
    /// scroll handler in `move_node`.
    pub fn next_content_node(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.nodes[i].has_content())
    }

    /// Last node strictly before `before` that has any selectable content.
    /// Used by the mouse scroll handler in `move_node`.
    pub fn prev_content_node(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.nodes[i].has_content())
    }

    /// True when `idx` is the first node of a new content block.
    /// Headings, paragraphs, and code blocks are always their own block.
    /// A ListItem is a block start only if the preceding node doesn't belong
    /// to the same root-level list.
    pub fn is_block_start(&self, idx: usize) -> bool {
        if idx == 0 {
            return true;
        }
        match &self.nodes[idx] {
            DocNode::ListItem { list_id, .. } => !matches!(
                &self.nodes[idx - 1],
                DocNode::ListItem { list_id: prev_id, .. } if prev_id == list_id
            ),
            _ => true,
        }
    }

    /// Last node index that belongs to the same conceptual block as `start`.
    /// For list items all sharing the same list_id, returns the last such item.
    /// For paragraphs/headings/code blocks, returns `start` itself.
    #[cfg(test)]
    fn block_end(&self, start: usize) -> usize {
        match &self.nodes[start] {
            DocNode::ListItem { list_id, .. } => {
                let target = *list_id;
                let mut end = start;
                for i in (start + 1)..self.nodes.len() {
                    if let DocNode::ListItem { list_id: id, .. } = &self.nodes[i]
                        && *id == target
                    {
                        end = i;
                        continue;
                    }
                    break;
                }
                end
            }
            _ => start,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heading_and_paragraph() {
        let doc = Document::parse("# Hello World\n\nSome text.");
        assert_eq!(doc.nodes.len(), 2);
        assert!(doc.nodes[0].is_heading());
        let DocNode::Heading {
            level,
            text,
            source_line,
        } = &doc.nodes[0]
        else {
            panic!("expected Heading")
        };
        assert_eq!(*level, 1);
        assert_eq!(text, "Hello World");
        assert_eq!(*source_line, 0);
    }

    #[test]
    fn joins_soft_wrapped_paragraph_lines() {
        let doc = Document::parse("This is a sentence that\ncontinues here. Next sentence.");
        assert_eq!(doc.nodes.len(), 1);
        let DocNode::Paragraph { text, .. } = &doc.nodes[0] else {
            panic!("expected Paragraph")
        };
        assert_eq!(
            text,
            "This is a sentence that continues here. Next sentence."
        );
    }

    #[test]
    fn parses_unordered_list() {
        let doc = Document::parse("- First item\n- Second item\n- Third item");
        assert_eq!(doc.nodes.len(), 3);
        for node in &doc.nodes {
            assert!(matches!(node, DocNode::ListItem { ordered: false, .. }));
        }
        let DocNode::ListItem { prefix, text, .. } = &doc.nodes[0] else {
            panic!()
        };
        assert_eq!(prefix, "• ");
        assert!(text.starts_with("First item"), "got: {text:?}");
    }

    #[test]
    fn parses_ordered_list() {
        let doc = Document::parse("1. Alpha\n2. Beta\n3. Gamma");
        assert_eq!(doc.nodes.len(), 3);
        let prefixes: Vec<_> = doc
            .nodes
            .iter()
            .filter_map(|n| {
                if let DocNode::ListItem { prefix, .. } = n {
                    Some(prefix.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(prefixes, ["1. ", "2. ", "3. "]);
    }

    #[test]
    fn parses_fenced_code_block() {
        let doc = Document::parse("```rust\nfn main() {}\n```");
        assert_eq!(doc.nodes.len(), 1);
        let DocNode::CodeBlock {
            language, content, ..
        } = &doc.nodes[0]
        else {
            panic!("expected CodeBlock")
        };
        assert_eq!(language.as_deref(), Some("rust"));
        assert!(content.contains("fn main"));
    }

    #[test]
    fn content_node_navigation_includes_headings_and_code() {
        let src = "# Title\n\nText here.\n\n```sh\necho hi\n```\n\nMore text.";
        let doc = Document::parse(src);
        // nodes: [Heading, Paragraph, CodeBlock, Paragraph]
        // Headings and non-empty code blocks count as content.
        assert_eq!(doc.next_content_node(0), Some(0)); // heading is visitable
        assert_eq!(doc.next_content_node(2), Some(2)); // code block is visitable
        assert_eq!(doc.prev_content_node(3), Some(2)); // prev from last paragraph hits code block
    }

    #[test]
    fn nested_list_produces_flat_nodes_with_depth() {
        let src = "- Top\n  - Nested\n- Top2";
        let doc = Document::parse(src);
        let depths: Vec<usize> = doc
            .nodes
            .iter()
            .filter_map(|n| {
                if let DocNode::ListItem { depth, .. } = n {
                    Some(*depth)
                } else {
                    None
                }
            })
            .collect();
        // outer items at depth 0, nested item at depth 1
        assert!(depths.contains(&0));
        assert!(depths.contains(&1));
    }

    #[test]
    fn source_line_mapping() {
        let src = "# Title\n\nParagraph starts here\nand continues.\n\n## Next";
        let doc = Document::parse(src);
        assert_eq!(doc.nodes[0].source_start_line(), 0); // "# Title" on line 0
        assert_eq!(doc.nodes[1].source_start_line(), 2); // paragraph starts on line 2
    }

    // ── block_end / is_block_start ────────────────────────────────────────────

    #[test]
    fn two_consecutive_lists_have_distinct_list_ids() {
        // Unordered then ordered — markdown spec guarantees these are separate List nodes.
        let doc = Document::parse("- A1\n- A2\n\n1. B1\n2. B2");
        assert_eq!(doc.nodes.len(), 4);
        let ids: Vec<usize> = doc
            .nodes
            .iter()
            .filter_map(|n| {
                if let DocNode::ListItem { list_id, .. } = n {
                    Some(*list_id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(ids[0], ids[1], "items from same list share list_id");
        assert_ne!(
            ids[0], ids[2],
            "items from different lists have different list_ids"
        );
        assert_eq!(ids[2], ids[3], "items from same second list share list_id");
    }

    #[test]
    fn block_end_stops_at_first_list_boundary() {
        // Unordered then ordered produces two distinct lists in the AST.
        let doc = Document::parse("- A1\n- A2\n\n1. B1\n2. B2");
        assert_eq!(
            doc.block_end(0),
            1,
            "block_end of list A must not include list B"
        );
        assert_eq!(
            doc.block_end(2),
            3,
            "block_end of list B covers its own items"
        );
    }

    #[test]
    fn is_block_start_true_at_second_list_start() {
        let doc = Document::parse("- A\n- B\n\n1. C\n2. D");
        assert!(doc.is_block_start(0));
        assert!(
            !doc.is_block_start(1),
            "second item of first list is not a block start"
        );
        assert!(
            doc.is_block_start(2),
            "first item of second list IS a block start"
        );
        assert!(
            !doc.is_block_start(3),
            "second item of second list is not a block start"
        );
    }

    #[test]
    fn block_end_for_non_list_node_returns_self() {
        let doc = Document::parse("# Title\n\nParagraph.\n\n```rust\ncode\n```");
        assert_eq!(doc.block_end(0), 0, "heading");
        assert_eq!(doc.block_end(1), 1, "paragraph");
        assert_eq!(doc.block_end(2), 2, "code block");
    }

    #[test]
    fn nested_list_items_share_list_id_and_block_end_spans_all() {
        let doc = Document::parse("- Top\n  - Nested\n- Top2");
        // nodes: [ListItem(depth=0, Top), ListItem(depth=1, Nested), ListItem(depth=0, Top2)]
        let ids: Vec<usize> = doc
            .nodes
            .iter()
            .filter_map(|n| {
                if let DocNode::ListItem { list_id, .. } = n {
                    Some(*list_id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(ids[0], ids[1], "nested item shares list_id with parent");
        assert_eq!(ids[1], ids[2], "all items in the same list share list_id");
        assert_eq!(
            doc.block_end(0),
            2,
            "block_end from first item spans nested and sibling"
        );
    }
}
