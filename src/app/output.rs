use super::*;

impl App {
    pub fn to_output(&self) -> EmitModel {
        self.review.emit_model(&self.view, Utc::now().to_rfc3339())
    }

    pub fn to_human_output(&self) -> String {
        render_human_output(&self.to_output())
    }
}
