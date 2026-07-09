use super::*;

mod mouse;
mod navigation;
mod normal;
mod text;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        let input_mode = self.input_mode.clone();
        match input_mode.clone() {
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
        if should_capture_key_cue(&input_mode, key) {
            self.capture_key_cue(key);
        } else {
            self.key_hud = None;
        }
    }
}

fn should_capture_key_cue(input_mode: &InputMode, key: KeyEvent) -> bool {
    match input_mode {
        InputMode::Normal => true,
        InputMode::Change
        | InputMode::Feedback
        | InputMode::InsertBefore
        | InputMode::InsertAfter
        | InputMode::Search
        | InputMode::EditChange(..)
        | InputMode::EditFeedback(..) => matches!(key.code, KeyCode::Enter | KeyCode::Esc),
    }
}
