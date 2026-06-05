use ratatui::prelude::*;
use unicode_width::UnicodeWidthChar;

pub(crate) fn wrap_styled_spans(
    spans: Vec<Span<'static>>,
    width: usize,
) -> Vec<Vec<Span<'static>>> {
    let mut w = Wrapper::new(width.max(1));
    for span in &spans {
        w.process_span(span);
    }
    w.finish()
}

struct Wrapper {
    lines: Vec<Vec<Span<'static>>>,
    col: usize,
    width: usize,
    ws_buf: String,
    ws_style: Style,
    ws_width: usize,
    word_buf: String,
    word_style: Style,
    word_width: usize,
    /// True after an explicit `\n` char, until the first non-whitespace is written.
    /// In this state leading whitespace is emitted as indentation rather than discarded.
    indent_mode: bool,
}

impl Wrapper {
    fn new(width: usize) -> Self {
        Self {
            lines: vec![Vec::new()],
            col: 0,
            width,
            ws_buf: String::new(),
            ws_style: Style::default(),
            ws_width: 0,
            word_buf: String::new(),
            word_style: Style::default(),
            word_width: 0,
            indent_mode: false,
        }
    }

    fn emit(&mut self, text: String, style: Style) {
        if text.is_empty() {
            return;
        }
        if let Some(line) = self.lines.last_mut() {
            line.push(Span::styled(text, style));
        }
    }

    fn new_line(&mut self) {
        self.lines.push(Vec::new());
        self.col = 0;
        self.ws_buf.clear();
        self.ws_width = 0;
    }

    fn flush_word(&mut self) {
        if self.word_buf.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word_buf);
        let style = self.word_style;
        let word_width = self.word_width;
        self.word_width = 0;

        if self.col == 0 {
            // Start of line: discard any pending leading whitespace.
            self.ws_buf.clear();
            self.ws_width = 0;
            self.indent_mode = false;
            if word_width <= self.width {
                self.emit(word, style);
                self.col = word_width;
            } else {
                self.force_break(&word, style);
            }
        } else if self.col + self.ws_width + word_width <= self.width {
            // Word fits on the current line with its preceding whitespace.
            let ws = std::mem::take(&mut self.ws_buf);
            let ws_style = self.ws_style;
            let ws_w = self.ws_width;
            self.ws_width = 0;
            self.emit(ws, ws_style);
            self.col += ws_w;
            self.emit(word, style);
            self.col += word_width;
        } else {
            // Word doesn't fit: wrap to the next line.
            self.ws_buf.clear();
            self.ws_width = 0;
            self.indent_mode = false; // auto-wrap: no explicit indentation on continuation
            self.new_line();
            if word_width <= self.width {
                self.emit(word, style);
                self.col = word_width;
            } else {
                self.force_break(&word, style);
            }
        }
    }

    // Character-by-character fallback for words wider than the full line.
    fn force_break(&mut self, word: &str, style: Style) {
        let mut buf = String::new();
        for ch in word.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
            if self.col + ch_width > self.width && self.col > 0 {
                self.emit(std::mem::take(&mut buf), style);
                self.new_line();
            }
            buf.push(ch);
            self.col += ch_width;
        }
        self.emit(buf, style);
    }

    fn process_span(&mut self, span: &Span<'static>) {
        let style = span.style;
        for ch in span.content.chars() {
            if ch == '\n' {
                self.flush_word();
                self.ws_buf.clear();
                self.ws_width = 0;
                self.new_line();
                self.indent_mode = true;
                continue;
            }

            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);

            if ch.is_whitespace() {
                if !self.word_buf.is_empty() {
                    self.flush_word();
                }
                if self.col == 0 && self.indent_mode {
                    // Leading whitespace after an explicit \n: emit as indentation.
                    self.emit(ch.to_string(), style);
                    self.col += ch_width;
                } else if self.col > 0 {
                    // Buffer whitespace between words.
                    if self.ws_buf.is_empty() {
                        self.ws_style = style;
                    }
                    self.ws_buf.push(ch);
                    self.ws_width += ch_width;
                }
                // else col==0 && !indent_mode: discard (leading ws after auto-wrap)
            } else {
                self.indent_mode = false;
                if self.word_buf.is_empty() {
                    self.word_style = style;
                }
                self.word_buf.push(ch);
                self.word_width += ch_width;
            }
        }
        // Flush any completed word at the end of the span.
        if !self.word_buf.is_empty() {
            self.flush_word();
        }
    }

    fn finish(mut self) -> Vec<Vec<Span<'static>>> {
        self.flush_word();
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_lines(spans: Vec<Span<'static>>, width: usize) -> Vec<String> {
        wrap_styled_spans(spans, width)
            .into_iter()
            .map(|line| line.into_iter().map(|s| s.content.into_owned()).collect())
            .collect()
    }

    #[test]
    fn explicit_newline_indentation_preserved() {
        // A span containing "\n  " should render the 2-space indent on the second line,
        // not discard it (which the col==0 whitespace-discard logic used to do).
        let spans = vec![
            Span::raw("Title\n".to_owned()),
            Span::raw("  ".to_owned()),
            Span::raw("Indented".to_owned()),
        ];
        let lines = plain_lines(spans, 80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Title");
        assert_eq!(
            lines[1], "  Indented",
            "indentation must be preserved after \\n"
        );
    }

    #[test]
    fn word_wrap_does_not_indent_continuation() {
        // When a word wraps due to line length, the wrapped line must NOT be indented.
        let spans = vec![Span::raw("word1 word2 word3".to_owned())];
        let lines = plain_lines(spans, 12);
        assert!(
            lines[1].starts_with("word"),
            "wrapped line should start directly with word, got {lines:?}"
        );
    }

    #[test]
    fn word_wider_than_line_force_breaks_character_by_character() {
        // A single word longer than `width` cannot be wrapped at a space —
        // force_break splits it character-by-character so every line stays
        // within the budget and no characters are dropped.
        let spans = vec![Span::raw("abcdefghij".to_owned())];
        let lines = plain_lines(spans, 4);
        // Each line must be no wider than 4 columns; together they must
        // contain every original character in order.
        for line in &lines {
            assert!(line.chars().count() <= 4, "line {line:?} exceeds width 4");
        }
        let rejoined: String = lines.concat();
        assert_eq!(rejoined, "abcdefghij");
    }

    #[test]
    fn force_break_handles_wide_unicode_chars() {
        // CJK chars are 2 columns; the budget must respect column width,
        // not character count, so a 4-column budget fits 2 CJK chars per
        // line, not 4.
        let spans = vec![Span::raw("日本語テスト".to_owned())];
        let lines = plain_lines(spans, 4);
        // Each line should be at most 2 chars (each costs 2 columns).
        for line in &lines {
            assert!(
                line.chars().count() <= 2,
                "line {line:?} exceeds 2 wide chars in 4-col budget"
            );
        }
        let rejoined: String = lines.concat();
        assert_eq!(rejoined, "日本語テスト");
    }
}
