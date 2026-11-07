//! Shared keyboard policy for main and viewer windows. No presenter or backend state.
#![forbid(unsafe_code)]

mod logical;
mod native;

pub use logical::chord;
pub use native::ShortcutState;
