//! Composition bridge: copy bounded visible identities, and present completed RGBA buffers.
//!
//! The pulse runs while a pass reports work and stops after a quiet pass. Presentation
//! changes, native scale changes, refused sends, visibility changes, delivered frames,
//! the cache location and notices the service reports, and cache clears wake it again.
use crate::{pulse::Pulse, wake_gate::WakeGate};
use midnight_core::{
    Command, DesktopRequestId, Event, PreferencesRevision, ThumbnailerAssociations,
};
use midnight_thumbnail_host::scheduler::Work;
use midnight_thumbnail_host::service::{Service, View};
use midnight_ui::{
    BeginOwned, DeferredDrop, MainWindow, OwnedCommandScope, Presenter, ThumbnailTarget,
};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[path = "thumbnail_content.rs"]
mod content;

#[cfg(all(test, target_os = "linux"))]
#[path = "native_thumbnail_linux_tests.rs"]
mod native_linux_tests;

const INTERVAL: Duration = Duration::from_millis(100);

/// Runs a UI pass on the event loop's thread; false when no event loop can be reached
/// from another thread (a headless test platform).
type Post = Arc<dyn Fn(Box<dyn FnOnce() + Send>) -> bool + Send + Sync>;

/// Monotonic time for video cycles (`Presenter::advance_thumbnail_animation`); tests
/// substitute Slint's test clock (`set_clock`).
type Clock = Rc<dyn Fn() -> Duration>;

fn monotonic_clock() -> Clock {
    let start = Instant::now();
    Rc::new(move || start.elapsed())
}

#[derive(Clone)]
pub(crate) struct RoutingSnapshot {
    pub(crate) generation: u64,
    pub(crate) associations: Arc<ThumbnailerAssociations>,
}

pub(crate) struct Routing {
    snapshot: Mutex<RoutingSnapshot>,
    generation: AtomicU64,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(RoutingSnapshot {
                generation: 1,
                associations: Arc::default(),
            }),
            generation: AtomicU64::new(1),
        }
    }
}

impl Routing {
    pub(crate) fn snapshot(&self) -> RoutingSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Acquire) == generation
    }

    pub(crate) fn accept(&self, associations: Arc<ThumbnailerAssociations>) -> bool {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if snapshot.associations == associations {
            return false;
        }
        snapshot.generation = snapshot.generation.wrapping_add(1);
        snapshot.associations = associations;
        self.generation
            .store(snapshot.generation, Ordering::Release);
        true
    }
}

// Both queues can accumulate acknowledgements behind the window's bounded event
// drain. Settings can coalesce saves, so a later receipt releases older snapshots.
const MAX_PENDING_RECEIPTS: usize = 128;

#[derive(Default)]
struct RoutingReceipts {
    load_seen: bool,
    saved_through: Option<PreferencesRevision>,
    saves: BTreeMap<PreferencesRevision, Arc<ThumbnailerAssociations>>,
    artwork: HashMap<DesktopRequestId, u64>,
}

enum Receipt {
    Save(PreferencesRevision, Arc<ThumbnailerAssociations>),
    Artwork(DesktopRequestId, u64),
}

/// Joins successfully queued UI requests to the acknowledgements used at presentation.
pub(crate) struct RoutingCommands {
    routing: Arc<Routing>,
    import_resource: bool,
    receipts: RefCell<RoutingReceipts>,
}

impl RoutingCommands {
    pub(crate) fn new(routing: Arc<Routing>, import_resource: bool) -> Self {
        Self {
            routing,
            import_resource,
            receipts: RefCell::default(),
        }
    }

    pub(crate) fn dispatch(
        &self,
        command: Command,
        send: impl FnOnce(Command) -> bool,
        refuse: impl FnOnce(Command),
    ) -> bool {
        let receipt = match &command {
            Command::SavePreferences {
                revision,
                preferences,
            } => Some(Receipt::Save(
                *revision,
                preferences.thumbnailer_associations.clone(),
            )),
            Command::PickPath { request, input }
                if crate::command_artwork::requires_import(input.kind, self.import_resource) =>
            {
                Some(Receipt::Artwork(
                    *request,
                    self.routing.generation.load(Ordering::Acquire),
                ))
            }
            _ => None,
        };
        {
            let receipts = self.receipts.borrow();
            let full = match &receipt {
                Some(Receipt::Save(revision, _)) => {
                    receipts.saves.len() >= MAX_PENDING_RECEIPTS
                        && !receipts.saves.contains_key(revision)
                }
                Some(Receipt::Artwork(request, _)) => {
                    receipts.artwork.len() >= MAX_PENDING_RECEIPTS
                        && !receipts.artwork.contains_key(request)
                }
                None => false,
            };
            if full {
                drop(receipts);
                refuse(command);
                return false;
            }
        }
        if !send(command) {
            return false;
        }
        let mut receipts = self.receipts.borrow_mut();
        match receipt {
            Some(Receipt::Save(revision, association))
                if receipts.saved_through.is_none_or(|saved| revision > saved) =>
            {
                receipts.saves.entry(revision).or_insert(association);
            }
            Some(Receipt::Artwork(request, generation)) => {
                receipts.artwork.entry(request).or_insert(generation);
            }
            _ => {}
        }
        true
    }

