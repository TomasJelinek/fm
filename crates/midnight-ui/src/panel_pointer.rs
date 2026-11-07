//! Observe panel ownership after child input dispatch without consuming the event.
use crate::{MainWindow, SelectionRectangle};
use i_slint_core::window::{WindowInner, WindowKind};
use slint::{ComponentHandle, platform::WindowEvent};

pub(crate) fn observe(window: &MainWindow, event: &WindowEvent) {
    if matches!(
        event,
        WindowEvent::WindowActiveChanged(false)
            | WindowEvent::CloseRequested
            | WindowEvent::PointerExited
    ) {
        window.global::<SelectionRectangle>().invoke_cancel();
    }
    let WindowEvent::PointerPressed { position, .. } = event else {
        return;
    };
    if !window.window().is_visible()
        || WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
    {
        return;
    }
    let bounds = [
        (
            window.get_left_pane_left(),
            window.get_left_pane_top(),
            window.get_left_pane_width(),
            window.get_left_pane_height(),
        ),
        (
            window.get_right_pane_left(),
            window.get_right_pane_top(),
            window.get_right_pane_width(),
            window.get_right_pane_height(),
        ),
    ];
    for (pane, (left, top, width, height)) in bounds.into_iter().enumerate() {
        let pane = pane as i32;
        if (window.get_panel_zoomed() && pane != window.get_active_pane())
            || ![left, top, width, height, position.x, position.y]
                .into_iter()
                .all(f32::is_finite)
            || position.x < left
            || position.x >= left + width
            || position.y < top
            || position.y >= top + height
        {
            continue;
        }
        // Blank presses can focus the outer shortcut scope before this observer.
        // Restore file input there, or transfer the other list's existing focus;
        // native editors and child controls retain their own focus.
        let focused = window.get_focused_pane();
        let focus_files = window.get_shell_focused() || (focused >= 0 && focused != pane);
        if pane != window.get_active_pane() {
            window.invoke_activate(pane);
        }
        if focus_files {
            window.invoke_focus_files();
        }
        break;
    }
}
