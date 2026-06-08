use super::*;

mod mouse;
mod navigation;
mod normal;
mod text;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match self.input_mode.clone() {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Change => self.handle_change_key(key),
            InputMode::Feedback => self.handle_feedback_key(key),
            InputMode::InsertBefore => self.handle_insert_key(key, true),
            InputMode::InsertAfter => self.handle_insert_key(key, false),
            InputMode::Search => self.handle_search_key(key),
            InputMode::EditChange(node_idx, change_idx) => {
                self.handle_edit_change_key(key, node_idx, change_idx);
            }
            InputMode::EditFeedback(node_idx, feedback_idx) => {
                self.handle_edit_feedback_key(key, node_idx, feedback_idx);
            }
        }
    }
}
