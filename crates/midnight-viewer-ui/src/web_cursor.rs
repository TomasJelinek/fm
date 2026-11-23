//! The pointer cursor over an HTML page. Winit's pointer events over the page go to the
//! browser instead of Slint (`web_input.rs`), so Slint never updates the cursor there: the
//! cursor the page asks for (`ViewerOutput::WebCursor`, from Chromium's cursor changes) is
//! set on the native window while the pointer is over the page, and the window's default
//! comes back when it leaves. Without such a report (an older browser), a link under the
//! pointer still shows the hand. Winit sets it the same way on Windows and Linux.
use super::*;
use midnight_core::viewer_web as api;
use slint::winit_030::winit::window::CursorIcon;

/// The native cursor of a page's cursor; None hides it.
pub(super) fn icon(cursor: api::Cursor) -> Option<CursorIcon> {
    Some(match cursor {
        api::Cursor::Default => CursorIcon::Default,
        api::Cursor::Pointer => CursorIcon::Pointer,
        api::Cursor::Text => CursorIcon::Text,
        api::Cursor::VerticalText => CursorIcon::VerticalText,
        api::Cursor::Crosshair => CursorIcon::Crosshair,
        api::Cursor::Wait => CursorIcon::Wait,
        api::Cursor::Progress => CursorIcon::Progress,
        api::Cursor::Help => CursorIcon::Help,
        api::Cursor::Move => CursorIcon::Move,
        api::Cursor::AllScroll => CursorIcon::AllScroll,
        api::Cursor::NotAllowed => CursorIcon::NotAllowed,
        api::Cursor::Grab => CursorIcon::Grab,
        api::Cursor::Grabbing => CursorIcon::Grabbing,
        api::Cursor::ColResize => CursorIcon::ColResize,
        api::Cursor::RowResize => CursorIcon::RowResize,
        api::Cursor::EwResize => CursorIcon::EwResize,
        api::Cursor::NsResize => CursorIcon::NsResize,
        api::Cursor::NeswResize => CursorIcon::NeswResize,
        api::Cursor::NwseResize => CursorIcon::NwseResize,
        api::Cursor::ZoomIn => CursorIcon::ZoomIn,
        api::Cursor::ZoomOut => CursorIcon::ZoomOut,
        api::Cursor::Copy => CursorIcon::Copy,
        api::Cursor::Alias => CursorIcon::Alias,
        api::Cursor::ContextMenu => CursorIcon::ContextMenu,
        api::Cursor::Cell => CursorIcon::Cell,
        api::Cursor::None => return None,
    })
}

/// The cursor over the page now: the page's own, or without one the link hand.
pub(super) fn wanted(open: &OpenWindow) -> Option<CursorIcon> {
    match open.web_native.borrow().page_cursor() {
        Some(cursor) => icon(cursor),
        None if open.window.get_link_hover() => Some(CursorIcon::Pointer),
        None => Some(CursorIcon::Default),
    }
}

/// The page asked for `cursor`.
pub(super) fn received(open: &OpenWindow, cursor: api::Cursor) {
    open.web_native.borrow_mut().set_page_cursor(cursor);
    apply(open);
}

/// Show the page's cursor while the pointer is over the page.
pub(super) fn apply(open: &OpenWindow) {
    if open.window.get_web_mode() && open.web_native.borrow().pointer_inside() {
        set(open, wanted(open));
    }
}

/// The pointer left the page: the window's default cursor again, which Slint expects.
pub(super) fn restore(open: &OpenWindow) {
    set(open, Some(CursorIcon::Default));
}

fn set(open: &OpenWindow, cursor: Option<CursorIcon>) {
    if !open.web_native.borrow_mut().shown_cursor(cursor) {
        return;
    }
    let _ = open.window.window().with_winit_window(|window| {
        window.set_cursor_visible(cursor.is_some());
        if let Some(cursor) = cursor {
            window.set_cursor(cursor);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_cursor_has_its_native_counterpart_and_none_hides_it() {
        let expected = [
            (api::Cursor::Default, Some(CursorIcon::Default)),
            (api::Cursor::Pointer, Some(CursorIcon::Pointer)),
            (api::Cursor::Text, Some(CursorIcon::Text)),
            (api::Cursor::VerticalText, Some(CursorIcon::VerticalText)),
            (api::Cursor::Crosshair, Some(CursorIcon::Crosshair)),
            (api::Cursor::Wait, Some(CursorIcon::Wait)),
            (api::Cursor::Progress, Some(CursorIcon::Progress)),
            (api::Cursor::Help, Some(CursorIcon::Help)),
            (api::Cursor::Move, Some(CursorIcon::Move)),
            (api::Cursor::AllScroll, Some(CursorIcon::AllScroll)),
            (api::Cursor::NotAllowed, Some(CursorIcon::NotAllowed)),
            (api::Cursor::Grab, Some(CursorIcon::Grab)),
            (api::Cursor::Grabbing, Some(CursorIcon::Grabbing)),
            (api::Cursor::ColResize, Some(CursorIcon::ColResize)),
            (api::Cursor::RowResize, Some(CursorIcon::RowResize)),
            (api::Cursor::EwResize, Some(CursorIcon::EwResize)),
            (api::Cursor::NsResize, Some(CursorIcon::NsResize)),
            (api::Cursor::NeswResize, Some(CursorIcon::NeswResize)),
            (api::Cursor::NwseResize, Some(CursorIcon::NwseResize)),
            (api::Cursor::ZoomIn, Some(CursorIcon::ZoomIn)),
            (api::Cursor::ZoomOut, Some(CursorIcon::ZoomOut)),
            (api::Cursor::Copy, Some(CursorIcon::Copy)),
            (api::Cursor::Alias, Some(CursorIcon::Alias)),
            (api::Cursor::ContextMenu, Some(CursorIcon::ContextMenu)),
            (api::Cursor::Cell, Some(CursorIcon::Cell)),
            (api::Cursor::None, None),
        ];
        assert_eq!(expected.len(), api::Cursor::ALL.len());
        for (cursor, native) in expected {
            assert_eq!(icon(cursor), native, "{cursor:?}");
        }
    }
}
