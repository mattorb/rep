use super::*;

impl App {
    pub(super) fn handle_normal_key(&mut self, key: KeyEvent) {
        self.notification = None;
        self.nav_feedback = None;

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.quit_confirm_pending {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('q') | KeyCode::Enter => {
                    self.should_quit = true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.quit_confirm_pending = false;
                    self.status = "Quit cancelled.".to_string();
                }
                _ => {
                    // Other keys are ignored so a stray keystroke doesn't
                    // accidentally answer the prompt.
                }
            }
            return;
        }

        if let Some(scroll) = self.ast_view_scroll {
            match key.code {
                KeyCode::Esc | KeyCode::Char('I') => {
                    self.ast_view_scroll = None;
                    self.status = "Closed AST view.".to_string();
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.ast_view_scroll = Some(scroll.saturating_add(3));
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.ast_view_scroll = Some(scroll.saturating_sub(3));
                }
                _ => {}
            }
            return;
        }

        if self.link_popup_urls.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('O') => {
                    self.link_popup_urls = None;
                    self.status = "Closed link popup.".to_string();
                }
                _ => {
                    self.link_popup_urls = None;
                }
            }
            return;
        }

        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.show_help = false;
                    self.status = "Closed help.".to_string();
                }
                _ => {
                    self.show_help = false;
                }
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => {
                self.quit_confirm_pending = true;
            }
            KeyCode::Char('Q') => {
                self.silent_quit = true;
                self.should_quit = true;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.show_help = true;
                self.status = "Help open. Press ? or Esc to close.".to_string();
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.search_buffer.clear();
                self.status = "Search: type pattern and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('n') => self.jump_search(true),
            KeyCode::Char('N') => self.jump_search(false),
            KeyCode::Char('c') => self.begin_change_or_edit(),
            KeyCode::Char('f') => self.begin_feedback_or_edit(),
            KeyCode::Char('b') => {
                self.input_mode = InputMode::InsertBefore;
                self.insert_buffer.clear();
                self.status = "Insert before: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('a') => {
                self.input_mode = InputMode::InsertAfter;
                self.insert_buffer.clear();
                self.status = "Insert after: type text and press Enter. Esc cancels.".to_string();
            }
            KeyCode::Char('e') => self.begin_edit_annotation(),
            KeyCode::Char(' ') => self.mode_cycle(true),
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Right => self.move_active_unit(true),
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Left => self.move_active_unit(false),
            KeyCode::Backspace => self.mode_cycle(false),
            // i / o adjust the active selection unit by one step
            // without wrapping. i = "in" / finer; o = "out" / coarser.
            KeyCode::Char('i') => self.mode_adjust(true),
            KeyCode::Char('o') => self.mode_adjust(false),
            // Capital variants are the popups: I opens the AST popup,
            // O reveals links from the current sentence.
            KeyCode::Char('I') => {
                self.ast_view_scroll = Some(0);
                self.status = "AST view. j/k scroll, I or Esc close.".to_string();
            }
            KeyCode::Char('O') if !self.reveal_links_for_current_sentence() => {
                self.status = "No markdown links in current sentence.".to_string();
            }
            KeyCode::Char('x') => self.toggle_strike(),
            KeyCode::Char('r') => {
                let output = self.to_human_output();
                self.notification = Some(match copy_to_clipboard(&output) {
                    ClipboardOutcome::OsCommand => "Copied to clipboard".to_string(),
                    ClipboardOutcome::Osc52 => "Sent via OSC 52".to_string(),
                    ClipboardOutcome::Failed => "Copy failed — no clipboard available".to_string(),
                });
            }
            KeyCode::Char('[') => self.jump_to_annotation(false),
            KeyCode::Char(']') => self.jump_to_annotation(true),
            _ => {}
        }
    }
}
