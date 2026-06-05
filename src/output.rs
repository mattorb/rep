#![cfg_attr(test, allow(dead_code))]

#[derive(Debug)]
pub struct AgentOutput {
    pub source_file: String,
    pub generated_at: String,
    pub keymap: KeymapOutput,
    pub annotations: Vec<LineAnnotationOutput>,
}

#[derive(Debug)]
pub struct KeymapOutput {
    /// Cycle the active selection unit forward (section -> paragraph ->
    /// line -> sentence -> word -> section -> ...).
    pub mode_cycle_forward: String,
    /// Cycle the active selection unit backward.
    pub mode_cycle_backward: String,
    /// Move to the next anchor in the active unit (or down a node when on
    /// the last anchor of the current node).
    pub unit_next: String,
    /// Move to the previous anchor in the active unit.
    pub unit_prev: String,
    pub reveal_link: String,
    pub annotation_prev: String,
    pub annotation_next: String,
    pub help: String,
    pub change: String,
    pub feedback: String,
    pub insert_before: String,
    pub insert_after: String,
    /// Sentence-mode only — `x` clears any change/feedback first, then
    /// strikes the sentence on a second press. In other modes it surfaces
    /// a "sentence-only" status message.
    pub strike: String,
    pub quit: String,
    pub quit_silent: String,
}

#[derive(Debug)]
pub struct LineAnnotationOutput {
    pub line_number: usize,
    pub line_text: String,
    pub context: LineContext,
    pub changes: Vec<ChangeOutput>,
    pub feedbacks: Vec<FeedbackOutput>,
    pub inserts_before: Vec<InsertOutput>,
    pub inserts_after: Vec<InsertOutput>,
    pub reactions: Vec<ReactionOutput>,
}

#[derive(Debug)]
pub struct LineContext {
    pub previous_line: Option<String>,
    pub current_line: String,
    pub next_line: Option<String>,
}

#[derive(Debug)]
pub struct ChangeOutput {
    pub created_at: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub change: String,
}

#[derive(Debug)]
pub struct FeedbackOutput {
    pub created_at: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub feedback: String,
}

#[derive(Debug)]
pub struct InsertOutput {
    pub created_at: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub struct ReactionOutput {
    pub kind: String,
    /// Selection unit at the time the reaction (e.g. strike) was applied —
    /// "sentence" / "word" / "line" / "paragraph" / "section".
    pub target_unit: String,
    /// Per-unit index within the node, 1-based.
    pub unit_index: usize,
    /// Captured text of the targeted unit (selection plain text, markers
    /// stripped per Req 11).
    pub target_text: String,
}

pub fn clean_context(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact = compact.replace('"', "\\\"");
    let count = compact.chars().count();
    if count <= max_chars {
        return compact;
    }
    let mut truncated = compact
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::clean_context;

    #[test]
    fn context_cleaning_collapses_whitespace_and_truncates() {
        let cleaned = clean_context(" a   b\tc ", 4);
        assert_eq!(cleaned, "a b…");
    }

    #[test]
    fn zero_max_chars_returns_empty() {
        assert_eq!(clean_context("anything", 0), "");
    }

    #[test]
    fn short_input_passes_through_unchanged() {
        assert_eq!(clean_context("short", 100), "short");
    }

    #[test]
    fn embedded_double_quotes_are_escaped() {
        assert_eq!(clean_context(r#"a "quoted" b"#, 100), r#"a \"quoted\" b"#);
    }

    #[test]
    fn newlines_collapse_to_single_spaces() {
        assert_eq!(
            clean_context("line one\nline two", 100),
            "line one line two"
        );
    }
}
