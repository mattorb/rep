use super::*;

impl DocumentView {
    fn source_line_for_anchor(&self, anchor: SelectionAnchor) -> usize {
        let node_first_line = self
            .document
            .nodes
            .get(anchor.node_idx)
            .map_or(0, |n| n.source_start_line());
        self.where_for_annotation(
            anchor.unit,
            anchor.node_idx,
            Some(anchor.unit_idx),
            node_first_line,
        )
    }

    fn sentence_context(&self, anchor: SelectionAnchor) -> Option<(usize, String)> {
        let unit_idx = anchor.unit_idx;
        let node = self.selection_index.nodes.get(anchor.node_idx)?;
        let range = node.sentence_ranges.get(unit_idx)?;
        let text = node
            .selection_plain_text
            .get(range.clone())?
            .trim()
            .to_string();
        Some((unit_idx, text))
    }

    pub(crate) fn sentence_index_for_anchor(&self, anchor: SelectionAnchor) -> Option<usize> {
        if anchor.unit != SelectionUnit::Sentence {
            return None;
        }
        self.selection_index
            .nodes
            .get(anchor.node_idx)?
            .sentence_ranges
            .get(anchor.unit_idx)?;
        Some(anchor.unit_idx)
    }

    pub(crate) fn target_capture(&self, anchor: SelectionAnchor) -> Option<(usize, String)> {
        match anchor.unit {
            SelectionUnit::Line => self.line_capture(anchor.node_idx, anchor.unit_idx),
            SelectionUnit::Word => self.word_capture(anchor.node_idx, anchor.unit_idx),
            SelectionUnit::Paragraph => self.paragraph_capture(anchor.node_idx),
            SelectionUnit::Section => self.section_capture(anchor.node_idx),
            SelectionUnit::Sentence => self.sentence_context(anchor),
        }
    }

    pub(crate) fn annotation_target_capture(
        &self,
        anchor: SelectionAnchor,
    ) -> AnnotationTargetCapture {
        let target = self.target_capture(anchor);
        AnnotationTargetCapture {
            sentence_index: target.as_ref().map(|(idx, _)| *idx),
            sentence_text: target.map(|(_, text)| text),
            source_line: self.source_line_for_anchor(anchor),
        }
    }

    pub(crate) fn annotation_action_context(
        &self,
        node_idx: usize,
        target_unit: SelectionUnit,
        unit_idx: Option<usize>,
        target_text: Option<&str>,
    ) -> SourceActionContext {
        self.action_context_for(node_idx, target_unit, unit_idx, target_text)
    }

