use super::*;

impl App {
    // ── Output ────────────────────────────────────────────────────────────────

    pub fn to_output(&self) -> EmitModel {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());

        let mut annotations = Vec::new();
        let mut actions = Vec::new();
        for node_idx in touched {
            let line_context = self.view.node_line_context(node_idx);

            let changes = self
                .changes
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| {
                    let context = self.view.annotation_action_context(
                        node_idx,
                        a.target_unit,
                        a.sentence_index,
                        a.sentence_text.as_deref(),
                    );
                    actions.push(self.annotation_action(context, "change", "CHANGE", &a.change));
                    EmitChange {
                        created_at: a.created_at,
                        target_unit: a.target_unit.mode_str().to_string(),
                        sentence_index: a.sentence_index.map(|i| i + 1),
                        sentence_text: a.sentence_text,
                        change: a.change,
                    }
                })
                .collect();

            let feedbacks = self
                .feedbacks
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| {
                    let context = self.view.annotation_action_context(
                        node_idx,
                        a.target_unit,
                        a.sentence_index,
                        a.sentence_text.as_deref(),
                    );
                    actions.push(self.annotation_action(
                        context,
                        "revise-to-incorporate-feedback",
                        "FEEDBACK",
                        &a.feedback,
                    ));
                    EmitFeedback {
                        created_at: a.created_at,
                        target_unit: a.target_unit.mode_str().to_string(),
                        sentence_index: a.sentence_index.map(|i| i + 1),
                        sentence_text: a.sentence_text,
                        feedback: a.feedback,
                    }
                })
                .collect();

            let mut map_inserts = |action: &str,
                                   bucket: Option<&Vec<InsertAnnotation>>|
             -> Vec<EmitInsert> {
                bucket
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| {
                        let context = self.view.annotation_action_context(
                            node_idx,
                            a.target_unit,
                            a.sentence_index,
                            a.sentence_text.as_deref(),
                        );
                        actions.push(self.annotation_action(context, action, "INSERT", &a.text));
                        EmitInsert {
                            created_at: a.created_at,
                            target_unit: a.target_unit.mode_str().to_string(),
                            sentence_index: a.sentence_index.map(|i| i + 1),
                            sentence_text: a.sentence_text,
                            text: a.text,
                        }
                    })
                    .collect()
            };
            let inserts_before = map_inserts("insert-before", self.inserts_before.get(&node_idx));
            let inserts_after = map_inserts("insert-after", self.inserts_after.get(&node_idx));

            let reactions = self
                .strikes
                .get(&node_idx)
                .map(|set| {
                    set.iter()
                        .map(|&(unit, idx)| {
                            let (target_text, context) =
                                self.view.strike_action_context(node_idx, unit, idx);
                            actions.push(self.strike_action(context));
                            EmitReaction {
                                kind: "strike".to_string(),
                                target_unit: unit.mode_str().to_string(),
                                unit_index: idx + 1,
                                target_text,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            annotations.push(EmitLineAnnotation {
                line_number: line_context.source_line + 1,
                line_text: line_context.line_text.clone(),
                context: EmitLineContext {
                    previous_line: line_context.previous_line,
                    current_line: line_context.line_text,
                    next_line: line_context.next_line,
                },
                changes,
                feedbacks,
                inserts_before,
                inserts_after,
                reactions,
            });
        }

        EmitModel {
            source_file: self.source_path.display().to_string(),
            generated_at: Utc::now().to_rfc3339(),
            keymap: EmitKeymap::rep_defaults(),
            annotations,
            actions,
        }
    }

    pub fn to_human_output(&self) -> String {
        render_human_output(&self.to_output())
    }

    fn annotation_action(
        &self,
        context: SourceActionContext,
        action: &str,
        payload_key: &str,
        payload_text: &str,
    ) -> EmitAction {
        self.action_model(
            action,
            context,
            Some(EmitPayload {
                key: payload_key.to_string(),
                text: clean_context(payload_text, EMIT_PAYLOAD_MAX_CHARS),
            }),
        )
    }

    fn strike_action(&self, context: SourceActionContext) -> EmitAction {
        self.action_model("delete this", context, None)
    }

    fn action_model(
        &self,
        action: &str,
        context: SourceActionContext,
        payload: Option<EmitPayload>,
    ) -> EmitAction {
        let prev_clean_line = clean_context(&context.previous_line, EMIT_CONTEXT_MAX_CHARS);
        let target = clean_context(&context.target, EMIT_TARGET_MAX_CHARS);
        let next_clean_line = clean_context(&context.next_line, EMIT_CONTEXT_MAX_CHARS);
        EmitAction {
            action: action.to_string(),
            where_line: context.where_line + 1,
            context: EmitActionContext {
                previous_line: (!prev_clean_line.is_empty()).then_some(prev_clean_line),
                target,
                next_line: (!next_clean_line.is_empty()).then_some(next_clean_line),
            },
            payload,
        }
    }
}
