use super::*;

impl DocumentView {
    pub(crate) fn links_for_anchor(&self, anchor: SelectionAnchor) -> Vec<String> {
        let Some(rn) = self.rendered_nodes.get(anchor.node_idx) else {
            return Vec::new();
        };
        let scope: Option<Range<usize>> = if anchor.unit == SelectionUnit::Sentence {
            rn.sentence_ranges.get(anchor.unit_idx).cloned()
        } else {
            None
        };
        let mut urls = Vec::new();
        for link in &rn.links {
            let overlaps = scope
                .as_ref()
                .is_none_or(|r| link.end > r.start && link.start < r.end);
            if overlaps && !urls.iter().any(|u: &String| u == &link.url) {
                urls.push(link.url.clone());
            }
        }
        urls
    }
}
