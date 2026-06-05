use super::*;

impl App {
    fn target_text_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<String> {
        let node = self.index.nodes.get(node_idx)?;
        match unit {
            SelectionUnit::Sentence => {
                let r = node.sentence_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.trim().to_string())
            }
            SelectionUnit::Word => {
                let r = node.word_ranges.get(unit_idx)?;
                Some(node.selection_plain_text.get(r.clone())?.to_string())
            }
            SelectionUnit::Line => {
                if let DocNode::ListItem { .. } = self.doc.nodes.get(node_idx)? {
                    Some(node.selection_plain_text.clone())
                } else {
                    let (line, _) = node.source_line_ranges.get(unit_idx)?.clone();
                    Some(self.source_lines.get(line)?.clone())
                }
            }
            SelectionUnit::Paragraph => Some(node.selection_plain_text.replace('\n', " ")),
            SelectionUnit::Section => {
                let section = self
                    .index
                    .sections
                    .iter()
                    .find(|s| s.start_node_idx == node_idx)?;
                let mut parts: Vec<String> = Vec::new();
                for i in section.start_node_idx..=section.end_node_idx {
                    if let Some(n) = self.index.nodes.get(i)
                        && !n.selection_plain_text.is_empty()
                    {
                        parts.push(n.selection_plain_text.replace('\n', " "));
                    }
                }
                Some(parts.join(" "))
            }
        }
    }
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
            let (source_line, line_text) = self.node_line_context(node_idx);

            let previous_line = source_line
                .checked_sub(1)
                .and_then(|i| self.source_lines.get(i))
                .cloned();
            let next_line = self.source_lines.get(source_line + 1).cloned();

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
                line_number: source_line + 1,
                line_text: line_text.clone(),
                context: LineContext {
                    previous_line,
                    current_line: line_text,
                    next_line,
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
            let (source_line, line_text) = self.node_line_context(node_idx);
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
                        .target_text_for_unit(node_idx, unit, unit_idx)
                        .unwrap_or_else(|| line_clean.clone());
                    let target = clean_context(&raw_target, EMIT_TARGET_MAX_CHARS);
                    let strike_line =
                        self.where_for_annotation(unit, node_idx, Some(unit_idx), source_line);
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
            self.where_for_annotation(target_unit, node_idx, sentence_index, node_first_line);
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

    fn node_line_context(&self, node_idx: usize) -> (usize, String) {
        let source_line = self
            .doc
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(source_line)
            .cloned()
            .unwrap_or_default();
        (source_line, line_text)
    }

    /// Returns the source line where an annotation's selection text
    /// begins: per-line for Line annotations, per-word for Word,
    /// per-sentence for Sentence (computed from the rendered_nodes
    /// display plain text `\n` count), and the node's first line for
    /// Paragraph / Section (those emit their entire span anyway).
    pub(super) fn where_for_annotation(
        &self,
        target_unit: SelectionUnit,
        node_idx: usize,
        sentence_index: Option<usize>,
        node_first_line: usize,
    ) -> usize {
        match target_unit {
            SelectionUnit::Line => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.index
                    .nodes
                    .get(node_idx)
                    .and_then(|n| n.source_line_ranges.get(unit_idx).map(|p| p.0))
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Word => {
                // Word emits at the source line where the word's bytes begin.
                let unit_idx = sentence_index.unwrap_or(0);
                self.word_source_line(node_idx, unit_idx)
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Sentence => {
                // Sentence inside a multi-source-line node: emit the line
                // where the sentence's text begins, not the node's first
                // line. Computed by counting `\n` characters in the
                // rendered display plain text up to the sentence range's
                // start (mirrors the strike-emit logic above).
                sentence_index
                    .and_then(|si| {
                        let rn = self.rendered_nodes.get(node_idx)?;
                        let r = rn.sentence_ranges.get(si)?;
                        Some(node_first_line + newlines_before_byte(&rn.plain, r.start))
                    })
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Paragraph | SelectionUnit::Section => node_first_line,
        }
    }

    /// Source line where a word's bytes begin within its node's selection
    /// plain text. Maps via the index's `source_line_ranges` table.
    fn word_source_line(&self, node_idx: usize, word_idx: usize) -> Option<usize> {
        let index_node = self.index.nodes.get(node_idx)?;
        let word_range = index_node.word_ranges.get(word_idx)?;
        let word_text = index_node.selection_plain_text.get(word_range.clone())?;
        let first_line = index_node.source_line_ranges.first().map_or_else(
            || {
                self.doc
                    .nodes
                    .get(node_idx)
                    .map_or(0, |n| n.source_start_line())
            },
            |(l, _)| *l,
        );
        // Find the same occurrence of the word in the rendered display
        // plain text — repeated words must map to the right occurrence,
        // not just the first match. Count occurrences in selection plain
        // text up to word_range.start, then locate the Nth occurrence in
        // display.
        let rn = self.rendered_nodes.get(node_idx)?;
        let occurrence = count_occurrences_before(
            &index_node.selection_plain_text,
            word_text,
            word_range.start,
        );
        let pos = nth_occurrence(&rn.plain, word_text, occurrence).unwrap_or(0);
        Some(first_line + newlines_before_byte(&rn.plain, pos))
    }

    fn context_lines(&self, source_line: usize) -> (String, String) {
        let prev = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .map_or("", String::as_str);
        let next = self
            .source_lines
            .get(source_line + 1)
            .map_or("", String::as_str);
        (
            clean_context(prev, EMIT_CONTEXT_MAX_CHARS),
            clean_context(next, EMIT_CONTEXT_MAX_CHARS),
        )
    }
}
