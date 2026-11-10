//! Shared keyboard policy for main and viewer windows. No presenter or backend state.
#![forbid(unsafe_code)]

mod logical;
mod native;
mod placement;
mod wheel_motion;

pub use logical::{chord, escape_pressed};
pub use native::{ShortcutState, native_escape_pressed};
pub use placement::WindowPlacement;
pub use wheel_motion::{MAX_PENDING_PIXELS, WHEEL_DURATION, WHEEL_FRAME, WheelMotion};
