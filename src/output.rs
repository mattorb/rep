#![cfg_attr(test, allow(dead_code))]

#[cfg(test)]
#[derive(Debug)]
pub struct AgentOutput {
    pub source_file: String,
    pub generated_at: String,
    pub keymap: KeymapOutput,
    pub annotations: Vec<LineAnnotationOutput>,
}

#[cfg(test)]
#[derive(Debug)]
pub struct KeymapOutput {
    pub line_prev: String,
    pub line_next: String,
    pub sentence_prev: String,
    pub sentence_next: String,
    pub reveal_link: String,
    pub section_prev: String,
    pub section_next: String,
    pub paragraph_prev: String,
    pub paragraph_next: String,
    pub annotation_prev: String,
    pub annotation_next: String,
    pub help: String,
    pub change: String,
    pub feedback: String,
    pub strike: String,
    pub quit: String,
    pub quit_silent: String,
}

#[cfg(test)]
#[derive(Debug)]
pub struct LineAnnotationOutput {
    pub line_number: usize,
    pub line_text: String,
    pub context: LineContext,
    pub changes: Vec<ChangeOutput>,
    pub feedbacks: Vec<FeedbackOutput>,
    pub reactions: Vec<ReactionOutput>,
}

#[cfg(test)]
#[derive(Debug)]
pub struct LineContext {
    pub previous_line: Option<String>,
    pub current_line: String,
    pub next_line: Option<String>,
}

#[cfg(test)]
#[derive(Debug)]
pub struct ChangeOutput {
    pub created_at: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub change: String,
}

#[cfg(test)]
#[derive(Debug)]
pub struct FeedbackOutput {
    pub created_at: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub feedback: String,
}

#[cfg(test)]
#[derive(Debug)]
pub struct ReactionOutput {
    pub kind: String,
    pub sentence_index: usize,
    pub sentence_text: String,
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
}