    pub(crate) fn track_owned(self: &Rc<Self>, begin: BeginOwned) -> BeginOwned {
        let routing = self.clone();
        Rc::new(move || -> Option<Box<dyn OwnedCommandScope>> {
            Some(Box::new(RoutedScope {
                inner: begin()?,
                routing: routing.clone(),
            }))
        })
    }

    /// Install only a successful load or an exact successful save acknowledgement.
    /// Suppress stale picker results before the presenter can bind an imported path.
    pub(crate) fn apply(&self, event: &mut Event) -> bool {
        let mut receipts = self.receipts.borrow_mut();
        let associations = match event {
            Event::PreferencesLoaded(result) if !receipts.load_seen => {
                receipts.load_seen = true;
                result
                    .as_ref()
                    .ok()
                    .map(|accepted| accepted.preferences.thumbnailer_associations.clone())
            }
            Event::PreferencesSaved { revision, result } => {
                let Some(associations) = receipts.saves.remove(revision) else {
                    return false;
                };
                receipts.saved_through = Some(*revision);
                receipts.saves.retain(|pending, _| *pending > *revision);
                result.is_ok().then_some(associations)
            }
            Event::PathPicked { request, result } => {
                if receipts
                    .artwork
                    .remove(request)
                    .is_some_and(|generation| !self.routing.is_current(generation))
                {
                    *result = Ok(None);
                }
                None
            }
            _ => None,
        };
        drop(receipts);
        associations.is_some_and(|associations| self.routing.accept(associations))
    }
}

struct RoutedScope {
    inner: Box<dyn OwnedCommandScope>,
    routing: Rc<RoutingCommands>,
}

impl OwnedCommandScope for RoutedScope {
    fn send(&self, command: Command) -> bool {
        self.routing.dispatch(
            command,
            |command| self.inner.send(command),
            |command| self.inner.retire(Box::new(command)),
        )
    }

