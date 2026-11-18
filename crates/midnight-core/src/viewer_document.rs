//! App-private document presentation state; no persistence or raster ABI change.

/// Session-only document layout. New documents open Paged with Fit zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentLayout {
    #[default]
    Paged,
    Continuous,
}