    pub(crate) fn strike_action_context(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> (String, SourceActionContext) {
        let target_text = self
            .target_text_for_unit(node_idx, unit, unit_idx)
            .unwrap_or_default();
        let context_target = (!target_text.is_empty()).then_some(target_text.as_str());
        let context = self.action_context_for(node_idx, unit, Some(unit_idx), context_target);
        (target_text, context)
    }

    fn target_text_for_unit(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        unit_idx: usize,
    ) -> Option<String> {
        let node = self.selection_index.nodes.get(node_idx)?;
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
                if let DocNode::ListItem { .. } = self.document.nodes.get(node_idx)? {
                    Some(node.selection_plain_text.clone())
                } else {
                    let (line, _) = node.source_line_ranges.get(unit_idx)?.clone();
                    Some(self.source_lines.get(line)?.clone())
                }
            }
            SelectionUnit::Paragraph => Some(node.selection_plain_text.replace('\n', " ")),
            SelectionUnit::Section => {
                let section = self
                    .selection_index
                    .sections
                    .iter()
                    .find(|s| s.start_node_idx == node_idx)?;
                let mut parts: Vec<String> = Vec::new();
                for i in section.start_node_idx..=section.end_node_idx {
                    if let Some(n) = self.selection_index.nodes.get(i)
                        && !n.selection_plain_text.is_empty()
                    {
                        parts.push(n.selection_plain_text.replace('\n', " "));
                    }
                }
                Some(parts.join(" "))
            }
        }
    }

    pub(crate) fn node_line_context(&self, node_idx: usize) -> SourceLineContext {
        let source_line = self
            .document
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(source_line)
            .cloned()
            .unwrap_or_default();
        let previous_line = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .cloned();
        let next_line = self.source_lines.get(source_line + 1).cloned();

        SourceLineContext {
            source_line,
            line_text,
            previous_line,
            next_line,
        }
    }

    fn action_context_for(
        &self,
        node_idx: usize,
        target_unit: SelectionUnit,
        unit_idx: Option<usize>,
        target_text: Option<&str>,
    ) -> SourceActionContext {
        let node_first_line = self
            .document
            .nodes
            .get(node_idx)
            .map_or(0, |n| n.source_start_line());
        let line_text = self
            .source_lines
            .get(node_first_line)
            .map_or("", String::as_str);
        let where_line =
            self.where_for_annotation(target_unit, node_idx, unit_idx, node_first_line);
        let (previous_line, next_line) = self.neighboring_source_lines(where_line);

        SourceActionContext {
            where_line,
            target: target_text.unwrap_or(line_text).to_string(),
            previous_line: previous_line.to_string(),
            next_line: next_line.to_string(),
        }
    }

    fn where_for_annotation(
        &self,
        target_unit: SelectionUnit,
        node_idx: usize,
        sentence_index: Option<usize>,
        node_first_line: usize,
    ) -> usize {
        match target_unit {
            SelectionUnit::Line => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.selection_index
                    .nodes
                    .get(node_idx)
                    .and_then(|n| n.source_line_ranges.get(unit_idx).map(|p| p.0))
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Word => {
                let unit_idx = sentence_index.unwrap_or(0);
                self.word_source_line(node_idx, unit_idx)
                    .unwrap_or(node_first_line)
            }
            SelectionUnit::Sentence => sentence_index
                .and_then(|si| {
                    let rn = self.rendered_nodes.get(node_idx)?;
                    let r = rn.sentence_ranges.get(si)?;
                    Some(node_first_line + newlines_before_byte(&rn.plain, r.start))
                })
                .unwrap_or(node_first_line),
            SelectionUnit::Paragraph | SelectionUnit::Section => node_first_line,
        }
    }

    fn neighboring_source_lines(&self, source_line: usize) -> (&str, &str) {
        let prev = source_line
            .checked_sub(1)
            .and_then(|i| self.source_lines.get(i))
            .map_or("", String::as_str);
        let next = self
            .source_lines
            .get(source_line + 1)
            .map_or("", String::as_str);
        (prev, next)
    }
    fn paragraph_capture(&self, node_idx: usize) -> Option<(usize, String)> {
        let plain = self
            .selection_index
            .nodes
            .get(node_idx)
            .map(|n| n.selection_plain_text.clone())?;
        Some((0, plain.replace('\n', " ")))
    }

    fn section_capture(&self, node_idx: usize) -> Option<(usize, String)> {
        let section = self
            .selection_index
            .sections
            .iter()
            .find(|s| s.start_node_idx == node_idx)?;
        let mut parts: Vec<String> = Vec::new();
        for i in section.start_node_idx..=section.end_node_idx {
            if let Some(n) = self.selection_index.nodes.get(i)
                && !n.selection_plain_text.is_empty()
            {
                parts.push(n.selection_plain_text.replace('\n', " "));
            }
        }
        Some((0, parts.join(" ")))
    }

    fn word_capture(&self, node_idx: usize, unit_idx: usize) -> Option<(usize, String)> {
        let node = self.selection_index.nodes.get(node_idx)?;
        let range = node.word_ranges.get(unit_idx)?;
        let text = node.selection_plain_text.get(range.clone())?.to_string();
        Some((unit_idx, text))
    }

    fn line_capture(&self, node_idx: usize, unit_idx: usize) -> Option<(usize, String)> {
        if let DocNode::ListItem { .. } = self.document.nodes.get(node_idx)? {
            let plain = self
                .selection_index
                .nodes
                .get(node_idx)
                .map(|n| n.selection_plain_text.clone())?;
            Some((unit_idx, plain))
        } else {
            let (line, _) = self
                .selection_index
                .nodes
                .get(node_idx)?
                .source_line_ranges
                .get(unit_idx)?
                .clone();
            let line_text = self.source_lines.get(line)?.clone();
            Some((unit_idx, line_text))
        }
    }

    fn word_source_line(&self, node_idx: usize, word_idx: usize) -> Option<usize> {
        let index_node = self.selection_index.nodes.get(node_idx)?;
        let word_range = index_node.word_ranges.get(word_idx)?;
        let word_text = index_node.selection_plain_text.get(word_range.clone())?;
        let first_line = index_node.source_line_ranges.first().map_or_else(
            || {
                self.document
                    .nodes
                    .get(node_idx)
                    .map_or(0, |n| n.source_start_line())
            },
            |(l, _)| *l,
        );
        let rn = self.rendered_nodes.get(node_idx)?;
        let occurrence = count_occurrences_before(
            &index_node.selection_plain_text,
            word_text,
            word_range.start,
        );
        let pos = nth_occurrence(&rn.plain, word_text, occurrence).unwrap_or(0);
        Some(first_line + newlines_before_byte(&rn.plain, pos))
    }
}
