//! Saved per-panel filename filtering; compilation and matching belong to workers.
use crate::PaneId;

pub const MAX_FILTER_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterMode {
    #[default]
    Masks,
    Regex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFilter {
    pub enabled: bool,
    pub mode: FilterMode,
    pub expression: String,
    pub case_sensitive: bool,
}

impl Default for PanelFilter {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: FilterMode::Masks,
            expression: "*.*".into(),
            case_sensitive: false,
        }
    }
}

impl PanelFilter {
    /// Syntax and compiled-pattern budgets are checked by the backend compiler.
    pub fn validate_bounds(&self) -> Result<(), &'static str> {
        if self.expression.len() > MAX_FILTER_BYTES {
            Err("A panel filter may contain at most 4096 UTF-8 bytes.")
        } else if self.expression.contains('\0') {
            Err("A panel filter cannot contain NUL.")
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct FilterValidationRequest {
    pub id: u64,
    pub pane: PaneId,
    pub filter: PanelFilter,
}
