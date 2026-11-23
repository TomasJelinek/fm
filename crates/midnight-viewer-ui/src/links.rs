//! Links in a viewer window. The worker decides what a link does (`worker_links.rs` in
//! `midnight-viewer-engine`, with the policy in `midnight_core::classify_link`); this window:
//!
//! - asks the worker what the pointer is over, one question at a time, and shows the
//!   answer as the pointer cursor and on the status line (an HTML page's browser reports
//!   its hovered link itself);
//! - sends a release after a press, so a click on a link follows it;
//! - moves the keyboard focus through the shown page's links with Tab and Shift+Tab and
//!   follows the focused one with Enter, while there is a link to move to (an HTML page
//!   keeps these keys);
//! - goes back and forward through the documents opened from links with
//!   Alt+Shift+Left/Right, the mouse's Back and Forward buttons and, for an HTML page, the
//!   Back and Forward buttons, which are enabled while there is a document to go to, and
//!   Alt+Left/Right (`web_input::browser_history`); Reload (F5, Ctrl+R, its button) reads
//!   the shown file again;
//! - hands what only the main window or the system can do to the listener the
//!   application installs (`ViewerWindows::set_link_listener`).
use super::*;
use midnight_core::{ViewerLinkInput, ViewerLinkRequest};

/// What the main window does for a followed link. The reply is the viewer's status line.
pub type LinkListener = Rc<dyn Fn(ViewerLinkRequest, Box<dyn FnOnce(String)>)>;
pub(super) type Relay = Rc<RefCell<LinkListener>>;

pub(super) fn relay() -> Relay {
    Rc::new(RefCell::new(Rc::new(|_, _| {})))
}

pub(super) struct LinkState {
    relay: Relay,
    /// The shown page's links and the focused one, as the worker reported them.
    count: u32,
    focused: Option<u32>,
    /// A hover question waits for its answer; the newest pointer position is asked next.
    asking: bool,
    pending: Option<(f32, f32)>,
    /// The pointer is over the text.
    inside: bool,
}

impl LinkState {
    pub(super) fn new(relay: Relay) -> Self {
        Self {
            relay,
            count: 0,
            focused: None,
            asking: false,
            pending: None,
            inside: false,
        }
    }
}

