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
            let source_line = line_context.source_line;
            let line_clean = clean_context(&line_context.line_text, EMIT_TARGET_MAX_CHARS);

            let changes = self
                .changes
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| {
                    actions.push(self.annotation_action(
                        node_idx,
                        source_line,
                        &line_clean,
                        "change",
                        "CHANGE",
                        a.target_unit,
                        a.sentence_index,
                        a.sentence_text.as_deref(),
                        &a.change,
                    ));
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
                    actions.push(self.annotation_action(
                        node_idx,
                        source_line,
                        &line_clean,
                        "revise-to-incorporate-feedback",
                        "FEEDBACK",
                        a.target_unit,
                        a.sentence_index,
                        a.sentence_text.as_deref(),
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

            let mut map_inserts =
                |action: &str, bucket: Option<&Vec<InsertAnnotation>>| -> Vec<EmitInsert> {
                    bucket
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|a| {
                            actions.push(self.annotation_action(
                                node_idx,
                                source_line,
                                &line_clean,
                                action,
                                "INSERT",
                                a.target_unit,
                                a.sentence_index,
                                a.sentence_text.as_deref(),
                                &a.text,
                            ));
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
                            let target_text = self
                                .view
                                .target_text_for_unit(node_idx, unit, idx)
                                .unwrap_or_default();
                            actions.push(self.strike_action(
                                node_idx,
                                source_line,
                                &line_clean,
                                unit,
                                idx,
                                &target_text,
                            ));
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

    #[allow(clippy::too_many_arguments)]
    fn annotation_action(
        &self,
        node_idx: usize,
        node_first_line: usize,
        line_clean: &str,
        action: &str,
        payload_key: &str,
        target_unit: SelectionUnit,
        sentence_index: Option<usize>,
        sentence_text: Option<&str>,
        payload_text: &str,
    ) -> EmitAction {
        let where_line =
            self.view
                .where_for_annotation(target_unit, node_idx, sentence_index, node_first_line);
        let target = sentence_text.map_or_else(
            || line_clean.to_owned(),
            |s| clean_context(s, EMIT_TARGET_MAX_CHARS),
        );
        self.action_model(
            action,
            where_line,
            target,
            Some(EmitPayload {
                key: payload_key.to_string(),
                text: clean_context(payload_text, EMIT_PAYLOAD_MAX_CHARS),
            }),
        )
    }

    fn strike_action(
        &self,
        node_idx: usize,
        node_first_line: usize,
        line_clean: &str,
        unit: SelectionUnit,
        unit_idx: usize,
        target_text: &str,
    ) -> EmitAction {
        let raw_target = if target_text.is_empty() {
            line_clean
        } else {
            target_text
        };
        let target = clean_context(raw_target, EMIT_TARGET_MAX_CHARS);
        let where_line =
            self.view
                .where_for_annotation(unit, node_idx, Some(unit_idx), node_first_line);
        self.action_model("delete this", where_line, target, None)
    }

    fn action_model(
        &self,
        action: &str,
        where_line: usize,
        target: String,
        payload: Option<EmitPayload>,
    ) -> EmitAction {
        let (prev_clean_line, next_clean_line) = self.context_lines(where_line);
        EmitAction {
            action: action.to_string(),
            where_line: where_line + 1,
            context: EmitActionContext {
                previous_line: (!prev_clean_line.is_empty()).then_some(prev_clean_line),
                target,
                next_line: (!next_clean_line.is_empty()).then_some(next_clean_line),
            },
            payload,
        }
    }

    fn context_lines(&self, source_line: usize) -> (String, String) {
        let (prev, next) = self.view.context_lines(source_line);
        (
            clean_context(prev, EMIT_CONTEXT_MAX_CHARS),
            clean_context(next, EMIT_CONTEXT_MAX_CHARS),
        )
    }
}
