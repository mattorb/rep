#![cfg_attr(test, allow(dead_code))]

#[derive(Debug)]
pub struct EmitModel {
    pub source_file: String,
    pub generated_at: String,
    pub keymap: EmitKeymap,
    pub annotations: Vec<EmitLineAnnotation>,
    pub actions: Vec<EmitAction>,
}

#[derive(Debug)]
pub struct EmitKeymap {
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

impl EmitKeymap {
    pub(crate) fn rep_defaults() -> Self {
        Self {
            mode_cycle_forward: "i".to_string(),
            mode_cycle_backward: "o".to_string(),
            unit_next: "j".to_string(),
            unit_prev: "k".to_string(),
            reveal_link: "O".to_string(),
            annotation_prev: "[".to_string(),
            annotation_next: "]".to_string(),
            help: "?".to_string(),
            change: "c".to_string(),
            feedback: "f".to_string(),
            insert_before: "b".to_string(),
            insert_after: "a".to_string(),
            strike: "x".to_string(),
            quit: "q".to_string(),
            quit_silent: "Q".to_string(),
        }
    }
}

#[derive(Debug)]
pub struct EmitLineAnnotation {
    pub line_number: usize,
    pub line_text: String,
    pub context: EmitLineContext,
    pub changes: Vec<EmitChange>,
    pub feedbacks: Vec<EmitFeedback>,
    pub inserts_before: Vec<EmitInsert>,
    pub inserts_after: Vec<EmitInsert>,
    pub reactions: Vec<EmitReaction>,
}

#[derive(Debug)]
pub struct EmitLineContext {
    pub previous_line: Option<String>,
    pub current_line: String,
    pub next_line: Option<String>,
}

#[derive(Debug)]
pub struct EmitChange {
    pub created_at: String,
    pub target_unit: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub change: String,
}

#[derive(Debug)]
pub struct EmitFeedback {
    pub created_at: String,
    pub target_unit: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub feedback: String,
}

#[derive(Debug)]
pub struct EmitInsert {
    pub created_at: String,
    pub target_unit: String,
    pub sentence_index: Option<usize>,
    pub sentence_text: Option<String>,
    pub text: String,
}

#[derive(Debug)]
pub struct EmitReaction {
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

#[derive(Debug)]
pub struct EmitAction {
    pub action: String,
    /// 1-based source line number for human output.
    pub where_line: usize,
    pub context: EmitActionContext,
    pub payload: Option<EmitPayload>,
}

#[derive(Debug)]
pub struct EmitActionContext {
    pub previous_line: Option<String>,
    pub target: String,
    pub next_line: Option<String>,
}

#[derive(Debug)]
pub struct EmitPayload {
    pub key: String,
    pub text: String,
}

pub fn render_human_output(model: &EmitModel) -> String {
    use std::fmt::Write;

    let mut out = String::new();
    let _ = writeln!(out, "FILE: {}", model.source_file);

    if model.actions.is_empty() {
        out.push_str("\nNo actions.\n");
        return out;
    }

    for action in &model.actions {
        out.push('\n');
        let _ = writeln!(out, "ACTION: {}", action.action);
        let _ = writeln!(out, "WHERE: line {}", action.where_line);
        out.push_str("CONTEXT:\n");
        if let Some(previous_line) = &action.context.previous_line {
            let _ = writeln!(out, "  prev: \"{previous_line}\"");
        }
        let _ = writeln!(out, "  target: \"{}\"", action.context.target);
        if let Some(next_line) = &action.context.next_line {
            let _ = writeln!(out, "  next: \"{next_line}\"");
        }
        if let Some(payload) = &action.payload {
            let _ = writeln!(out, "{}: \"{}\"", payload.key, payload.text);
        }
    }

    out
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
    use super::{
        EmitAction, EmitActionContext, EmitKeymap, EmitModel, EmitPayload, clean_context,
        render_human_output,
    };

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

    #[test]
    fn render_human_output_reports_no_actions() {
        let model = EmitModel {
            source_file: "input.md".to_string(),
            generated_at: "2026-01-01T00:00:00Z".to_string(),
            keymap: EmitKeymap::rep_defaults(),
            annotations: Vec::new(),
            actions: Vec::new(),
        };

        assert_eq!(
            render_human_output(&model),
            "FILE: input.md\n\nNo actions.\n"
        );
    }

    #[test]
    fn render_human_output_formats_action_blocks_from_model() {
        let model = EmitModel {
            source_file: "input.md".to_string(),
            generated_at: "2026-01-01T00:00:00Z".to_string(),
            keymap: EmitKeymap::rep_defaults(),
            annotations: Vec::new(),
            actions: vec![EmitAction {
                action: "change".to_string(),
                where_line: 12,
                context: EmitActionContext {
                    previous_line: Some("Before.".to_string()),
                    target: "Target.".to_string(),
                    next_line: Some("After.".to_string()),
                },
                payload: Some(EmitPayload {
                    key: "CHANGE".to_string(),
                    text: "Rewrite it.".to_string(),
                }),
            }],
        };

        assert_eq!(
            render_human_output(&model),
            concat!(
                "FILE: input.md\n",
                "\n",
                "ACTION: change\n",
                "WHERE: line 12\n",
                "CONTEXT:\n",
                "  prev: \"Before.\"\n",
                "  target: \"Target.\"\n",
                "  next: \"After.\"\n",
                "CHANGE: \"Rewrite it.\"\n",
            )
        );
    }
}
