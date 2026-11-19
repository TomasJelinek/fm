//! Slint 1.18.1 host coordinates for embedded, input-transparent tooltip popups.
use i_slint_core::{
    items::TooltipArea,
    lengths::LogicalLength,
    window::{WindowInner, WindowKind},
};

/// Refresh active tooltip anchors to this cursor in the host's logical coordinates.
///
/// Returns whether initial stock placement needs another layout pass. Negative
/// cursor coordinates mean the pointer left the window and leave anchors intact.
/// Recheck this embedded-popup contract when upgrading Slint or adding a backend
/// with native popup windows.
pub fn synchronize_tooltip_anchors(window: &slint::Window, cursor: slint::LogicalPosition) -> bool {
    if cursor.x < 0.0 || cursor.y < 0.0 {
        return false;
    }
    // A property write can update popup geometry; release popup storage first.
    let parents = {
        let inner = WindowInner::from_pub(window);
        inner
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .filter_map(|popup| popup.parent_item.upgrade())
            .collect::<Vec<_>>()
    };
    let mut changed = false;
    for parent in parents {
        let Some(area) = parent.downcast::<TooltipArea>() else {
            continue;
        };
        let origin = parent.map_to_native_window(parent.geometry().origin);
        // Ignore sub-pixel rounding when deciding whether initial placement needs retry.
        changed |= (area.as_pin_ref().mouse_x().get() - (cursor.x - origin.x)).abs() > 0.01
            || (area.as_pin_ref().mouse_y().get() - (cursor.y - origin.y)).abs() > 0.01;
        area.as_pin_ref()
            .mouse_x
            .set(LogicalLength::new(cursor.x - origin.x));
        area.as_pin_ref()
            .mouse_y
            .set(LogicalLength::new(cursor.y - origin.y));
    }
    changed
}
