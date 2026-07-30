use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::review::document::{
    ActionContext, CapturedTarget, DocumentFormat, NodeSourceContext, OutlineRow, ReviewDocument,
    ReviewLink, SourceLocator,
};
use crate::selection::index::{SelectionIndex, SelectionNodeInput, byte_range_to_scalar_range};
use crate::selection::model::{NavOutcome, SelectionAnchor, SelectionUnit};
use crate::selection::segment::{segment_sentences, segment_words};

pub(crate) const MAX_MANIFEST_NODES: usize = 100_000;
pub(crate) const MAX_NODE_TEXT_BYTES: usize = 1024 * 1024;
const MAX_SELECTOR_BYTES: usize = 16 * 1024;
const MAX_LINKS_PER_NODE: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HtmlManifest {
    pub(crate) version: u8,
    pub(crate) nodes: Vec<HtmlManifestNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HtmlManifestNode {
    pub(crate) source_id: u64,
    pub(crate) source_line: usize,
    pub(crate) tag: String,
    pub(crate) element_summary: String,
    pub(crate) text: String,
    pub(crate) logical_lines: Vec<ScalarRange>,
    pub(crate) selector: String,
    pub(crate) text_fragment: Option<usize>,
    pub(crate) heading_level: Option<u8>,
    pub(crate) list_id: Option<usize>,
    pub(crate) top_level_ordered_list_item: bool,
    pub(crate) links: Vec<ManifestLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScalarRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManifestLink {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) url: String,
}

#[derive(Debug)]
pub(crate) struct HtmlReviewDocument {
    source_path: PathBuf,
    manifest: HtmlManifest,
    selection_index: SelectionIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectionSlice {
    pub(crate) node: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl HtmlReviewDocument {
    pub(crate) fn from_manifest(source_path: PathBuf, manifest: HtmlManifest) -> Result<Self> {
        validate_manifest(&manifest)?;
        let inputs = manifest
            .nodes
            .iter()
            .map(|node| {
                let scalar_count = node.text.chars().count();
                let source_line_ranges = node
                    .logical_lines
                    .iter()
                    .map(|range| {
                        scalar_range_to_byte_range(&node.text, range)
                            .map(|bytes| (node.source_line - 1, bytes))
                    })
                    .collect::<Option<Vec<_>>>()
                    .expect("validated scalar line ranges");
                debug_assert_eq!(
                    scalar_count,
                    node.text.chars().count(),
                    "manifest text must remain immutable during construction"
                );
                SelectionNodeInput {
                    plain_text: node.text.clone(),
                    source_line_ranges,
                    sentence_ranges: segment_sentences(&node.text),
                    word_ranges: segment_words(&node.text),
                    heading_level: node.heading_level,
                    list_id: node.list_id,
                    is_top_level_ordered_list_item: node.top_level_ordered_list_item,
                    has_content: !node.text.trim().is_empty(),
                }
            })
            .collect();
        let selection_index = SelectionIndex::from_nodes(inputs);
        Ok(Self {
            source_path,
            manifest,
            selection_index,
        })
    }

    pub(crate) const fn manifest(&self) -> &HtmlManifest {
        &self.manifest
    }

    pub(crate) fn selection_slices(&self, anchor: SelectionAnchor) -> Vec<SelectionSlice> {
        if anchor.unit == SelectionUnit::Section {
            return self
                .section_span_for_start(anchor.node_idx)
                .filter_map(|node| {
                    let text = self.selection_index.nodes.get(node)?;
                    Some(SelectionSlice {
                        node,
                        start: 0,
                        end: text.selection_plain_text.chars().count(),
                    })
                })
                .collect();
        }
        self.anchor_byte_range(anchor)
            .and_then(|range| {
                let text = &self
                    .selection_index
                    .nodes
                    .get(anchor.node_idx)?
                    .selection_plain_text;
                byte_range_to_scalar_range(text, range)
            })
            .map(|range| {
                vec![SelectionSlice {
                    node: anchor.node_idx,
                    start: range.start,
                    end: range.end,
                }]
            })
            .unwrap_or_default()
    }

    pub(crate) fn anchor_at_scalar(
        &self,
        node_idx: usize,
        unit: SelectionUnit,
        scalar_offset: usize,
    ) -> Option<SelectionAnchor> {
        let node = self.selection_index.nodes.get(node_idx)?;
        let scalar_count = node.selection_plain_text.chars().count();
        if scalar_offset > scalar_count {
            return None;
        }
        let byte_offset = scalar_to_byte_offset(&node.selection_plain_text, scalar_offset)?;
        let index = match unit {
            SelectionUnit::Paragraph => self
                .selection_index
                .paragraphs
                .iter()
                .any(|&(node, _)| node == node_idx)
                .then_some(0),
            SelectionUnit::Line => closest_range(
                node.source_line_ranges
                    .iter()
                    .map(|(_, range)| range.clone()),
                byte_offset,
            ),
            SelectionUnit::Sentence => {
                closest_range(node.sentence_ranges.iter().cloned(), byte_offset)
            }
            SelectionUnit::Word => closest_range(node.word_ranges.iter().cloned(), byte_offset),
            SelectionUnit::Section => self
                .selection_index
                .sections
                .iter()
                .find(|section| {
                    section.start_node_idx <= node_idx && node_idx <= section.end_node_idx
                })
                .map(|_| 0),
        }?;
        let anchor = SelectionAnchor::new(node_idx, unit, index);
        Some(if unit == SelectionUnit::Section {
            self.clamp_anchor(anchor, SelectionUnit::Section)
        } else {
            anchor
        })
    }

    fn anchor_byte_range(&self, anchor: SelectionAnchor) -> Option<Range<usize>> {
        let node = self.selection_index.nodes.get(anchor.node_idx)?;
        match anchor.unit {
            SelectionUnit::Paragraph => Some(0..node.selection_plain_text.len()),
            SelectionUnit::Line => node
                .source_line_ranges
                .get(anchor.unit_idx)
                .map(|(_, range)| range.clone()),
            SelectionUnit::Sentence => node.sentence_ranges.get(anchor.unit_idx).cloned(),
            SelectionUnit::Word => node.word_ranges.get(anchor.unit_idx).cloned(),
            SelectionUnit::Section => None,
        }
    }

    fn anchor_text(&self, anchor: SelectionAnchor) -> Option<String> {
        if anchor.unit == SelectionUnit::Section {
            let parts = self
                .section_span_for_start(anchor.node_idx)
                .filter_map(|node| self.selection_index.nodes.get(node))
                .map(|node| node.selection_plain_text.replace('\n', " "))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>();
            return (!parts.is_empty()).then(|| parts.join(" "));
        }
        let node = self.selection_index.nodes.get(anchor.node_idx)?;
        let range = self.anchor_byte_range(anchor)?;
        node.selection_plain_text.get(range).map(ToOwned::to_owned)
    }

    fn locator_for_node(&self, node_idx: usize) -> SourceLocator {
        let node = &self.manifest.nodes[node_idx];
        SourceLocator::Html {
            line: node.source_line - 1,
            selector: node.selector.clone(),
            text_fragment: node.text_fragment,
        }
    }
}

impl ReviewDocument for HtmlReviewDocument {
    fn source_path(&self) -> &Path {
        &self.source_path
    }

    fn format(&self) -> DocumentFormat {
        DocumentFormat::Html
    }

    fn selection_index(&self) -> &SelectionIndex {
        &self.selection_index
    }

    fn initial_anchor(&self) -> SelectionAnchor {
        self.selection_index.sections.first().map_or_else(
            || {
                SelectionAnchor::new(
                    self.next_content_node(0).unwrap_or(0),
                    SelectionUnit::Sentence,
                    0,
                )
            },
            |section| SelectionAnchor::new(section.start_node_idx, SelectionUnit::Section, 0),
        )
    }

    fn node_count(&self) -> usize {
        self.manifest.nodes.len()
    }

    fn next_content_node(&self, from: usize) -> Option<usize> {
        (from..self.node_count()).find(|index| {
            self.selection_index
                .nodes
                .get(*index)
                .is_some_and(|node| !node.selection_plain_text.trim().is_empty())
        })
    }

    fn prev_content_node(&self, before: usize) -> Option<usize> {
        (0..before.min(self.node_count())).rev().find(|index| {
            self.selection_index
                .nodes
                .get(*index)
                .is_some_and(|node| !node.selection_plain_text.trim().is_empty())
        })
    }

    fn navigate(&self, anchor: SelectionAnchor, forward: bool) -> NavOutcome {
        if forward {
            crate::selection::navigator::next(&self.selection_index, anchor)
        } else {
            crate::selection::navigator::prev(&self.selection_index, anchor)
        }
    }

    fn clamp_anchor(&self, anchor: SelectionAnchor, target: SelectionUnit) -> SelectionAnchor {
        crate::selection::navigator::clamp(&self.selection_index, anchor, target)
    }

    fn has_any_anchor(&self, unit: SelectionUnit) -> bool {
        match unit {
            SelectionUnit::Section => !self.selection_index.sections.is_empty(),
            SelectionUnit::Paragraph => !self.selection_index.paragraphs.is_empty(),
            SelectionUnit::Line => !self.selection_index.lines.is_empty(),
            SelectionUnit::Sentence => !self.selection_index.sentences.is_empty(),
            SelectionUnit::Word => !self.selection_index.words.is_empty(),
        }
    }

    fn section_span_for_start(&self, node_idx: usize) -> Range<usize> {
        let end = self
            .selection_index
            .sections
            .iter()
            .find(|section| section.start_node_idx == node_idx)
            .map_or_else(|| self.node_count(), |section| section.end_node_idx + 1);
        node_idx..end
    }

    fn sentence_count_for_node(&self, node_idx: usize) -> usize {
        self.selection_index
            .nodes
            .get(node_idx)
            .map_or(0, |node| node.sentence_ranges.len())
    }

    fn sentence_index_for_anchor(&self, anchor: SelectionAnchor) -> Option<usize> {
        match anchor.unit {
            SelectionUnit::Sentence => Some(anchor.unit_idx),
            SelectionUnit::Word | SelectionUnit::Line => {
                let source = self.anchor_byte_range(anchor)?;
                self.selection_index
                    .nodes
                    .get(anchor.node_idx)?
                    .sentence_ranges
                    .iter()
                    .position(|range| range.start <= source.start && source.start < range.end)
            }
            SelectionUnit::Paragraph | SelectionUnit::Section => None,
        }
    }

    fn search_matches(&self, query: &str) -> Vec<(usize, usize)> {
        if query.is_empty() {
            return Vec::new();
        }
        let case_sensitive = query
            .chars()
            .any(|character| character.is_ascii_uppercase());
        let needle = if case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };
        let mut matches = Vec::new();
        for (node_idx, node) in self.selection_index.nodes.iter().enumerate() {
            let haystack = if case_sensitive {
                node.selection_plain_text.clone()
            } else {
                node.selection_plain_text.to_ascii_lowercase()
            };
            let mut cursor = 0;
            while cursor <= haystack.len() {
                let Some(relative) = haystack[cursor..].find(&needle) else {
                    break;
                };
                let absolute = cursor + relative;
                let sentence_idx = node
                    .sentence_ranges
                    .iter()
                    .position(|range| range.start <= absolute && absolute < range.end)
                    .unwrap_or(0);
                matches.push((node_idx, sentence_idx));
                cursor = absolute + needle.len();
            }
        }
        matches
    }

    fn capture_target(&self, anchor: SelectionAnchor) -> CapturedTarget {
        CapturedTarget {
            anchor,
            text: self.anchor_text(anchor),
            locator: self.locator_for_node(anchor.node_idx),
        }
    }

    fn has_target(&self, anchor: SelectionAnchor) -> bool {
        self.anchor_text(anchor).is_some()
    }

    fn action_context(&self, target: &CapturedTarget) -> ActionContext {
        let node_idx = target.anchor.node_idx;
        ActionContext {
            where_line: target.locator.source_line(),
            target: target.text.clone().unwrap_or_default(),
            previous: node_idx
                .checked_sub(1)
                .and_then(|index| self.manifest.nodes.get(index))
                .map_or_else(String::new, |node| node.text.clone()),
            next: self
                .manifest
                .nodes
                .get(node_idx + 1)
                .map_or_else(String::new, |node| node.text.clone()),
            locator: target.locator.clone(),
        }
    }

    fn node_source_context(&self, node_idx: usize) -> NodeSourceContext {
        let node = &self.manifest.nodes[node_idx];
        NodeSourceContext {
            source_line: node.source_line - 1,
            line_text: node.text.clone(),
            previous: node_idx
                .checked_sub(1)
                .and_then(|index| self.manifest.nodes.get(index))
                .map(|node| node.text.clone()),
            next: self
                .manifest
                .nodes
                .get(node_idx + 1)
                .map(|node| node.text.clone()),
        }
    }

    fn links_for(&self, anchor: SelectionAnchor) -> Vec<ReviewLink> {
        let Some(node) = self.manifest.nodes.get(anchor.node_idx) else {
            return Vec::new();
        };
        let selected = self
            .selection_slices(anchor)
            .into_iter()
            .find(|slice| slice.node == anchor.node_idx);
        node.links
            .iter()
            .filter(|link| {
                selected
                    .as_ref()
                    .is_none_or(|range| link.start < range.end && range.start < link.end)
            })
            .map(|link| ReviewLink {
                url: link.url.clone(),
            })
            .collect()
    }

    fn node_outline(&self) -> Vec<OutlineRow> {
        self.manifest
            .nodes
            .iter()
            .enumerate()
            .map(|(node_idx, node)| {
                let mut preview = node.text.chars().take(96).collect::<String>();
                if node.text.chars().count() > 96 {
                    preview.push('…');
                }
                OutlineRow {
                    node_idx,
                    level: node.heading_level.unwrap_or(1),
                    text: format!(
                        "{} · line {} · {}",
                        node.element_summary, node.source_line, preview
                    ),
                }
            })
            .collect()
    }
}

pub(crate) fn parse_manifest(body: &[u8]) -> Result<HtmlManifest> {
    let manifest: HtmlManifest = serde_json::from_slice(body)
        .map_err(|error| anyhow::anyhow!("invalid manifest: {error}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &HtmlManifest) -> Result<()> {
    if manifest.version != 1 {
        bail!("unsupported manifest version");
    }
    if manifest.nodes.len() > MAX_MANIFEST_NODES {
        bail!("manifest exceeds {MAX_MANIFEST_NODES} review nodes");
    }
    let mut source_ids = BTreeSet::new();
    for (index, node) in manifest.nodes.iter().enumerate() {
        if !source_ids.insert((node.source_id, node.text_fragment)) {
            bail!("manifest node {index} has a duplicate sourceId/textFragment pair");
        }
        if node.source_line == 0 {
            bail!("manifest node {index} sourceLine must be one-based");
        }
        if node.text.len() > MAX_NODE_TEXT_BYTES {
            bail!("manifest node {index} text exceeds 1 MiB");
        }
        if node.text.trim().is_empty() {
            bail!("manifest node {index} has no selectable text");
        }
        if node.tag.is_empty()
            || node.tag.len() > 32
            || !node
                .tag
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            bail!("manifest node {index} has an invalid tag");
        }
        if node.element_summary.is_empty()
            || node.element_summary.len() > 4096
            || node.element_summary.chars().any(char::is_control)
        {
            bail!("manifest node {index} has an invalid elementSummary");
        }
        if node.selector.is_empty() || node.selector.len() > MAX_SELECTOR_BYTES {
            bail!("manifest node {index} has an invalid selector");
        }
        if node.text_fragment == Some(0) {
            bail!("manifest node {index} textFragment must be one-based");
        }
        if node
            .heading_level
            .is_some_and(|level| !(1..=6).contains(&level))
        {
            bail!("manifest node {index} has an invalid headingLevel");
        }
        if node.top_level_ordered_list_item && node.list_id.is_none() {
            bail!("manifest node {index} top-level list item is missing listId");
        }
        let scalar_count = node.text.chars().count();
        validate_ranges(index, "logicalLines", &node.logical_lines, scalar_count)?;
        if node.logical_lines.is_empty() {
            bail!("manifest node {index} has no logicalLines");
        }
        if node.links.len() > MAX_LINKS_PER_NODE {
            bail!("manifest node {index} has too many links");
        }
        let mut link_ranges = Vec::with_capacity(node.links.len());
        for link in &node.links {
            if link.url.len() > MAX_SELECTOR_BYTES || link.url.is_empty() {
                bail!("manifest node {index} has an invalid link URL");
            }
            link_ranges.push(ScalarRange {
                start: link.start,
                end: link.end,
            });
        }
        validate_ranges(index, "links", &link_ranges, scalar_count)?;
    }
    Ok(())
}

fn validate_ranges(
    node_idx: usize,
    field: &str,
    ranges: &[ScalarRange],
    scalar_count: usize,
) -> Result<()> {
    let mut previous_end = 0;
    for range in ranges {
        if range.start > range.end || range.end > scalar_count || range.start < previous_end {
            bail!("manifest node {node_idx} has invalid {field} ranges");
        }
        previous_end = range.end;
    }
    Ok(())
}

fn scalar_range_to_byte_range(text: &str, range: &ScalarRange) -> Option<Range<usize>> {
    let start = scalar_to_byte_offset(text, range.start)?;
    let end = scalar_to_byte_offset(text, range.end)?;
    Some(start..end)
}

fn scalar_to_byte_offset(text: &str, scalar: usize) -> Option<usize> {
    if scalar == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(scalar).map(|(offset, _)| offset)
}

fn closest_range(ranges: impl Iterator<Item = Range<usize>>, offset: usize) -> Option<usize> {
    let ranges = ranges.collect::<Vec<_>>();
    ranges
        .iter()
        .position(|range| range.start <= offset && offset < range.end)
        .or_else(|| {
            ranges
                .iter()
                .enumerate()
                .min_by_key(|(_, range)| range.start.abs_diff(offset))
                .map(|(index, _)| index)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::render_human_output;
    use crate::review::session::ReviewSession;

    fn node(text: &str) -> HtmlManifestNode {
        HtmlManifestNode {
            source_id: 1,
            source_line: 3,
            tag: "p".to_string(),
            element_summary: "p#plan.review".to_string(),
            text: text.to_string(),
            logical_lines: vec![ScalarRange {
                start: 0,
                end: text.chars().count(),
            }],
            selector: "#plan".to_string(),
            text_fragment: None,
            heading_level: None,
            list_id: None,
            top_level_ordered_list_item: false,
            links: Vec::new(),
        }
    }

    fn document(nodes: Vec<HtmlManifestNode>) -> HtmlReviewDocument {
        HtmlReviewDocument::from_manifest(
            PathBuf::from("plan.html"),
            HtmlManifest { version: 1, nodes },
        )
        .unwrap()
    }

    #[test]
    fn builds_all_selection_units_and_scalar_slices() {
        let mut heading = node("Plan");
        heading.source_id = 0;
        heading.tag = "h1".to_string();
        heading.heading_level = Some(1);
        let mut paragraph = node("Ship 🚀 safely.\nThen test.");
        paragraph.logical_lines = vec![
            ScalarRange { start: 0, end: 14 },
            ScalarRange { start: 15, end: 25 },
        ];
        let document = document(vec![heading, paragraph]);

        assert_eq!(document.selection_index.sections.len(), 1);
        assert_eq!(
            document.initial_anchor(),
            SelectionAnchor::new(0, SelectionUnit::Section, 0)
        );
        assert_eq!(document.selection_index.nodes[1].sentence_ranges.len(), 2);
        let rocket = document
            .anchor_at_scalar(1, SelectionUnit::Word, 5)
            .unwrap();
        assert_eq!(document.anchor_text(rocket).as_deref(), Some("Ship"));
        let sentence = SelectionAnchor::new(1, SelectionUnit::Sentence, 0);
        assert_eq!(
            document.selection_slices(sentence),
            vec![SelectionSlice {
                node: 1,
                start: 0,
                end: 14
            }]
        );
    }

    #[test]
    fn validates_limits_ranges_and_duplicate_source_ids() {
        let mut invalid = node("text");
        invalid.logical_lines[0].end = 5;
        assert!(
            HtmlReviewDocument::from_manifest(
                PathBuf::from("plan.html"),
                HtmlManifest {
                    version: 1,
                    nodes: vec![invalid]
                }
            )
            .unwrap_err()
            .to_string()
            .contains("logicalLines")
        );

        let first = node("first");
        let mut second = node("second");
        second.source_line = 9;
        assert!(
            HtmlReviewDocument::from_manifest(
                PathBuf::from("plan.html"),
                HtmlManifest {
                    version: 1,
                    nodes: vec![first, second]
                }
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate sourceId")
        );
    }

    #[test]
    fn rejects_unknown_fields_and_oversized_node_text() {
        let unknown = br#"{"version":1,"nodes":[],"extra":true}"#;
        assert!(
            parse_manifest(unknown)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );

        let mut oversized = node(&"a".repeat(MAX_NODE_TEXT_BYTES + 1));
        oversized.logical_lines[0].end = oversized.text.len();
        assert!(
            HtmlReviewDocument::from_manifest(
                PathBuf::from("plan.html"),
                HtmlManifest {
                    version: 1,
                    nodes: vec![oversized]
                }
            )
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
        );
    }

    #[test]
    fn navigation_and_context_use_dom_order() {
        let first = node("First sentence.");
        let mut second = node("Second sentence.");
        second.source_id = 2;
        second.source_line = 8;
        second.element_summary = "p#second.important".to_string();
        second.selector = "body > p:nth-of-type(2)".to_string();
        let document = document(vec![first, second]);
        let anchor = document.initial_anchor();
        assert_eq!(
            anchor,
            SelectionAnchor::new(0, SelectionUnit::Sentence, 0),
            "prose-only HTML retains the finest available initial anchor"
        );
        let moved = document.navigate(anchor, true);
        assert_eq!(
            moved,
            NavOutcome::Moved(SelectionAnchor::new(1, SelectionUnit::Sentence, 0))
        );
        let target = document.capture_target(SelectionAnchor::new(1, SelectionUnit::Paragraph, 0));
        let context = document.action_context(&target);
        assert_eq!(context.previous, "First sentence.");
        assert_eq!(context.where_line, 7);
        let outline = document.node_outline();
        assert_eq!(outline.len(), 2);
        assert_eq!(
            outline[1].text,
            "p#second.important · line 8 · Second sentence."
        );
    }

    #[test]
    fn html_emit_golden_covers_actions_units_context_and_locators() {
        let mut heading = node("Plan");
        heading.source_id = 0;
        heading.source_line = 3;
        heading.tag = "h1".to_string();
        heading.selector = "#plan".to_string();
        heading.heading_level = Some(1);
        let mut target = node("First sentence. Second target.");
        target.source_id = 2;
        target.source_line = 6;
        target.selector = "#target".to_string();
        target.text_fragment = Some(2);
        let mut outcome = node("Outcome");
        outcome.source_id = 3;
        outcome.source_line = 9;
        outcome.selector = "#outcome".to_string();
        let document = document(vec![heading, target, outcome]);
        let mut review = ReviewSession::new(document.initial_anchor());

        review.set_anchor(
            &document,
            SelectionAnchor::new(0, SelectionUnit::Section, 0),
        );
        review.add_change(
            &document,
            "2026-01-01T00:00:01Z".to_string(),
            "Rename the plan.".to_string(),
        );
        review.set_anchor(
            &document,
            SelectionAnchor::new(1, SelectionUnit::Sentence, 1),
        );
        review.add_feedback(
            &document,
            "2026-01-01T00:00:02Z".to_string(),
            "Explain the target.".to_string(),
        );
        review.set_anchor(&document, SelectionAnchor::new(1, SelectionUnit::Word, 0));
        review.add_insert(
            &document,
            "2026-01-01T00:00:03Z".to_string(),
            "Before text.".to_string(),
            true,
        );
        review.set_anchor(&document, SelectionAnchor::new(1, SelectionUnit::Line, 0));
        review.add_insert(
            &document,
            "2026-01-01T00:00:04Z".to_string(),
            "After text.".to_string(),
            false,
        );
        review.set_anchor(
            &document,
            SelectionAnchor::new(2, SelectionUnit::Paragraph, 0),
        );
        review.toggle_strike(&document, "2026-01-01T00:00:05Z".to_string());

        let output =
            render_human_output(&review.emit_model(&document, "2026-01-01T00:01:00Z".to_string()));
        assert_eq!(
            output,
            include_str!("../../tests/fixtures/web/html-actions.golden.txt")
        );
    }

    #[test]
    fn html_emit_snapshot_covers_every_unit_and_action_pair() {
        let mut heading = node("Plan");
        heading.source_id = 0;
        heading.source_line = 3;
        heading.tag = "h1".to_string();
        heading.element_summary = "h1#plan".to_string();
        heading.selector = "#plan".to_string();
        heading.heading_level = Some(1);
        let mut target = node("First sentence. Second target.");
        target.source_id = 2;
        target.source_line = 6;
        target.element_summary = "p#target.review".to_string();
        target.selector = "#target".to_string();
        target.text_fragment = Some(2);
        let mut outcome = node("Outcome");
        outcome.source_id = 3;
        outcome.source_line = 9;
        outcome.element_summary = "h2#outcome".to_string();
        outcome.selector = "#outcome".to_string();
        outcome.tag = "h2".to_string();
        outcome.heading_level = Some(2);
        let document = document(vec![heading, target, outcome]);
        let units = [
            (
                "section",
                SelectionAnchor::new(0, SelectionUnit::Section, 0),
            ),
            (
                "paragraph",
                SelectionAnchor::new(1, SelectionUnit::Paragraph, 0),
            ),
            ("line", SelectionAnchor::new(1, SelectionUnit::Line, 0)),
            (
                "sentence",
                SelectionAnchor::new(1, SelectionUnit::Sentence, 1),
            ),
            ("word", SelectionAnchor::new(1, SelectionUnit::Word, 1)),
        ];
        let actions = [
            "change",
            "feedback",
            "insert-before",
            "insert-after",
            "delete",
        ];
        let mut matrix = String::new();

        for (unit_name, anchor) in units {
            for action in actions {
                let mut review = ReviewSession::new(document.initial_anchor());
                review.set_anchor(&document, anchor);
                match action {
                    "change" => {
                        review.add_change(
                            &document,
                            "2026-01-01T00:00:01Z".to_string(),
                            "Change payload.".to_string(),
                        );
                    }
                    "feedback" => {
                        review.add_feedback(
                            &document,
                            "2026-01-01T00:00:01Z".to_string(),
                            "Feedback payload.".to_string(),
                        );
                    }
                    "insert-before" => {
                        review.add_insert(
                            &document,
                            "2026-01-01T00:00:01Z".to_string(),
                            "Insert payload.".to_string(),
                            true,
                        );
                    }
                    "insert-after" => {
                        review.add_insert(
                            &document,
                            "2026-01-01T00:00:01Z".to_string(),
                            "Insert payload.".to_string(),
                            false,
                        );
                    }
                    "delete" => {
                        review.toggle_strike(&document, "2026-01-01T00:00:01Z".to_string());
                    }
                    _ => unreachable!(),
                }
                matrix.push_str(&format!("## {unit_name}/{action}\n"));
                matrix.push_str(&render_human_output(
                    &review.emit_model(&document, "2026-01-01T00:01:00Z".to_string()),
                ));
                matrix.push('\n');
            }
        }

        insta::assert_snapshot!("html_emit_matrix", matrix);
    }
}
