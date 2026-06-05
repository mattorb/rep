use super::*;

impl App {
    // ── Output ────────────────────────────────────────────────────────────────

    pub fn to_output(&self) -> AgentOutput {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());

        let mut annotations = Vec::new();
        for node_idx in touched {
            let line_context = self.view.node_line_context(node_idx);

            let changes = self
                .changes
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| ChangeOutput {
                    created_at: a.created_at,
                    sentence_index: a.sentence_index.map(|i| i + 1),
                    sentence_text: a.sentence_text,
                    change: a.change,
                })
                .collect();

            let feedbacks = self
                .feedbacks
                .get(&node_idx)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|a| FeedbackOutput {
                    created_at: a.created_at,
                    sentence_index: a.sentence_index.map(|i| i + 1),
                    sentence_text: a.sentence_text,
                    feedback: a.feedback,
                })
                .collect();

            let map_inserts = |bucket: Option<&Vec<InsertAnnotation>>| -> Vec<InsertOutput> {
                bucket
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| InsertOutput {
                        created_at: a.created_at,
                        sentence_index: a.sentence_index.map(|i| i + 1),
                        sentence_text: a.sentence_text,
                        text: a.text,
                    })
                    .collect()
            };
            let inserts_before = map_inserts(self.inserts_before.get(&node_idx));
            let inserts_after = map_inserts(self.inserts_after.get(&node_idx));

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
                            ReactionOutput {
                                kind: "strike".to_string(),
                                target_unit: unit.mode_str().to_string(),
                                unit_index: idx + 1,
                                target_text,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            annotations.push(LineAnnotationOutput {
                line_number: line_context.source_line + 1,
                line_text: line_context.line_text.clone(),
                context: LineContext {
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

        AgentOutput {
            source_file: self.source_path.display().to_string(),
            generated_at: Utc::now().to_rfc3339(),
            keymap: KeymapOutput {
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
            },
            annotations,
        }
    }

    pub fn to_human_output(&self) -> String {
        let mut touched = BTreeSet::new();
        touched.extend(self.changes.keys().copied());
        touched.extend(self.feedbacks.keys().copied());
        touched.extend(self.inserts_before.keys().copied());
        touched.extend(self.inserts_after.keys().copied());
        touched.extend(self.strikes.keys().copied());

        let mut out = String::new();
        let _ = writeln!(out, "FILE: {}", self.source_path.display());

        if touched.is_empty() {
            out.push_str("\nNo actions.\n");
            return out;
        }

        for node_idx in touched {
            let line_context = self.view.node_line_context(node_idx);
            let source_line = line_context.source_line;
            let line_text = line_context.line_text;
            let line_clean = clean_context(&line_text, EMIT_TARGET_MAX_CHARS);

            if let Some(changes) = self.changes.get(&node_idx) {
                for change in changes {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        "change",
                        "CHANGE",
                        change.target_unit,
                        change.sentence_index,
                        change.sentence_text.as_deref(),
                        &change.change,
                    );
                }
            }

            if let Some(feedbacks) = self.feedbacks.get(&node_idx) {
                for feedback in feedbacks {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        "revise-to-incorporate-feedback",
                        "FEEDBACK",
                        feedback.target_unit,
                        feedback.sentence_index,
                        feedback.sentence_text.as_deref(),
                        &feedback.feedback,
                    );
                }
            }

            for (action, bucket) in [
                ("insert-before", self.inserts_before.get(&node_idx)),
                ("insert-after", self.inserts_after.get(&node_idx)),
            ] {
                let Some(inserts) = bucket else { continue };
                for insert in inserts {
                    self.emit_annotation_block(
                        &mut out,
                        node_idx,
                        source_line,
                        &line_clean,
                        action,
                        "INSERT",
                        insert.target_unit,
                        insert.sentence_index,
                        insert.sentence_text.as_deref(),
                        &insert.text,
                    );
                }
            }

            if let Some(strikes) = self.strikes.get(&node_idx) {
                for &(unit, unit_idx) in strikes {
                    // Target text comes from the selection plain text
                    // view per Req 11 — same source the index uses for
                    // navigation / sentence emit.
                    let raw_target = self
                        .view
                        .target_text_for_unit(node_idx, unit, unit_idx)
                        .unwrap_or_else(|| line_clean.clone());
                    let target = clean_context(&raw_target, EMIT_TARGET_MAX_CHARS);
                    let strike_line =
                        self.view
                            .where_for_annotation(unit, node_idx, Some(unit_idx), source_line);
                    Self::emit_action_header(&mut out, "delete this", strike_line);
                    self.emit_context_block(&mut out, strike_line, &target);
                }
            }
        }

        out
    }

    /// Append a single annotation block (ACTION / WHERE / CONTEXT /
    /// payload) to `out`. Centralizes the change / feedback / insert-* /
    /// emit shape so the block grows in one place. Strikes follow a
    /// different shape (sentence-keyed, no target_unit) so they stay
    /// inline.
    #[allow(clippy::too_many_arguments)]
    fn emit_annotation_block(
        &self,
        out: &mut String,
        node_idx: usize,
        node_first_line: usize,
        line_clean: &str,
        action: &str,
        payload_key: &str,
        target_unit: SelectionUnit,
        sentence_index: Option<usize>,
        sentence_text: Option<&str>,
        payload_text: &str,
    ) {
        let where_line =
            self.view
                .where_for_annotation(target_unit, node_idx, sentence_index, node_first_line);
        let target = sentence_text.map_or_else(
            || line_clean.to_owned(),
            |s| clean_context(s, EMIT_TARGET_MAX_CHARS),
        );
        Self::emit_action_header(out, action, where_line);
        self.emit_context_block(out, where_line, &target);
        let _ = writeln!(
            out,
            "{payload_key}: \"{}\"",
            clean_context(payload_text, EMIT_PAYLOAD_MAX_CHARS)
        );
    }

    /// Write `\nACTION: <name>\nWHERE: line N\n` — shared by every
    /// emit shape (changes / feedbacks / inserts / strikes).
    fn emit_action_header(out: &mut String, action: &str, where_line: usize) {
        out.push('\n');
        let _ = writeln!(out, "ACTION: {action}");
        let _ = writeln!(out, "WHERE: line {}", where_line + 1);
    }

    /// Write the CONTEXT block: `CONTEXT:\n  prev: "..." (if any)\n  target: "..."\n  next: "..." (if any)\n`.
    /// Called by every emit shape that writes a CONTEXT section.
    fn emit_context_block(&self, out: &mut String, where_line: usize, target: &str) {
        let (prev_clean_line, next_clean_line) = self.context_lines(where_line);
        out.push_str("CONTEXT:\n");
        if !prev_clean_line.is_empty() {
            let _ = writeln!(out, "  prev: \"{prev_clean_line}\"");
        }
        let _ = writeln!(out, "  target: \"{target}\"");
        if !next_clean_line.is_empty() {
            let _ = writeln!(out, "  next: \"{next_clean_line}\"");
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
