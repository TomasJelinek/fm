//! Composition bridge: copy bounded visible identities, and present completed RGBA buffers.
//!
//! The pulse runs while a pass reports work and stops after a quiet pass. Presentation
//! changes, native scale changes, refused sends, visibility changes, delivered frames and
//! cache clears wake it again.
use crate::{pulse::Pulse, wake_gate::WakeGate};
use midnight_core::Command;
use midnight_thumbnail_host::scheduler::Work;
use midnight_thumbnail_host::service::{Service, View};
use midnight_ui::{MainWindow, Presenter, ThumbnailTarget};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};
#[path = "thumbnail_content.rs"]
mod content;

#[cfg(all(test, target_os = "linux"))]
#[path = "native_thumbnail_linux_tests.rs"]
mod native_linux_tests;

const INTERVAL: Duration = Duration::from_millis(100);
/// After the first preview request, keep polling until the service reports its
/// initialization outcome (a frame or a notice), at most this long. The service sets
/// notices without its ready notifier, so a stopped pulse would not publish them.
const OUTCOME_WAIT: Duration = Duration::from_secs(10);

pub(crate) struct Controller {
    pulse: Rc<Pulse>,
    service: Rc<Service>,
    follower: Rc<RefCell<Weak<Pulse>>>,
}
impl Controller {
    pub(crate) fn pulse(&self) -> Weak<Pulse> {
        Rc::downgrade(&self.pulse)
    }

    /// Wake `pulse` after a pass that moved a pane's row range: the range is also that
    /// pane's tree viewport, which `pulse` (the retry pass) sends.
    pub(crate) fn follow_with(&self, pulse: &Rc<Pulse>) {
        *self.follower.borrow_mut() = Rc::downgrade(pulse);
    }

