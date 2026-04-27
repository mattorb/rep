use pulldown_cmark::{
    CodeBlockKind, Event as MdEvent, HeadingLevel, Options as MdOptions, Parser as MdParser,
    Tag as MdTag, TagEnd,
};
use ratatui::prelude::*;

#[derive(Debug, Clone)]
pub struct RenderedMarkdownLine {
    pub plain: String,
    pub spans: Vec<Span<'static>>,
    pub links: Vec<MarkdownLinkRange>,
}

#[derive(Debug, Clone)]
pub struct MarkdownLinkRange {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

#[derive(Debug, Clone)]
struct MarkdownListState {
    ordered: bool,
    next_index: u64,
}

#[derive(Debug, Default)]
struct MarkdownLineRenderer {
    spans: Vec<Span<'static>>,
    plain: String,
    links: Vec<MarkdownLinkRange>,
    heading_level: Option<HeadingLevel>,
    heading_prefix_pending: bool,
    blockquote_depth: usize,
    list_stack: Vec<MarkdownListState>,
    pending_list_prefix: Option<String>,
    emphasis_depth: usize,
    strong_depth: usize,
    strike_depth: usize,
    in_code_block: bool,
    saw_indented_code_block: bool,
    active_link: Option<String>,
    active_image_alt: Option<String>,
}

pub fn render_markdown_line(line: &str) -> RenderedMarkdownLine {
    let parser = MdParser::new_ext(line, markdown_options());
    let mut renderer = MarkdownLineRenderer::default();
    for event in parser {
        renderer.handle_event(event);
    }
    let saw_indented_code_block = renderer.saw_indented_code_block;
    let rendered = renderer.finish();
    if saw_indented_code_block {
        return RenderedMarkdownLine {
            plain: line.to_owned(),
            spans: vec![Span::raw(line.to_owned())],
            links: Vec::new(),
        };
    }
    if rendered.plain.is_empty() && !line.is_empty() {
        RenderedMarkdownLine {
            plain: line.to_owned(),
            spans: vec![Span::raw(line.to_owned())],
            links: Vec::new(),
        }
    } else {
        rendered
    }
}

fn markdown_options() -> MdOptions {
    let mut options = MdOptions::empty();
    options.insert(MdOptions::ENABLE_GFM);
    options.insert(MdOptions::ENABLE_TABLES);
    options.insert(MdOptions::ENABLE_TASKLISTS);
    options.insert(MdOptions::ENABLE_STRIKETHROUGH);
    options.insert(MdOptions::ENABLE_SMART_PUNCTUATION);
    options.insert(MdOptions::ENABLE_DEFINITION_LIST);
    options
}

const fn heading_level_number(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn heading_style(level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H2 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H3 => Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
        HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    }
}

impl MarkdownLineRenderer {
    fn handle_event(&mut self, event: MdEvent<'_>) {
        match event {
            MdEvent::Start(tag) => self.handle_tag_start(tag),
            MdEvent::End(end) => self.handle_tag_end(end),
            MdEvent::Text(text) => {
                if let Some(alt) = self.active_image_alt.as_mut() {
                    alt.push_str(&text);
                } else {
                    self.push_text(&text, false);
                }
            }
            MdEvent::Code(code) => {
                if let Some(alt) = self.active_image_alt.as_mut() {
                    alt.push_str(&code);
                } else {
                    self.push_text(&code, true);
                }
            }
            MdEvent::SoftBreak | MdEvent::HardBreak => {
                self.push_text_with_style(" ", self.base_inline_style(), None);
            }
            MdEvent::Rule => {
                self.push_text_with_style(
                    "────────────────────────",
                    Style::default().fg(Color::DarkGray),
                    None,
                );
            }
            MdEvent::Html(html) | MdEvent::InlineHtml(html) => {
                self.push_text_with_style(&html, Style::default().fg(Color::DarkGray), None);
            }
            MdEvent::TaskListMarker(checked) => {
                self.push_text_with_style(
                    if checked { "[x] " } else { "[ ] " },
                    self.base_inline_style(),
                    None,
                );
            }
            MdEvent::FootnoteReference(label) => {
                self.push_text_with_style(
                    &format!("[^{label}]"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                    None,
                );
            }
            MdEvent::InlineMath(math) | MdEvent::DisplayMath(math) => {
                self.push_text_with_style(
                    &format!("${math}$"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                    None,
                );
            }
        }
    }

    fn handle_tag_start(&mut self, tag: MdTag<'_>) {
        match tag {
            MdTag::Paragraph => {}
            MdTag::Heading { level, .. } => {
                self.heading_level = Some(level);
                self.heading_prefix_pending = true;
            }
            MdTag::BlockQuote(_) => {
                self.blockquote_depth += 1;
            }
            MdTag::CodeBlock(kind) => {
                match kind {
                    // This app parses Markdown line-by-line, so a line that starts with
                    // four spaces is often just a wrapped continuation line in a list,
                    // not an intentional code block. Preserve such lines as raw text.
                    CodeBlockKind::Indented => {
                        self.saw_indented_code_block = true;
                    }
                    CodeBlockKind::Fenced(lang) => {
                        self.in_code_block = true;
                        let label = if lang.trim().is_empty() {
                            "```".to_owned()
                        } else {
                            format!("```{}", lang.trim())
                        };
                        self.push_text_with_style(
                            &label,
                            Style::default().fg(Color::DarkGray),
                            None,
                        );
                        self.push_text_with_style(" ", Style::default(), None);
                    }
                }
            }
            MdTag::List(start) => {
                self.list_stack.push(MarkdownListState {
                    ordered: start.is_some(),
                    next_index: start.unwrap_or(1),
                });
            }
            MdTag::Item => {
                self.pending_list_prefix = Some(self.next_list_prefix());
            }
            MdTag::Link { dest_url, .. } => {
                self.active_link = Some(dest_url.to_string());
            }
            MdTag::Image { .. } => {
                self.active_image_alt = Some(String::new());
            }
            MdTag::Emphasis => self.emphasis_depth += 1,
            MdTag::Strong => self.strong_depth += 1,
            MdTag::Strikethrough => self.strike_depth += 1,
            MdTag::Table(_)
            | MdTag::TableHead
            | MdTag::TableRow
            | MdTag::TableCell
            | MdTag::HtmlBlock
            | MdTag::FootnoteDefinition(_)
            | MdTag::DefinitionList
            | MdTag::DefinitionListTitle
            | MdTag::DefinitionListDefinition
            | MdTag::MetadataBlock(_) => {}
        }
    }

    fn handle_tag_end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Heading(_) => {
                self.heading_level = None;
                self.heading_prefix_pending = false;
            }
            TagEnd::BlockQuote(_) => {
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if self.in_code_block {
                    self.in_code_block = false;
                    self.push_text_with_style(" ```", Style::default().fg(Color::DarkGray), None);
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::Link => self.active_link = None,
            TagEnd::Image => {
                let alt = self.active_image_alt.take().unwrap_or_default();
                let label = if alt.trim().is_empty() {
                    "[image]".to_owned()
                } else {
                    format!("[image: {}]", alt.trim())
                };
                self.push_text_with_style(
                    &label,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::UNDERLINED | Modifier::ITALIC),
                    None,
                );
            }
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strong => self.strong_depth = self.strong_depth.saturating_sub(1),
            TagEnd::Strikethrough => self.strike_depth = self.strike_depth.saturating_sub(1),
            TagEnd::Paragraph
            | TagEnd::Item
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn finish(self) -> RenderedMarkdownLine {
        RenderedMarkdownLine {
            plain: self.plain,
            spans: self.spans,
            links: self.links,
        }
    }

    fn next_list_prefix(&mut self) -> String {
        let depth = self.list_stack.len().saturating_sub(1);
        let indent = "  ".repeat(depth);
        if let Some(list) = self.list_stack.last_mut() {
            if list.ordered {
                let current = list.next_index;
                list.next_index += 1;
                format!("{indent}{current}. ")
            } else {
                format!("{indent}• ")
            }
        } else {
            "• ".to_owned()
        }
    }

    fn base_inline_style(&self) -> Style {
        let mut style = Style::default();
        if let Some(level) = self.heading_level {
            style = style.patch(heading_style(level));
        }
        if self.emphasis_depth > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strong_depth > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.strike_depth > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        style
    }

    fn ensure_prefixes(&mut self) {
        if !self.plain.is_empty() {
            return;
        }

        if self.blockquote_depth > 0 {
            let prefix = "│ ".repeat(self.blockquote_depth);
            self.append_part(&prefix, Style::default().fg(Color::DarkGray), None);
        }

        if let Some(prefix) = self.pending_list_prefix.take() {
            self.append_part(&prefix, Style::default().fg(Color::DarkGray), None);
        }

        if self.heading_prefix_pending {
            if let Some(level) = self.heading_level {
                let marker = format!("{} ", "#".repeat(heading_level_number(level)));
                self.append_part(&marker, heading_style(level), None);
            }
            self.heading_prefix_pending = false;
        }
    }

    fn push_text(&mut self, text: &str, inline_code: bool) {
        let mut style = self.base_inline_style();
        if inline_code || self.in_code_block {
            style = style.patch(
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            );
        }
        let active_link = if self.in_code_block {
            None
        } else {
            self.active_link.clone()
        };
        self.push_text_with_style(text, style, active_link.as_deref());
    }

    fn push_text_with_style(&mut self, text: &str, style: Style, link_url: Option<&str>) {
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.ensure_prefixes();
                self.append_part(" ", style, link_url);
            }
            if part.is_empty() {
                continue;
            }
            self.ensure_prefixes();
            self.append_part(part, style, link_url);
        }
    }

    fn append_part(&mut self, part: &str, style: Style, link_url: Option<&str>) {
        let start = self.plain.len();
        self.plain.push_str(part);
        let end = self.plain.len();

        let style = if link_url.is_some() && !self.in_code_block {
            style.patch(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
            )
        } else {
            style
        };
        self.spans.push(Span::styled(part.to_owned(), style));

        if let Some(url) = link_url.filter(|url| !url.trim().is_empty())
            && end > start
        {
            self.links.push(MarkdownLinkRange {
                start,
                end,
                url: url.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::render_markdown_line;

    #[test]
    fn renders_markdown_headings() {
        let rendered = render_markdown_line("# Top");
        assert_eq!(rendered.plain, "# Top");
    }

    #[test]
    fn renders_markdown_lists() {
        let rendered = render_markdown_line("- first");
        assert_eq!(rendered.plain, "• first");
    }

    #[test]
    fn indented_wrapped_lines_are_not_rendered_as_code_blocks() {
        let line = "    --stdin, --version)";
        let rendered = render_markdown_line(line);
        assert_eq!(rendered.plain, line);
        assert_eq!(rendered.spans.len(), 1);
    }
}
