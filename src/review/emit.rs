use crate::output::{
    EmitAction, EmitActionContext, EmitChange, EmitFeedback, EmitInsert, EmitKeymap,
    EmitLineAnnotation, EmitLineContext, EmitModel, EmitPayload, EmitReaction, clean_context,
};
use crate::review::annotation::InsertAnnotation;
use crate::review::document::{ActionContext, CapturedTarget, ReviewDocument};
use crate::review::session::ReviewSession;
use crate::selection::model::{SelectionAnchor, SelectionUnit};

const TARGET_MAX_CHARS: usize = 180;
const CONTEXT_MAX_CHARS: usize = 140;
const PAYLOAD_MAX_CHARS: usize = 220;

impl ReviewSession {
    pub(crate) fn emit_model(
        &self,
        document: &impl ReviewDocument,
        generated_at: String,
    ) -> EmitModel {
        let mut annotations = Vec::new();
        let mut actions = Vec::new();

        for node_idx in self.annotations.touched_nodes() {
            let line_context = document.node_source_context(node_idx);

            let changes = self
                .annotations
                .changes
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|annotation| {
                    let target = stored_target(
                        document,
                        node_idx,
                        annotation.target_unit,
                        annotation.sentence_index,
                        annotation.sentence_text.clone(),
                    );
                    actions.push(annotation_action(
                        document.action_context(&target),
                        "change",
                        "CHANGE",
                        &annotation.change,
                    ));
                    EmitChange {
                        created_at: annotation.created_at,
                        target_unit: annotation.target_unit.mode_str().to_string(),
                        sentence_index: annotation.sentence_index.map(|index| index + 1),
                        sentence_text: annotation.sentence_text,
                        change: annotation.change,
                    }
                })
                .collect();

            let feedbacks = self
                .annotations
                .feedbacks
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|annotation| {
                    let target = stored_target(
                        document,
                        node_idx,
                        annotation.target_unit,
                        annotation.sentence_index,
                        annotation.sentence_text.clone(),
                    );
                    actions.push(annotation_action(
                        document.action_context(&target),
                        "revise-to-incorporate-feedback",
                        "FEEDBACK",
                        &annotation.feedback,
                    ));
                    EmitFeedback {
                        created_at: annotation.created_at,
                        target_unit: annotation.target_unit.mode_str().to_string(),
                        sentence_index: annotation.sentence_index.map(|index| index + 1),
                        sentence_text: annotation.sentence_text,
                        feedback: annotation.feedback,
                    }
                })
                .collect();

            let mut map_inserts =
                |action: &str, bucket: Option<&Vec<InsertAnnotation>>| -> Vec<EmitInsert> {
                    bucket
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|annotation| {
                            let target = stored_target(
                                document,
                                node_idx,
                                annotation.target_unit,
                                annotation.sentence_index,
                                annotation.sentence_text.clone(),
                            );
                            actions.push(annotation_action(
                                document.action_context(&target),
                                action,
                                "INSERT",
                                &annotation.text,
                            ));
                            EmitInsert {
                                created_at: annotation.created_at,
                                target_unit: annotation.target_unit.mode_str().to_string(),
                                sentence_index: annotation.sentence_index.map(|index| index + 1),
                                sentence_text: annotation.sentence_text,
                                text: annotation.text,
                            }
                        })
                        .collect()
                };
            let inserts_before = map_inserts(
                "insert-before",
                self.annotations.inserts_before.get(&node_idx),
            );
            let inserts_after = map_inserts(
                "insert-after",
                self.annotations.inserts_after.get(&node_idx),
            );

            let reactions = self
                .annotations
                .strikes
                .get(&node_idx)
                .map(|set| {
                    set.iter()
                        .map(|&(unit, unit_idx)| {
                            let target = document
                                .capture_target(SelectionAnchor::new(node_idx, unit, unit_idx));
                            actions.push(action_model(
                                "delete this",
                                document.action_context(&target),
                                None,
                            ));
                            EmitReaction {
                                kind: "strike".to_string(),
                                target_unit: unit.mode_str().to_string(),
                                unit_index: unit_idx + 1,
                                target_text: target.text.unwrap_or_default(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            annotations.push(EmitLineAnnotation {
                line_number: line_context.source_line + 1,
                line_text: line_context.line_text.clone(),
                context: EmitLineContext {
                    previous_line: line_context.previous,
                    current_line: line_context.line_text,
                    next_line: line_context.next,
                },
                changes,
                feedbacks,
                inserts_before,
                inserts_after,
                reactions,
            });
        }

        EmitModel {
            source_file: document.source_path().display().to_string(),
            generated_at,
            keymap: EmitKeymap::rep_defaults(),
            annotations,
            actions,
        }
    }
}

fn stored_target(
    document: &impl ReviewDocument,
    node_idx: usize,
    unit: SelectionUnit,
    unit_idx: Option<usize>,
    text: Option<String>,
) -> CapturedTarget {
    let mut target =
        document.capture_target(SelectionAnchor::new(node_idx, unit, unit_idx.unwrap_or(0)));
    target.text = text;
    target
}

fn annotation_action(
    context: ActionContext,
    action: &str,
    payload_key: &str,
    payload_text: &str,
) -> EmitAction {
    action_model(
        action,
        context,
        Some(EmitPayload {
            key: payload_key.to_string(),
            text: clean_context(payload_text, PAYLOAD_MAX_CHARS),
        }),
    )
}

fn action_model(action: &str, context: ActionContext, payload: Option<EmitPayload>) -> EmitAction {
    let previous = clean_context(&context.previous, CONTEXT_MAX_CHARS);
    let target = clean_context(&context.target, TARGET_MAX_CHARS);
    let next = clean_context(&context.next, CONTEXT_MAX_CHARS);
    EmitAction {
        action: action.to_string(),
        where_line: context.where_line + 1,
        context: EmitActionContext {
            previous_line: (!previous.is_empty()).then_some(previous),
            target,
            next_line: (!next.is_empty()).then_some(next),
        },
        payload,
    }
}