    pub(crate) fn stop(&self) {
        self.pulse.stop();
        self.service.stop();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
struct Deliveries {
    previous: Vec<ThumbnailTarget>,
    scale: f32,
    rejected: HashMap<(u64, u8), Work>,
}
impl Deliveries {
    fn record(&mut self, work: Work, retained_edge: u32) {
        if retained_edge < work.edge {
            self.rejected.insert((work.token, work.frame), work);
        } else {
            self.rejected.remove(&(work.token, work.frame));
        }
    }

    fn changed(&self, rows: &[ThumbnailTarget], scale: f32) -> bool {
        rows != self.previous || scale != self.scale
    }

    fn set_view(
        &mut self,
        rows: Vec<ThumbnailTarget>,
        scale: f32,
        mut retry: impl FnMut(Work) -> bool,
    ) {
        if !self.changed(&rows, scale) {
            return;
        }
        // Keep only bounded metadata until the view changes enough to reclaim space.
        for (_, work) in std::mem::take(&mut self.rejected) {
            if !retry(work) {
                self.rejected.insert((work.token, work.frame), work);
            }
        }
        self.previous = rows;
        self.scale = scale;
    }
}

/// Each pane's visible row range, which is also its tree viewport.
type Ranges = [(usize, usize); 2];

/// The visible thumbnail targets and the row ranges they were taken from.
fn window_targets(
    window: &MainWindow,
    presenter: &RefCell<Presenter>,
) -> (Vec<ThumbnailTarget>, Ranges) {
    let single_panel = window.get_panel_zoomed() || window.get_compact_window();
    let active_pane = window.get_active_pane();
    let (left_hover, left_range) = if !single_panel || active_pane == 0 {
        (
            window.get_left_hovered_row(),
            (
                window.get_left_thumbnail_row_start().max(0) as usize,
                window.get_left_thumbnail_row_end().max(0) as usize,
            ),
        )
    } else {
        (slint::SharedString::default(), (0, 0))
    };
    let (right_hover, right_range) = if !single_panel || active_pane == 1 {
        (
            window.get_right_hovered_row(),
            (
                window.get_right_thumbnail_row_start().max(0) as usize,
                window.get_right_thumbnail_row_end().max(0) as usize,
            ),
        )
    } else {
        (slint::SharedString::default(), (0, 0))
    };
    let ranges = [left_range, right_range];
    let targets = presenter
        .borrow_mut()
        .thumbnail_targets(ranges, [left_hover.as_str(), right_hover.as_str()]);
    (targets, ranges)
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
    send: Rc<dyn Fn(Command) -> bool>,
) -> Controller {
    install_service(
        window,
        presenter,
        Rc::new(Service::installed()),
        content::Factory::installed(filesystem),
        send,
    )
}

/// One delivery pass presents at most this many frames or spends this long uploading,
/// whichever comes first; frames left over are taken by the next wake or pulse.
const DELIVERY_FRAMES: usize = 16;
const DELIVERY_TIME: Duration = Duration::from_millis(8);

thread_local! {
    /// The UI thread's bridge, reached by backend wakes through the Slint event loop.
    static BRIDGE: RefCell<Weak<RefCell<Bridge>>> = const { RefCell::new(Weak::new()) };
    /// Its pulse, woken by delivered frames and cache clears.
    static PULSE: RefCell<Weak<Pulse>> = const { RefCell::new(Weak::new()) };
}

fn wake_pulse() {
    if let Some(pulse) = PULSE.with(|pulse| pulse.borrow().upgrade()) {
        pulse.wake();
    }
}

struct Bridge {
    window: slint::Weak<MainWindow>,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
    epoch: Rc<Cell<u64>>,
    wake: Arc<WakeGate>,
    deliveries: Deliveries,
    tokens: HashMap<String, u64>,
    next_token: u64,
    tick: u64,
    last_epoch: u64,
    /// The cache location, or the failure to resolve it, has been published.
    located: bool,
    /// Requests were made and the service has reported no frame or notice yet.
    outcome_wait: Option<Instant>,
    /// The row ranges of the last pass, and whether the latest pass moved them.
    ranges: Ranges,
    ranges_moved: bool,
}

impl Bridge {
    /// One pass: follow view changes and animation, and take ready frames. Returns
    /// whether work remains; the pulse stops after a pass that reports none.
    fn pulse(&mut self) -> bool {
        let Some(window) = self.window.upgrade() else {
            self.service.stop();
            return false;
        };
        if !window.window().is_visible() {
            // Startup waits hidden for preferences; poll until the window shows.
            return true;
        }
        let mut busy = false;
        if let Some(path) = self.service.take_cache_location() {
            window.set_thumbnail_cache_location(path.to_string_lossy().as_ref().into());
            self.located = true;
            busy = true;
        }
        let (rows, ranges) = window_targets(&window, &self.presenter);
        self.ranges_moved = ranges != self.ranges;
        self.ranges = ranges;
        let presenter = self.presenter.clone();
        let mut state = presenter.borrow_mut();
        // Native command artwork shares visible admission, but never enters an image decoder.
        // Retry a rejected send on the next pulse, even when the visible rows are unchanged.
        let refused = Cell::new(false);
        state.refresh_command_icons(&rows, &|command| {
            let sent = (self.send)(command);
            refused.set(refused.get() || !sent);
            sent
        });
        busy |= refused.get();
        let scale = window.window().scale_factor();
        let mut refresh_all = false;
        let mut changed = Vec::new();
        if self.epoch.get() != self.last_epoch {
            self.deliveries = Deliveries::default();
            self.tokens.clear();
            self.last_epoch = self.epoch.get();
            refresh_all = true;
            self.await_outcome();
        }
        if let Some(notice) = self.service.take_notice() {
            state.report_thumbnail_error(notice);
            refresh_all = true;
            self.located = true;
            self.outcome_reported();
        }
        if self.deliveries.changed(&rows, scale) {
            busy = true;
            // Entering/leaving an animated state can change the displayed
            // frame. Viewport scheduling alone has no presentation change.
            for row in self.deliveries.previous.iter().chain(&rows) {
                let old_active = self
                    .deliveries
                    .previous
                    .iter()
                    .find(|old| old.identity == row.identity)
                    .is_some_and(|old| old.selected || old.hovered);
                let new_active = rows
                    .iter()
                    .find(|new| new.identity == row.identity)
                    .is_some_and(|new| new.selected || new.hovered);
                if old_active != new_active {
                    changed.push(row.identity.clone());
                }
            }
            retain_source_tokens(&mut self.tokens, &self.deliveries.previous, &rows);
            let targets: Vec<_> = rows
                .iter()
                .filter_map(|row| {
                    if matches!(
                        row.content,
                        midnight_ui::ThumbnailContent::ExecutableIcon(_)
                    ) {
                        return None;
                    }
                    let token = if let Some(token) = self.tokens.get(&row.identity) {
                        *token
                    } else {
                        let token = self.next_token;
                        self.next_token = self.next_token.checked_add(1)?;
                        self.tokens.insert(row.identity.clone(), token);
                        token
                    };
                    self.capture.target(row, token, physical_edge(row, scale))
                })
                .collect();
            if !targets.is_empty() {
                self.await_outcome();
            }
            self.service.set_view(View {
                generation: self.epoch.get(),
                targets,
            });
            // Budget refusals wait for a changed view. Retain only bounded metadata,
            // never the decoded pixels, and keep the same pinned source token.
            let (tokens, service) = (&self.tokens, &self.service);
            self.deliveries.set_view(rows, scale, |work| {
                let Some((identity, _)) = tokens.iter().find(|(_, token)| **token == work.token)
                else {
                    return true;
                };
                let retained = state.thumbnail_coverage(identity, work.frame);
                retained >= work.edge || service.retry_delivery(work, retained)
            });
        }
        let presented = changed.len();
        busy |= self.take_ready(&mut state, scale, &mut changed) || changed.len() > presented;
        self.tick = self.tick.wrapping_add(1);
        if self.tick.is_multiple_of(2) && state.advance_thumbnail_animation() {
            changed.extend(state.animated_thumbnail_identities());
        }
        // Animation advances on every other pass; keep both while a frame animates.
        busy |= !state.animated_thumbnail_identities().is_empty();
        // Slint can synchronously read presenter-backed models while refreshing.
        // Finish all mutations before it borrows the presenter again.
        drop(state);
        if refresh_all {
            midnight_ui::refresh(&window, &presenter.borrow());
        } else if !changed.is_empty() {
            midnight_ui::refresh_thumbnail_rows(&window, &presenter.borrow(), &changed);
        }
        busy || refresh_all
            || !self.located
            || self
                .outcome_wait
                .is_some_and(|deadline| Instant::now() < deadline)
    }

    /// The first request starts waiting for the service's initialization outcome.
    fn await_outcome(&mut self) {
        if self.outcome_wait.is_none() {
            self.outcome_wait = Some(Instant::now() + OUTCOME_WAIT);
        }
    }

    /// A frame or a notice arrived: later results reach the presentation through the
    /// ready notifier or the next wake.
    fn outcome_reported(&mut self) {
        self.outcome_wait = Some(Instant::now());
    }

    /// A backend wake: present ready frames without waiting for the next pulse.
    fn deliver(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if !window.window().is_visible() || self.epoch.get() != self.last_epoch {
            return;
        }
        let presenter = self.presenter.clone();
        let mut changed = Vec::new();
        let scale = window.window().scale_factor();
        let exhausted = self.take_ready(&mut presenter.borrow_mut(), scale, &mut changed);
        if !changed.is_empty() {
            midnight_ui::refresh_thumbnail_rows(&window, &presenter.borrow(), &changed);
        }
        if exhausted {
            // Frames may remain: schedule another pass instead of waiting for the pulse.
            wake_bridge(&self.wake);
        }
    }

    /// Present ready frames within one bounded pass. Returns whether the budget ended it.
    fn take_ready(&mut self, state: &mut Presenter, scale: f32, changed: &mut Vec<String>) -> bool {
        let started = Instant::now();
        for _ in 0..DELIVERY_FRAMES {
            if started.elapsed() >= DELIVERY_TIME {
                return true;
            }
            let Some(ready) = self.service.poll() else {
                return false;
            };
            self.outcome_reported();
            if ready.generation != self.epoch.get() {
                continue;
            }
            let Some((identity, _)) = self.tokens.iter().find(|(_, token)| **token == ready.token)
            else {
                continue;
            };
            if self
                .deliveries
                .previous
                .iter()
                .find(|row| &row.identity == identity)
                .is_none_or(|row| ready.edge < physical_edge(row, scale))
            {
                continue;
            }
            let frame = &ready.frame;
            if state.receive_thumbnail_at_resolution(
                identity,
                frame.frame,
                frame.width,
                frame.height,
                ready.edge,
                &frame.rgba,
            ) {
                changed.push(identity.clone());
            }
            self.deliveries.record(
                Work {
                    generation: ready.generation,
                    token: ready.token,
                    edge: ready.edge,
                    frame: frame.frame,
                },
                state.thumbnail_coverage(identity, frame.frame),
            );
        }
        true
    }
}

/// Schedule one delivery pass on the UI thread; coalesced until that pass releases.
fn wake_bridge(gate: &Arc<WakeGate>) {
    if !gate.request() {
        return;
    }
    let pass = gate.clone();
    let scheduled = slint::invoke_from_event_loop(move || {
        // Release before draining: a frame queued after this point schedules another pass.
        pass.release();
        let bridge = BRIDGE.with(|bridge| bridge.borrow().upgrade());
        if let Some(bridge) = bridge
            && let Ok(mut bridge) = bridge.try_borrow_mut()
        {
            bridge.deliver();
        }
        // A delivered frame can start an animation or retry a refused upload.
        wake_pulse();
    });
    if scheduled.is_err() {
        // No event loop: a later pulse still delivers every frame.
        gate.release();
    }
}

fn install_service(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
) -> Controller {
    let control = service.clone();
    let epoch = Rc::new(Cell::new(1u64));
    let clear_epoch = epoch.clone();
    let clear_state = presenter.clone();
    let clear = service.clone();
    window.on_thumbnail_clear_cache(move || {
        clear_epoch.set(clear_epoch.get().wrapping_add(1));
        clear_state.borrow_mut().clear_thumbnail_images();
        clear.clear_cache();
        wake_pulse();
    });
    let wake = Arc::new(WakeGate::default());
    let ready = wake.clone();
    service.set_ready_notifier(Arc::new(move || wake_bridge(&ready)));
    let bridge = Rc::new(RefCell::new(Bridge {
        window: window.as_weak(),
        presenter,
        service,
        capture,
        send,
        epoch,
        wake,
        deliveries: Deliveries::default(),
        tokens: HashMap::new(),
        next_token: 1,
        tick: 0,
        last_epoch: 1,
        located: false,
        outcome_wait: None,
        ranges: Ranges::default(),
        ranges_moved: false,
    }));
    BRIDGE.with(|slot| *slot.borrow_mut() = Rc::downgrade(&bridge));
    let follower: Rc<RefCell<Weak<Pulse>>> = Rc::default();
    let next = follower.clone();
    let pulse = Pulse::start(INTERVAL, move || {
        let (busy, moved) = {
            let mut bridge = bridge.borrow_mut();
            let busy = bridge.pulse();
            (busy, std::mem::take(&mut bridge.ranges_moved))
        };
        // A moved row range is a moved tree viewport, which the retry pass sends.
        if moved && let Some(next) = next.borrow().upgrade() {
            next.wake();
        }
        busy
    });
    PULSE.with(|slot| *slot.borrow_mut() = Rc::downgrade(&pulse));
    Controller {
        pulse,
        service: control,
        follower,
    }
}
fn retain_source_tokens(
    tokens: &mut HashMap<String, u64>,
    previous: &[ThumbnailTarget],
    current: &[ThumbnailTarget],
) {
    tokens.retain(|identity, _| {
        previous
            .iter()
            .find(|row| &row.identity == identity)
            .is_some_and(|old| {
                current.iter().any(|new| {
                    new.identity == old.identity
                        && new.content == old.content
                        && new.confirmed_ssd == old.confirmed_ssd
                })
            })
    });
}

fn physical_edge(target: &ThumbnailTarget, scale: f32) -> u32 {
    if matches!(
        target.content,
        midnight_ui::ThumbnailContent::ExecutableIcon(_)
    ) {
        return 64;
    }
    if target.identity.starts_with("command-icon:") {
        return 128;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    midnight_thumbnail_host::cache::resolution_edge((target.edge as f32 * scale).ceil() as u32)
}

#[cfg(test)]
#[path = "thumbnails_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "thumbnails_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "command_icon_bridge_tests.rs"]
mod command_icon_bridge_tests;

#[cfg(test)]
#[path = "thumbnails_pulse_tests.rs"]
mod pulse_tests;
