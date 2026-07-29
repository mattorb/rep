/// UI-neutral commands whose semantics are shared by the terminal and browser
/// frontends. Frontends translate key or DOM events into these commands and
/// render the returned status text in their own chrome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCommand {
    MoveNode { delta: isize },
    MoveActiveUnit { forward: bool },
    CycleUnit { forward: bool },
    AdjustUnit { finer: bool },
    Search { query: String, forward: bool },
    JumpSearch { forward: bool },
    JumpAnnotation { forward: bool },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CommandResult {
    pub(crate) status: Option<String>,
}

impl CommandResult {
    pub(crate) fn with_status(status: impl Into<String>) -> Self {
        Self {
            status: Some(status.into()),
        }
    }
}
