//! Bridge this window's generated tooltip state to the shared pinned runtime contract.
use super::{OpenWindow, generated::TooltipPlacement};
use midnight_ui_input::{
    PostWindowEventObserver, observe_post_window_events, synchronize_tooltip_anchors,
};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, PlatformError, platform::WindowEvent};
use std::rc::Rc;

pub(super) fn install(open: &Rc<OpenWindow>) -> Result<PostWindowEventObserver, PlatformError> {
    let weak = Rc::downgrade(open);
    open.window
        .global::<TooltipPlacement>()
        .on_synchronize(move || {
            weak.upgrade()
                .is_some_and(|open| !open.closed.get() && synchronize(&open))
        });
    let weak = Rc::downgrade(open);
    observe_post_window_events(move |window, event| {
        let Some(open) = weak.upgrade() else { return };
        if open.closed.get() || !std::ptr::eq(window, open.window.window()) {
            return;
        }
        let size = match event {
            WindowEvent::Resized { size } => *size,
            _ => window.size().to_logical(window.scale_factor()),
        };
        set_bounds(&open, size);
        let placement = open.window.global::<TooltipPlacement>();
        match event {
            WindowEvent::PointerMoved { position }
            | WindowEvent::PointerPressed { position, .. }
            | WindowEvent::PointerReleased { position, .. }
            | WindowEvent::PointerScrolled { position, .. } => {
                placement.set_cursor_x(position.x);
                placement.set_cursor_y(position.y);
            }
            WindowEvent::PointerExited => {
                placement.set_cursor_x(-1.0);
                placement.set_cursor_y(-1.0);
            }
            _ => {}
        }
        synchronize_anchors(&open);
    })
}

fn set_bounds(open: &OpenWindow, size: LogicalSize) {
    if size.width > 0.0 && size.height > 0.0 {
        let placement = open.window.global::<TooltipPlacement>();
        placement.set_surface_width(size.width);
        placement.set_surface_height(size.height);
    }
}

fn synchronize_anchors(open: &OpenWindow) -> bool {
    let placement = open.window.global::<TooltipPlacement>();
    synchronize_tooltip_anchors(
        open.window.window(),
        LogicalPosition::new(placement.get_cursor_x(), placement.get_cursor_y()),
    )
}

pub(super) fn synchronize(open: &OpenWindow) -> bool {
    set_bounds(
        open,
        open.window
            .window()
            .size()
            .to_logical(open.window.window().scale_factor()),
    );
    synchronize_anchors(open)
}
