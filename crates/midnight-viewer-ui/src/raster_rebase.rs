//! Correct a replacement page to the camera actually shown before resuming input.
//! No pixels or source data are retained here; there is only one bounded wheel intent.
use super::*;
use midnight_ui_input::WheelMotion;

#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    accepted: Option<u64>,
    // Worker transition IDs remain monotonic across files, modes and layouts.
    last_seen: u64,
}

struct Pending {
    surface: ViewerSurfaceId,
    transition: u64,
    origin: [f64; 2],
    sent: bool,
    motion: WheelMotion,
    direct: bool,
}

pub(super) fn waiting(open: &OpenWindow) -> bool {
    open.raster_rebase.borrow().pending.is_some()
}

pub(super) fn needs_retry(open: &OpenWindow) -> bool {
    open.raster_rebase
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| !pending.sent)
}

pub(super) fn receive(open: &OpenWindow, surface: ViewerSurfaceId, transition: u64) {
    {
        let mut state = open.raster_rebase.borrow_mut();
        if transition <= state.last_seen || state.pending.is_some() {
            return;
        }
        let mut retained = open.retained_raster.borrow_mut();
        if retained
            .camera
            .info()
            .is_none_or(|info| info.surface != surface)
        {
            return;
        }
        let origin = retained.camera.origin;
        let (motion, direct) = retained.camera.take_pending();
        retained.camera.suspend();
        state.last_seen = transition;
        state.accepted = None;
        state.pending = Some(Pending {
            surface,
            transition,
            origin,
            sent: false,
            motion,
            direct,
        });
    }
    open.text_position.borrow_mut().cancel();
    raster_motion::cancel_legacy(open);
    retry(open);
}

pub(super) fn retry(open: &OpenWindow) {
    let request = open
        .raster_rebase
        .borrow()
        .pending
        .as_ref()
        .filter(|pending| !pending.sent)
        .map(|pending| (pending.transition, pending.origin));
    if let Some((transition, origin)) = request
        && open.enqueue(ViewerInput::RasterRebase { transition, origin })
        && let Some(pending) = open.raster_rebase.borrow_mut().pending.as_mut()
        && pending.transition == transition
    {
        pending.sent = true;
    }
    if needs_retry(open) {
        if !open.raster_redraw.running() {
            open.raster_redraw.restart();
        }
        open.window.window().request_redraw();
    } else if waiting(open) {
        open.raster_redraw.stop();
    }
}

/// None means normal input routing may proceed; a waiting handoff owns all deltas.
pub(super) fn push(open: &OpenWindow, x: f32, y: f32, direct: bool) -> Option<bool> {
    let mut state = open.raster_rebase.borrow_mut();
    let pending = state.pending.as_mut()?;
    if x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0) {
        if direct != pending.direct {
            pending.motion.cancel();
        }
        pending.direct = direct;
        let now = image_motion::now(open);
        if direct {
            pending.motion.push_direct(x, y, now);
        } else {
            pending.motion.push(x, y, now);
        }
    }
    // Even a zero/invalid delta must not fall through to the speculative layout.
    Some(true)
}

pub(super) fn allows(open: &OpenWindow, ack: Option<u64>) -> bool {
    let state = open.raster_rebase.borrow();
    if let Some(pending) = &state.pending {
        pending.sent && ack == Some(pending.transition)
    } else {
        ack.is_none() || ack == state.accepted
    }
}

/// Called only after every validation and presentation step succeeded.
pub(super) fn presented(open: &OpenWindow, frame: &ViewerFrame) {
    let ack = frame.raster_rebase;
    let pending = {
        let mut state = open.raster_rebase.borrow_mut();
        if state
            .pending
            .as_ref()
            .is_none_or(|pending| ack != Some(pending.transition))
        {
            return;
        }
        state.accepted = ack;
        state.pending.take().unwrap()
    };
    let [x, y] = pending.motion.pending().map(|value| value as f32);
    if x == 0.0 && y == 0.0 {
        return;
    }
    // Restart the coast at presentation time. Time spent waiting cannot spend
    // animation time or become a large catch-up step on the new page.
    if open.window.get_retained_raster() {
        raster_presentation::push(open, x, y, pending.direct);
    } else {
        raster_motion::resume(
            open,
            x,
            y,
            pending.direct,
            [
                frame.width as f32 / open.window.get_raster_viewport_width().max(1.0),
                frame.height as f32 / open.window.get_raster_viewport_height().max(1.0),
            ],
        );
    }
}

/// Pointer/focus cancellation stops motion, but the page still needs correction.
pub(super) fn cancel_motion(open: &OpenWindow) {
    if let Some(pending) = &mut open.raster_rebase.borrow_mut().pending {
        pending.motion.cancel();
    }
}

pub(super) fn cancel_context(open: &OpenWindow) {
    let mut state = open.raster_rebase.borrow_mut();
    state.pending = None;
    state.accepted = None;
    open.raster_redraw.stop();
}

pub(super) fn unavailable(open: &OpenWindow, surface: ViewerSurfaceId) -> bool {
    let matches = open
        .raster_rebase
        .borrow()
        .pending
        .as_ref()
        .is_some_and(|pending| pending.surface == surface);
    if matches {
        cancel_context(open);
    }
    matches
}