pub(super) fn bind(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_hover_point(move |x, y| {
        if let Some(open) = weak.upgrade() {
            hover(&open, Some((x, y)));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_hover_exit(move || {
        if let Some(open) = weak.upgrade() {
            hover(&open, None);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_release_point(move |x, y| {
        if let Some(open) = weak.upgrade()
            && !raster_rebase::waiting(&open)
            && let Some(input) = raster_point(&open, x, y, true)
        {
            open.enqueue(input);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_link_history(move |back| {
        if let Some(open) = weak.upgrade() {
            history(&open, back);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_reload(move || {
        if let Some(open) = weak.upgrade() {
            reload(&open);
        }
    });
}

/// Whether Back and Forward have a document to go to, as the worker reported.
pub(super) fn history_state(open: &OpenWindow, back: bool, forward: bool) {
    open.window.set_link_back_available(back);
    open.window.set_link_forward_available(forward);
}

/// Read the shown document again from its file.
pub(super) fn reload(open: &OpenWindow) -> bool {
    open.enqueue(ViewerInput::Link(ViewerLinkInput::Reload))
}

/// The pointer moved to `point` over the text, or left it (None).
fn hover(open: &OpenWindow, point: Option<(f32, f32)>) {
    let ask = {
        let mut state = open.links.borrow_mut();
        state.inside = point.is_some();
        state.pending = point;
        point.is_some() && !state.asking
    };
    if point.is_none() {
        show(open, None);
    }
    if ask {
        ask_next(open);
    }
}

fn ask_next(open: &OpenWindow) {
    let Some((x, y)) = open.links.borrow_mut().pending.take() else {
        return;
    };
    if let Some(input) = raster_point(open, x, y, false)
        && open.enqueue(input)
    {
        open.links.borrow_mut().asking = true;
    }
}

/// A point of the displayed text as the worker reads it: in the displayed camera's raster
/// pixels, as a selection's.
fn raster_point(open: &OpenWindow, x: f32, y: f32, release: bool) -> Option<ViewerInput> {
    let state = open.retained_raster.borrow();
    let info = state.camera.info()?;
    let x = x * info.viewport[0] as f32 / open.window.get_raster_viewport_width().max(1.0);
    let y = y * info.viewport[1] as f32 / open.window.get_raster_viewport_height().max(1.0);
    let (pixel_id, origin) = (info.pixel_id, state.camera.origin);
    Some(ViewerInput::Raster {
        surface: info.surface,
        input: if release {
            ViewerRasterInput::Release {
                pixel_id,
                origin,
                x,
                y,
            }
        } else {
            ViewerRasterInput::Hover {
                pixel_id,
                origin,
                x,
                y,
            }
        },
    })
}

/// The worker's answer to a hover, or an HTML page's hovered link: where it leads, or None.
pub(super) fn answered(open: &OpenWindow, status: Option<&str>) {
    let shown = {
        let mut state = open.links.borrow_mut();
        state.asking = false;
        state.inside || open.window.get_web_mode()
    };
    show(open, if shown { status } else { None });
    ask_next(open);
}

fn show(open: &OpenWindow, status: Option<&str>) {
    open.window.set_link_hover(status.is_some());
    open.window
        .set_link_status(status.unwrap_or_default().into());
    // Over an HTML page Slint does not see the pointer: the hand is set natively.
    web_cursor::apply(open);
}

/// The shown page's links and which one has the keyboard focus.
pub(super) fn focus(open: &OpenWindow, count: u32, focused: Option<u32>) {
    let mut state = open.links.borrow_mut();
    state.count = count;
    state.focused = focused;
}

/// Back through the documents opened from links, or forward again.
pub(super) fn history(open: &OpenWindow, back: bool) -> bool {
    open.enqueue(ViewerInput::Link(if back {
        ViewerLinkInput::Back
    } else {
        ViewerLinkInput::Forward
    }))
}

/// A link action from the keyboard, or None for other actions. Tab, Shift+Tab and Enter
/// stay with the links only while there is one to move to or follow; otherwise they move
/// the focus between the window's controls as usual. HTML pages keep them.
pub(super) fn shortcut(open: &OpenWindow, action: &str) -> Option<bool> {
    let input = match action {
        "viewer.link-back" => return Some(history(open, true)),
        "viewer.link-forward" => return Some(history(open, false)),
        "viewer.reload" => return Some(reload(open)),
        "viewer.next-link" | "viewer.previous-link" | "viewer.follow-link" => {
            if open.window.get_web_mode() {
                return Some(false);
            }
            let state = open.links.borrow();
            let (input, possible) = match action {
                "viewer.next-link" => (
                    ViewerLinkInput::Next,
                    state.count > 0
                        && state
                            .focused
                            .is_none_or(|focused| focused.saturating_add(1) < state.count),
                ),
                "viewer.previous-link" => (ViewerLinkInput::Previous, state.focused.is_some()),
                _ => (ViewerLinkInput::Follow, state.focused.is_some()),
            };
            if !possible {
                return Some(false);
            }
            input
        }
        _ => return None,
    };
    Some(open.enqueue(ViewerInput::Link(input)))
}

/// Something only the main window or the system can do for a followed link. Its answer
/// goes to the worker, which shows it on the status line.
pub(super) fn request(open: &OpenWindow, request: ViewerLinkRequest) {
    let listener = open.links.borrow().relay.borrow().clone();
    let send = open.input.borrow().clone();
    listener(
        request,
        Box::new(move |status| {
            send(ViewerInput::LinkOutcome(status));
        }),
    );
}
