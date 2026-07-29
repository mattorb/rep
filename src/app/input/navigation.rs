use super::*;
use crate::review::command::ReviewCommand;

impl App {
    fn apply_review_command(&mut self, command: ReviewCommand) {
        if let Some(status) = self.review.apply(&self.view, command).status {
            self.status = status;
        }
    }

    pub(super) fn run_search(&mut self, query: &str, forward: bool) {
        self.apply_review_command(ReviewCommand::Search {
            query: query.to_string(),
            forward,
        });
    }

    pub(super) fn jump_search(&mut self, forward: bool) {
        self.apply_review_command(ReviewCommand::JumpSearch { forward });
    }

    pub(in crate::app) fn move_node(&mut self, delta: isize) {
        self.apply_review_command(ReviewCommand::MoveNode { delta });
    }

    pub(super) fn move_active_unit(&mut self, forward: bool) {
        self.apply_review_command(ReviewCommand::MoveActiveUnit { forward });
    }

    pub(super) fn mode_cycle(&mut self, forward: bool) {
        self.apply_review_command(ReviewCommand::CycleUnit { forward });
    }

    pub(super) fn mode_adjust(&mut self, finer: bool) {
        self.apply_review_command(ReviewCommand::AdjustUnit { finer });
    }

    pub(in crate::app) const fn mode_indicator(&self) -> &'static str {
        self.review.mode_indicator()
    }

    pub(super) fn jump_to_annotation(&mut self, forward: bool) {
        self.apply_review_command(ReviewCommand::JumpAnnotation { forward });
    }
}
