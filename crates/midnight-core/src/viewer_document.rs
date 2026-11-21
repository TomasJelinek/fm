//! App-private document presentation state; no raster ABI change.

/// Document layout. New documents open in the layout chosen last (a viewer preference,
/// Paged until the user picks Continuous) with Fit zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentLayout {
    #[default]
    Paged,
    Continuous,
}

impl DocumentLayout {
    pub fn storage_name(self) -> &'static str {
        match self {
            Self::Paged => "paged",
            Self::Continuous => "continuous",
        }
    }

    pub fn from_storage_name(value: &str) -> Option<Self> {
        match value {
            "paged" => Some(Self::Paged),
            "continuous" => Some(Self::Continuous),
            _ => None,
        }
    }
}
