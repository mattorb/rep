#![allow(dead_code)]

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
    /// Adjust the active selection unit one step finer without wrapping.
    pub selection_finer: String,
    /// Adjust the active selection unit one step coarser without wrapping.
    pub selection_coarser: String,
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
            mode_cycle_forward: "Space".to_string(),
            mode_cycle_backward: "Backspace".to_string(),
            unit_next: "j".to_string(),
            unit_prev: "k".to_string(),
            selection_finer: "i".to_string(),
            selection_coarser: "o".to_string(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyBindingDocRow {
    pub keys: String,
    pub action: String,
}

pub(crate) fn keybinding_doc_rows() -> Vec<KeyBindingDocRow> {
    let keymap = EmitKeymap::rep_defaults();
    vec![
        KeyBindingDocRow {
            keys: format!("`{}`, `Down`, `Right`", keymap.unit_next),
            action: "Move to the next active unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`, `Up`, `Left`", keymap.unit_prev),
            action: "Move to the previous active unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.mode_cycle_forward),
            action: "Cycle to the next selection unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.mode_cycle_backward),
            action: "Cycle to the previous selection unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.selection_finer),
            action: "Use a finer selection unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.selection_coarser),
            action: "Use a coarser selection unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.change),
            action: "Add or edit a literal change request".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.feedback),
            action: "Add or edit feedback or intent".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.insert_before),
            action: "Insert text before the current unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.insert_after),
            action: "Insert text after the current unit".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.strike),
            action: "Clear existing annotations or mark the unit for deletion".to_string(),
        },
        KeyBindingDocRow {
            keys: "`e`".to_string(),
            action: "Edit an existing annotation".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`, `{}`", keymap.annotation_prev, keymap.annotation_next),
            action: "Jump to the previous or next annotation".to_string(),
        },
        KeyBindingDocRow {
            keys: "`/`".to_string(),
            action: "Search".to_string(),
        },
        KeyBindingDocRow {
            keys: "`n`, `N`".to_string(),
            action: "Jump to the next or previous search match".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`, `Shift` + `/`", keymap.help),
            action: "Open or close help".to_string(),
        },
        KeyBindingDocRow {
            keys: "`I`".to_string(),
            action: "Open or close the AST view".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.reveal_link),
            action: "Reveal markdown links for the current sentence".to_string(),
        },
        KeyBindingDocRow {
            keys: "`r`".to_string(),
            action: "Copy annotations to the clipboard".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.quit),
            action: "Quit and print annotations to stdout".to_string(),
        },
        KeyBindingDocRow {
            keys: format!("`{}`", keymap.quit_silent),
            action: "Quit silently and discard annotations".to_string(),
        },
        KeyBindingDocRow {
            keys: "`Enter`".to_string(),
            action: "Save text in change, feedback, insert, edit, or search modes".to_string(),
        },
        KeyBindingDocRow {
            keys: "`Esc`".to_string(),
            action: "Cancel the current input mode or close an open popup".to_string(),
        },
    ]
}

pub(crate) fn readme_keybinding_table() -> String {
    let mut table = String::from("| Key | Action |\n| --- | --- |\n");
    for row in keybinding_doc_rows() {
        table.push_str(&format!("| {} | {} |\n", row.keys, row.action));
    }
    table
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
        readme_keybinding_table, render_human_output,
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
    fn rep_default_keymap_matches_runtime_mode_cycle_keys() {
        let keymap = EmitKeymap::rep_defaults();
        assert_eq!(keymap.mode_cycle_forward, "Space");
        assert_eq!(keymap.mode_cycle_backward, "Backspace");
        assert_eq!(keymap.selection_finer, "i");
        assert_eq!(keymap.selection_coarser, "o");
    }

    #[test]
    fn readme_keybindings_match_default_keymap() {
        let readme = include_str!("../README.md");
        let start = readme
            .find("| Key | Action |")
            .expect("README contains keybinding table");
        let rest = &readme[start..];
        let end = rest
            .find("\n\n## ")
            .expect("README keybinding table is followed by a heading");
        assert_eq!(&rest[..end + 1], readme_keybinding_table());
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
