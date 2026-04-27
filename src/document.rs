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
        #[cfg_attr(not(test), allow(dead_code))]
        text: String,
        sentences: Vec<String>,
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
        sentences: Vec<String>,
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
    #[allow(dead_code)]
    pub fn sentences(&self) -> &[String] {
        match self {
            Self::Paragraph { sentences, .. } | Self::ListItem { sentences, .. } => sentences,
            _ => &[],
        }
    }

    pub fn has_sentences(&self) -> bool {
        match self {
            Self::Paragraph { sentences, .. } | Self::ListItem { sentences, .. } => {
                !sentences.is_empty()
            }
            Self::Heading { text, .. } => !text.is_empty(),
            Self::CodeBlock { content, .. } => !content.is_empty(),
            Self::ThematicBreak { .. } => false,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_heading(&self) -> bool {
        matches!(self, Self::Heading { .. })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_paragraph(&self) -> bool {
        matches!(self, Self::Paragraph { .. })
    }

    /// True for markdown headings and top-level ordered list items (numbered sections).
    pub fn is_section(&self) -> bool {
        matches!(self, Self::Heading { .. })
            || matches!(
                self,
                Self::ListItem {
                    ordered: true,
                    depth: 0,
                    ..
                }
            )
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
        let ast = match markdown::to_mdast(content, &options) {
            Ok(node) => node,
            Err(_) => return Self { nodes: Vec::new() },
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
            let source_line = h.position.as_ref().map(|p| p.start.line - 1).unwrap_or(0);
            out.push(DocNode::Heading {
                level: h.depth,
                text,
                source_line,
            });
        }
        mdast::Node::Paragraph(p) => {
            let raw = extract_plain_text(node);
            let text = normalize_whitespace(&raw);
            let sentences = text_to_sentences(&text);
            let source_lines = p
                .position
                .as_ref()
                .map(|pos| (pos.start.line - 1)..pos.end.line)
                .unwrap_or(0..1);
            out.push(DocNode::Paragraph {
                text,
                sentences,
                source_lines,
            });
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
                    let sentences = text_to_sentences(&text);
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
                        sentences,
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
            let source_line = tb.position.as_ref().map(|p| p.start.line - 1).unwrap_or(0);
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
                                Some(extract_plain_text(c).split_whitespace().collect::<Vec<_>>().join(" "))
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
            out.push(DocNode::Paragraph {
                text,
                sentences: Vec::new(),
                source_lines,
            });
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
            out.push(DocNode::Paragraph {
                text,
                sentences: Vec::new(),
                source_lines,
            });
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

fn text_to_sentences(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut result: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < len {
        if matches!(bytes[i], b'.' | b'!' | b'?')
            && i + 2 < len
            && bytes[i + 1] == b' '
            && bytes[i + 2].is_ascii_uppercase()
        {
            let s = text[start..i + 1].trim();
            if !s.is_empty() {
                result.push(s.to_string());
            }
            i += 2;
            start = i;
            continue;
        }
        i += 1;
    }
    let s = text[start..].trim();
    if !s.is_empty() {
        result.push(s.to_string());
    }
    if result.is_empty() {
        result.push(text.trim().to_string());
    }
    result
}

// ── Navigation helpers ────────────────────────────────────────────────────────

impl Document {
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Next node of any type at or after `from`.
    #[allow(dead_code)]
    pub fn next_node(&self, from: usize) -> Option<usize> {
        if from < self.nodes.len() {
            Some(from)
        } else {
            None
        }
    }

    /// Previous node of any type strictly before `before`.
    #[allow(dead_code)]
    pub fn prev_node(&self, before: usize) -> Option<usize> {
        before.checked_sub(1).filter(|&i| i < self.nodes.len())
    }

    /// First node at or after `from` that has at least one sentence.
    pub fn next_node_with_sentences(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.nodes[i].has_sentences())
    }

    /// Last node strictly before `before` that has at least one sentence.
    pub fn prev_node_with_sentences(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.nodes[i].has_sentences())
    }

    /// First section node (heading or top-level ordered list) at or after `from`.
    pub fn next_section(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.nodes[i].is_section())
    }

    /// Last section node strictly before `before`.
    pub fn prev_section(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.nodes[i].is_section())
    }

    // kept for tests
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn next_heading(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.nodes[i].is_heading())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn prev_heading(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.nodes[i].is_heading())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn next_paragraph(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.nodes[i].is_paragraph())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn prev_paragraph(&self, before: usize) -> Option<usize> {
        (0..before).rev().find(|&i| self.nodes[i].is_paragraph())
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

    /// First block-start node at or after `from` that has content.
    pub fn next_block(&self, from: usize) -> Option<usize> {
        (from..self.nodes.len()).find(|&i| self.is_block_start(i) && self.nodes[i].has_sentences())
    }

    /// Last block-start node strictly before `before` that has content.
    pub fn prev_block(&self, before: usize) -> Option<usize> {
        (0..before)
            .rev()
            .find(|&i| self.is_block_start(i) && self.nodes[i].has_sentences())
    }

    /// Last node index that belongs to the same conceptual block as `start`.
    /// For list items all sharing the same list_id, returns the last such item.
    /// For paragraphs/headings/code blocks, returns `start` itself.
    pub fn block_end(&self, start: usize) -> usize {
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
        let DocNode::Paragraph {
            text, sentences, ..
        } = &doc.nodes[0]
        else {
            panic!("expected Paragraph")
        };
        assert_eq!(
            text,
            "This is a sentence that continues here. Next sentence."
        );
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0], "This is a sentence that continues here.");
        assert_eq!(sentences[1], "Next sentence.");
    }

    #[test]
    fn parses_unordered_list() {
        let doc = Document::parse("- First item\n- Second item\n- Third item");
        assert_eq!(doc.nodes.len(), 3);
        for node in &doc.nodes {
            assert!(matches!(node, DocNode::ListItem { ordered: false, .. }));
        }
        let DocNode::ListItem {
            prefix, sentences, ..
        } = &doc.nodes[0]
        else {
            panic!()
        };
        assert_eq!(prefix, "• ");
        assert!(sentences[0].starts_with("First item"), "got: {sentences:?}");
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
    fn navigation_next_and_prev_heading() {
        let src = "# Section 1\n\nParagraph.\n\n## Section 2\n\nMore text.";
        let doc = Document::parse(src);
        // nodes: [Heading(1), Paragraph, Heading(2), Paragraph]
        assert_eq!(doc.next_heading(0), Some(0));
        assert_eq!(doc.next_heading(1), Some(2));
        assert_eq!(doc.prev_heading(2), Some(0));
        assert_eq!(doc.prev_heading(0), None);
    }

    #[test]
    fn navigation_next_and_prev_paragraph() {
        let src = "# Section 1\n\nFirst paragraph.\n\nSecond paragraph.";
        let doc = Document::parse(src);
        // nodes: [Heading, Paragraph, Paragraph]
        assert_eq!(doc.next_paragraph(0), Some(1));
        assert_eq!(doc.next_paragraph(2), Some(2));
        assert_eq!(doc.prev_paragraph(2), Some(1));
        assert_eq!(doc.prev_paragraph(1), None);
    }

    #[test]
    fn navigation_nodes_with_sentences_includes_headings_and_code() {
        let src = "# Title\n\nText here.\n\n```sh\necho hi\n```\n\nMore text.";
        let doc = Document::parse(src);
        // nodes: [Heading, Paragraph, CodeBlock, Paragraph]
        // Headings and non-empty code blocks now count as having sentences.
        assert_eq!(doc.next_node_with_sentences(0), Some(0)); // heading is visitable
        assert_eq!(doc.next_node_with_sentences(2), Some(2)); // code block is visitable
        assert_eq!(doc.prev_node_with_sentences(3), Some(2)); // prev from last paragraph hits code block
    }

    #[test]
    fn list_item_with_multiple_sentences() {
        let doc = Document::parse("- First sentence. Second sentence. Third.");
        assert_eq!(doc.nodes.len(), 1);
        let DocNode::ListItem { sentences, .. } = &doc.nodes[0] else {
            panic!()
        };
        assert_eq!(sentences.len(), 3);
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

    // ── text_to_sentences ─────────────────────────────────────────────────────

    #[test]
    fn text_to_sentences_splits_on_question_and_exclamation() {
        let result = text_to_sentences("Is it done? Yes it is! Now move on.");
        assert_eq!(result.len(), 3, "{result:?}");
        assert_eq!(result[0], "Is it done?");
        assert_eq!(result[1], "Yes it is!");
        assert_eq!(result[2], "Now move on.");
    }

    #[test]
    fn text_to_sentences_no_terminal_punctuation_is_one_sentence() {
        let result = text_to_sentences("A sentence without a period");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "A sentence without a period");
    }

    #[test]
    fn text_to_sentences_empty_and_whitespace_returns_empty() {
        assert!(text_to_sentences("").is_empty());
        assert!(text_to_sentences("   ").is_empty());
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
