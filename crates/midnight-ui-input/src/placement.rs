//! UI-thread window geometry snapshots; no filesystem or backend work.
use slint::{
    LogicalSize, PhysicalPosition, PhysicalSize, Window, WindowSize, platform::WindowAdapter,
    winit_030::WinitWindowAccessor,
};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::Duration,
};

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;

/// Interval and count of the post-show alignment samples: at most about two seconds.
const ALIGN_INTERVAL: Duration = Duration::from_millis(25);
const ALIGN_ATTEMPTS: u32 = 80;
/// A larger error is a transient pre-placement origin or a window manager that chose its
/// own spot (both left alone), not a decoration offset.
const DECORATION_OFFSET: i64 = 128;

thread_local! {
    /// The window manager's request-to-result offset learned on this UI thread. Zero
    /// where frame extents and placement gravity agree (Windows, Mutter, KWin).
    static LEARNED_OFFSET: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

/// Initial child-window bounds and native state captured from its parent.
/// Position is the outer frame origin; size is the client area in physical pixels.
#[derive(Clone, Debug)]
pub struct WindowPlacement {
    size: WindowSize,
    logical_size: LogicalSize,
    scale_factor: Option<f32>,
    position: Option<PhysicalPosition>,
    /// The parent's native client origin and size, where absolute origins exist.
    client: Option<(PhysicalPosition, PhysicalSize)>,
    maximized: bool,
    fullscreen: bool,
}

impl WindowPlacement {
    /// Capture the parent's native bounds. Wayland leaves placement to its compositor.
    pub fn capture(parent: &Window) -> Self {
        let size = parent.size();
        let scale_factor = parent.scale_factor();
        // Wayland reports no absolute origin; Slint's (0, 0) fallback is not a position.
        let native = parent.with_winit_window(|native| {
            let outer = native.outer_position().ok()?;
            let inner = native.inner_position().ok()?;
            Some((
                PhysicalPosition::new(outer.x, outer.y),
                PhysicalPosition::new(inner.x, inner.y),
            ))
        });
        let (position, client) = match native {
            Some(Some((outer, inner))) => (Some(outer), Some((inner, size))),
            Some(None) => (None, None),
            // Not a Winit window (headless test platforms): Slint's own origin.
            None => (Some(parent.position()), None),
        };
        Self {
            size: size.into(),
            logical_size: size.to_logical(scale_factor),
            scale_factor: Some(scale_factor),
            position,
            client,
            maximized: parent.is_maximized(),
            fullscreen: parent.is_fullscreen(),
        }
    }

    /// Initial logical content size, before the native backend reports the child's monitor scale.
    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    /// Apply before showing the child so the native backend can use the requested initial bounds.
    pub fn apply(&self, child: &Window) {
        // Native child creation can wait until the next event-loop boundary.
        // Seed its captured monitor scale before physical sizing establishes the
        // first logical layout and backend font metrics; native events replace it.
        if let Some(scale_factor) = self.scale_factor {
            child.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor });
        }
        let requested = self.requested(LEARNED_OFFSET.with(Cell::get));
        if let Some(position) = requested {
            child.set_position(position);
        }
        child.set_size(self.size.clone());
        child.set_maximized(self.maximized);
        child.set_fullscreen(self.fullscreen);
        if let Some((requested, client, size)) = self.alignment(requested) {
            let adapter = i_slint_core::window::WindowInner::from_pub(child).window_adapter();
            let alignment = Rc::new(Alignment {
                child: Rc::downgrade(&adapter),
                requested,
                client,
                size,
                track: Cell::new(Track::default()),
            });
            slint::Timer::single_shot(ALIGN_INTERVAL, move || alignment.poll(1));
        }
    }

    /// The child's outer-origin request: the parent's plus the learned manager offset.
    fn requested(&self, learned: (i32, i32)) -> Option<PhysicalPosition> {
        self.position.map(|position| offset(position, learned))
    }

    /// Request, parent client origin and size to verify after showing. A maximized or
    /// fullscreen frame belongs to the manager; Wayland reports no origins.
    fn alignment(
        &self,
        requested: Option<PhysicalPosition>,
    ) -> Option<(PhysicalPosition, PhysicalPosition, PhysicalSize)> {
        if self.maximized || self.fullscreen {
            return None;
        }
        let (client, size) = self.client?;
        Some((requested?, client, size))
    }
}

/// Moves a newly shown child once so its client area covers the parent's.
///
/// An X11 window manager places a frame by its own gravity rules but may report frame
/// extents that include shadows (Weston's XWayland manager in WSLg: 32 px per side),
/// so the requested and the reported outer origin differ by a constant. Client
/// origins are exact on every backend that reports them, and both windows carry the
/// same decorations, so equal client origins and sizes mean equal frames.
///
/// The user may move or resize the child while it is sampled: that ends the check
/// (`Track::step`), so a drag is never snapped back or learned as an offset, and a
/// resized child keeps its size.
struct Alignment {
    child: Weak<dyn WindowAdapter>,
    requested: PhysicalPosition,
    client: PhysicalPosition,
    size: PhysicalSize,
    track: Cell<Track>,
}

