//! Shared keyboard, placement and wheel presentation policy. No presenter or backend state.
#![forbid(unsafe_code)]

mod kinetic;
mod kinetic_native;
mod logical;
mod native;
mod native_scroll;
mod placement;
mod post_window_event;
mod scroll_press;
mod tooltip;
mod wheel_motion;

pub use kinetic_native::WheelSource;
pub use logical::{chord, escape_pressed};
pub use native::{ShortcutState, native_escape_pressed};
pub use native_scroll::{
    CommandStrip, GlideRegion, KineticSink, KineticSurface, NativeScroll,
    install as install_native_scroll, notify_scrollbar_wheel, popup_open,
};
pub use placement::WindowPlacement;
pub use post_window_event::{PostWindowEventObserver, observe_post_window_events};
pub use scroll_press::PressState;
pub use tooltip::synchronize_tooltip_anchors;
pub use wheel_motion::{WHEEL_FRAME, WheelMotion};
