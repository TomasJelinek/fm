//! Shared keyboard, placement and wheel presentation policy. No presenter or backend state.
#![forbid(unsafe_code)]

mod logical;
mod native;
mod native_scroll;
mod placement;
mod scroll_press;
mod wheel_motion;

pub use logical::{chord, escape_pressed};
pub use native::{ShortcutState, native_escape_pressed};
pub use native_scroll::{
    CommandStrip, NativeScroll, install as install_native_scroll, notify_scrollbar_wheel,
};
pub use placement::WindowPlacement;
pub use scroll_press::PressState;
pub use wheel_motion::{MAX_PENDING_PIXELS, WHEEL_DURATION, WHEEL_FRAME, WheelMotion};
