//! Owned exact lexer state captured before a foreground positional read begins.
use crate::{styles::RoleHint, syntax::State};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Snapshot {
    pub offset: u64,
    pub state: State,
    pub line_start: bool,
    /// Resolved roles use decoded UTF-8 coordinates relative to this snapshot.
    pub hints: [Option<RoleHint>; 2],
}

impl Snapshot {
    pub(crate) fn new(offset: u64, state: State, line_start: bool) -> Self {
        Self {
            offset,
            state,
            line_start,
            hints: [None; 2],
        }
    }
}
