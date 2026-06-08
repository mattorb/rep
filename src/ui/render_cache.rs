use std::ops::Range;

#[derive(Debug, Default)]
pub(crate) struct RenderCache {
    pub(crate) node_heights: Vec<u16>,
    pub(crate) visible_rows: Vec<(usize, Range<usize>)>,
}

impl RenderCache {
    pub(crate) fn replace_document_rows(
        &mut self,
        node_heights: Vec<u16>,
        visible_rows: Vec<(usize, Range<usize>)>,
    ) {
        self.node_heights = node_heights;
        self.visible_rows = visible_rows;
    }
}