    fn retire(&self, value: DeferredDrop) {
        self.inner.retire(value);
    }
}

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
    let edges = midnight_ui::tile_preview_edges(window);
    let mut presenter = presenter.borrow_mut();
    presenter.set_tile_preview_edges(edges);
    let targets = presenter.thumbnail_targets(ranges, [left_hover.as_str(), right_hover.as_str()]);
    (targets, ranges)
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
    send: Rc<dyn Fn(Command) -> bool>,
    routing: Arc<Routing>,
) -> Controller {
    install_service_with_routing(
        window,
        presenter,
        Rc::new(Service::installed()),
        content::Factory::installed(filesystem),
        send,
        Arc::new(|pass| slint::invoke_from_event_loop(pass).is_ok()),
        routing,
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

/// Drive video cycles from `clock` instead of the monotonic clock.
#[cfg(test)]
fn set_clock(clock: Clock) {
    if let Some(bridge) = BRIDGE.with(|bridge| bridge.borrow().upgrade()) {
        bridge.borrow_mut().clock = clock;
    }
}

pub(crate) fn wake_pulse() {
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
    routing: Arc<Routing>,
    route_generation: u64,
    wake: Arc<WakeGate>,
    post: Post,
    deliveries: Deliveries,
    tokens: HashMap<String, u64>,
    next_token: u64,
    clock: Clock,
    last_epoch: u64,
    /// The row ranges of the last pass, and whether the latest pass moved them.
    ranges: Ranges,
    ranges_moved: bool,
    /// Icons are drawn at other edges than the rows on screen were built for.
    icons_moved: bool,
}

impl Bridge {
    /// One pass: follow view changes and animation, and take ready frames. Returns
    /// whether work remains; the pulse stops after a pass that reports none.
    fn pulse(&mut self) -> bool {
        let Some(window) = self.window.upgrade() else {
            self.service.stop();
            return false;
        };
        let routing = self.routing.snapshot();
        let route_changed = routing.generation != self.route_generation;
        if route_changed {
            self.route_generation = routing.generation;
            self.epoch.set(self.epoch.get().wrapping_add(1));
        }

        // Density, fonts, view modes, the window's size and the display scale
        // decide the edges native icons are drawn at, and each of them wakes
        // this pulse. Also while hidden: the backend then has the artwork of the
        // right size ready when the window shows.
        let icons_refused = Cell::new(false);
        // Read the window before borrowing the presenter its models read.
        let icon_view = midnight_ui::icon_view(&window);
        let icons_moved = self
            .presenter
            .borrow_mut()
            .sync_icon_view(icon_view, &|command| {
                let sent = (self.send)(command);
                icons_refused.set(!sent);
                sent
            });
        self.icons_moved |= icons_moved;
        if !window.window().is_visible() {
            if route_changed {
                // A hidden window can still have work from its previous view.
                self.service.set_view(View {
                    generation: self.epoch.get(),
                    thumbnailer_associations: routing.associations,
                    targets: Vec::new(),
                });
            }
            // Startup waits hidden for preferences; poll until the window shows.
            return true;
        }
        // A refused send is offered again on the next pass.
        let mut busy = icons_refused.get();
        if let Some(path) = self.service.take_cache_location() {
            window.set_thumbnail_cache_location(
                midnight_core::visible_name(&path.to_string_lossy())
                    .as_ref()
                    .into(),
            );
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
        let scale = window.window().scale_factor();
        // Tiles of programs show their own artwork, which the desktop actor reads.
        state.refresh_file_icons(&rows, scale, &|command| {
            let sent = (self.send)(command);
            refused.set(refused.get() || !sent);
            sent
        });
        busy |= refused.get();
        // Rows, tiles and choices take the bitmaps of their new edges.
        let mut refresh_all = std::mem::take(&mut self.icons_moved);
        let mut changed = Vec::new();
        let epoch_changed = self.epoch.get() != self.last_epoch;
        if epoch_changed {
            self.deliveries = Deliveries::default();
            self.tokens.clear();
            self.last_epoch = self.epoch.get();
            refresh_all = true;
        }
        if let Some(notice) = self.service.take_notice() {
            state.report_thumbnail_error(notice);
            refresh_all = true;
        }
        if epoch_changed || self.deliveries.changed(&rows, scale) {
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
                            | midnight_ui::ThumbnailContent::FileIcon(_)
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
            let generation = self.epoch.get();
            self.service.set_view(View {
                generation,
                thumbnailer_associations: routing.associations,
                targets,
            });
            // Budget refusals wait for a changed view. Retain only bounded metadata,
            // never the decoded pixels, and keep the same pinned source token.
            let (tokens, service) = (&self.tokens, &self.service);
            self.deliveries.set_view(rows, scale, |work| {
                if work.generation != generation {
                    return true;
                }
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
        // Each animated tile steps on its own clock, however often passes run.
        changed.extend(state.advance_thumbnail_animation((self.clock)()));
        // Keep passing while a tile animates where it can be seen. A minimized window
        // stays visible to Slint; its restore resizes the window, which wakes this pulse.
        busy |=
            !window.window().is_minimized() && !state.animated_thumbnail_identities().is_empty();
        // Slint can synchronously read presenter-backed models while refreshing.
        // Finish all mutations before it borrows the presenter again.
        drop(state);
        if refresh_all {
            midnight_ui::refresh(&window, &presenter.borrow());
        } else if !changed.is_empty() {
            midnight_ui::refresh_thumbnail_rows(&window, &presenter.borrow(), &changed);
        }
        // Nothing is awaited here: the service's ready notifier wakes the pulse for a
        // frame, the cache location or a notice that arrives later.
        busy || refresh_all
    }

    /// A backend wake: present ready frames without waiting for the next pulse.
    fn deliver(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if !window.window().is_visible()
            || self.epoch.get() != self.last_epoch
            || !self.routing.is_current(self.route_generation)
        {
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
            wake_bridge(&self.wake, &self.post);
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
fn wake_bridge(gate: &Arc<WakeGate>, post: &Post) {
    if !gate.request() {
        return;
    }
    let pass = gate.clone();
    let scheduled = post(Box::new(move || {
        // Release before draining: a frame queued after this point schedules another pass.
        pass.release();
        let bridge = BRIDGE.with(|bridge| bridge.borrow().upgrade());
        if let Some(bridge) = bridge
            && let Ok(mut bridge) = bridge.try_borrow_mut()
        {
            bridge.deliver();
        }
        // A delivered frame can start an animation or retry a refused upload; a cache
        // location or notice is taken by the pass this wake runs.
        wake_pulse();
    }));
    if !scheduled {
        // No event loop: frames, the cache location and notices stay stored for the next
        // pulse pass, which a quiet pulse runs only when something else wakes it.
        gate.release();
    }
}

#[cfg(test)]
fn install_service(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
) -> Controller {
    install_service_with(
        window,
        presenter,
        service,
        capture,
        send,
        Arc::new(|pass| slint::invoke_from_event_loop(pass).is_ok()),
    )
}

#[cfg(test)]
fn install_service_with(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
    post: Post,
) -> Controller {
    install_service_with_routing(
        window,
        presenter,
        service,
        capture,
        send,
        post,
        Arc::default(),
    )
}

fn install_service_with_routing(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
    post: Post,
    routing: Arc<Routing>,
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
    let (ready, notified) = (wake.clone(), post.clone());
    // Called for every frame, cache location and notice the service stores.
    service.set_ready_notifier(Arc::new(move || wake_bridge(&ready, &notified)));
    let route_generation = routing.snapshot().generation;
    let bridge = Rc::new(RefCell::new(Bridge {
        window: window.as_weak(),
        presenter,
        service,
        capture,
        send,
        epoch,
        routing,
        route_generation,
        wake,
        post,
        deliveries: Deliveries::default(),
        tokens: HashMap::new(),
        next_token: 1,
        clock: monotonic_clock(),
        last_epoch: 1,
        ranges: Ranges::default(),
        ranges_moved: false,
        icons_moved: false,
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