impl Alignment {
    fn poll(self: Rc<Self>, attempt: u32) {
        let Some(adapter) = self.child.upgrade() else {
            return;
        };
        let window = adapter.window();
        if !window.is_visible() {
            // Hidden or closed before its first placement completed.
            return;
        }
        let sample = window.with_winit_window(|native| {
            // X11 reports visibility once the window manager has mapped the frame.
            let mapped = native.is_visible().unwrap_or(true);
            let client = native.inner_position().ok()?;
            Some((mapped, native.is_maximized(), (client.x, client.y)))
        });
        let requested = (self.requested.x, self.requested.y);
        let parent = (self.client.x, self.client.y);
        let outcome = match sample {
            // Not created yet, or not yet mapped by the window manager.
            None | Some(Some((false, _, _))) => Correction::Elsewhere,
            // No absolute origins, or the user already maximized it.
            Some(None) | Some(Some((true, true, _))) => return,
            Some(Some((true, false, client))) => {
                let size = window.size();
                let mut track = self.track.get();
                let step = track.step(
                    Sample {
                        client,
                        size: (size.width, size.height),
                    },
                    (self.size.width, self.size.height),
                );
                self.track.set(track);
                match step {
                    Step::Wait => Correction::Elsewhere,
                    // The user moves or resized it: leave it where it is.
                    Step::Stop => return,
                    Step::Settled(client) => correction(requested, parent, client),
                }
            }
        };
        match outcome {
            Correction::Aligned => {}
            Correction::Move(position) => {
                LEARNED_OFFSET.with(|learned| {
                    learned.set(learned_offset(learned.get(), requested, position));
                });
                window.set_position(PhysicalPosition::new(position.0, position.1));
            }
            // Keep sampling: a pre-placement origin, or a manager's own spot (left alone).
            Correction::Elsewhere => {
                if attempt < ALIGN_ATTEMPTS {
                    slint::Timer::single_shot(ALIGN_INTERVAL, move || self.poll(attempt + 1));
                }
                return;
            }
        }
        if window.size() != self.size {
            window.set_size(self.size);
        }
    }
}

/// One mapped, unmaximized sample of the child: client origin and size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    client: (i32, i32),
    size: (u32, u32),
}

/// What the samples so far say about the child.
#[derive(Clone, Copy, Debug, Default)]
struct Track {
    /// The size the child had since it was mapped, or last resized to the requested one.
    size: Option<(u32, u32)>,
    last: Option<Sample>,
    /// Two equal samples were seen: the window manager finished a placement.
    settled: bool,
    /// Consecutive changed samples since the last settled one.
    moves: u32,
}

#[derive(Debug, PartialEq)]
enum Step {
    /// Keep sampling.
    Wait,
    /// The user moves or resized the child: end the check without touching it.
    Stop,
    /// Two equal samples: compare this client origin with the parent's.
    Settled((i32, i32)),
}

impl Track {
    /// A window manager's placement jumps once (from an origin it reports before
    /// placement to its final spot) and then holds still. A child that keeps moving
    /// after it settled is being dragged, and a size other than the mapped or the
    /// requested one was chosen by the user (or the manager): either ends the check.
    fn step(&mut self, sample: Sample, requested_size: (u32, u32)) -> Step {
        let size = *self.size.get_or_insert(sample.size);
        if sample.size != size {
            if sample.size != requested_size {
                return Step::Stop;
            }
            self.size = Some(sample.size);
        }
        if self.last.replace(sample) != Some(sample) {
            if self.settled {
                self.moves += 1;
                if self.moves > 1 {
                    return Step::Stop;
                }
            }
            return Step::Wait;
        }
        self.settled = true;
        self.moves = 0;
        Step::Settled(sample.client)
    }
}

/// The result of comparing a settled child's client origin with its parent's.
#[derive(Debug, PartialEq)]
enum Correction {
    Aligned,
    /// Request this outer origin instead.
    Move((i32, i32)),
    /// More than a decoration away: not a frame-extents error.
    Elsewhere,
}

fn offset(position: PhysicalPosition, by: (i32, i32)) -> PhysicalPosition {
    PhysicalPosition::new(
        position.x.saturating_add(by.0),
        position.y.saturating_add(by.1),
    )
}

/// The outer origin to request so the child's client origin lands on the parent's.
///
/// `requested` is the child's earlier outer-origin request; client origins are the
/// observed physical ones. A window manager maps a request to a client origin by a
/// fixed translation, so the client error is also the request error.
fn correction(
    requested: (i32, i32),
    parent_client: (i32, i32),
    child_client: (i32, i32),
) -> Correction {
    if parent_client == child_client {
        return Correction::Aligned;
    }
    let error = (
        i64::from(parent_client.0) - i64::from(child_client.0),
        i64::from(parent_client.1) - i64::from(child_client.1),
    );
    if error.0.abs() > DECORATION_OFFSET || error.1.abs() > DECORATION_OFFSET {
        return Correction::Elsewhere;
    }
    let axis = |request: i32, error: i64| {
        (i64::from(request) + error).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    Correction::Move((axis(requested.0, error.0), axis(requested.1, error.1)))
}

/// The offset later children add to the parent's outer origin, so they open aligned
/// without a visible move. Only a decoration-sized total is learned.
fn learned_offset(learned: (i32, i32), requested: (i32, i32), corrected: (i32, i32)) -> (i32, i32) {
    let total = |learned: i32, requested: i32, corrected: i32| {
        i64::from(learned) + i64::from(corrected) - i64::from(requested)
    };
    let total = (
        total(learned.0, requested.0, corrected.0),
        total(learned.1, requested.1, corrected.1),
    );
    if total.0.abs() <= DECORATION_OFFSET && total.1.abs() <= DECORATION_OFFSET {
        (total.0 as i32, total.1 as i32)
    } else {
        learned
    }
}

impl From<LogicalSize> for WindowPlacement {
    fn from(size: LogicalSize) -> Self {
        Self {
            size: size.into(),
            logical_size: size,
            scale_factor: None,
            position: None,
            client: None,
            maximized: false,
            fullscreen: false,
        }
    }
}
