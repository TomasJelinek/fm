//! The thumbnail pulse runs while a pass reports work and stops after a quiet pass;
//! presentation changes, refused sends, animation and the service's ready notifications
//! keep or restart it. Only a pass that moved a row range wakes the retry pass.
use super::*;
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId, PanelViewMode, UserCommand,
    UserCommandId, UserCommandInvocation, UserCommandLaunch, UserPreferences,
};
use slint::{
    Model,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Condvar, Mutex},
};

/// The deadline of every wait; it elapses only on failure.
const DEADLINE: Duration = Duration::from_secs(10);

type Pass = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Posted {
    passes: Mutex<VecDeque<Pass>>,
    arrived: Condvar,
}

/// Stands in for Slint's event loop, which this binary's platforms do not offer: the
/// passes the service's ready notifier posts wait here until a test runs them.
#[derive(Clone, Default)]
pub(super) struct TestEventLoop(Arc<Posted>);

impl TestEventLoop {
    pub(super) fn post(&self) -> Post {
        let posted = self.0.clone();
        Arc::new(move |pass| {
            posted
                .passes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push_back(pass);
            posted.arrived.notify_all();
            true
        })
    }

    /// Block until a pass was posted; the deadline only elapses on failure.
    pub(super) fn wait(&self) {
        let passes = self
            .0
            .passes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (passes, _) = self
            .0
            .arrived
            .wait_timeout_while(passes, DEADLINE, |passes| passes.is_empty())
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            !passes.is_empty(),
            "the service posted no ready notification"
        );
    }

    /// Run the posted passes on this thread, as the event loop runs them.
    pub(super) fn run(&self) {
        loop {
            let pass = self
                .0
                .passes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front();
            let Some(pass) = pass else {
                break;
            };
            pass();
        }
    }
}

struct ClockPlatform {
    window: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

struct Fixture {
    window: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    clock: Rc<Cell<Duration>>,
    presenter: Rc<RefCell<Presenter>>,
    controller: Controller,
    icon_views: Rc<RefCell<Vec<usize>>>,
    refuse: Rc<Cell<bool>>,
    /// The icon edges the backend accepted, in order.
    icon_edges: Rc<RefCell<Vec<midnight_core::IconEdges>>>,
    /// While set, the backend refuses icon edges alone, as a full queue does.
    refuse_edges: Rc<Cell<bool>>,
    /// The cache location the service reports; never created, as discovery fails first.
    cache: PathBuf,
    event_loop: TestEventLoop,
}

impl Fixture {
    fn new(mode: PanelViewMode, command: bool) -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let clock = Rc::new(Cell::new(Duration::ZERO));
        slint::platform::set_platform(Box::new(ClockPlatform {
            window: native.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        let window = MainWindow::new().unwrap();
        let root = std::env::temp_dir().join("thumbnail pulse 文档");
        let roots = [root.join("left"), root.join("right")];
        let presenter = Rc::new(RefCell::new(Presenter::new(
            roots[0].clone(),
            roots[1].clone(),
        )));
        let user_commands = command.then(|| UserCommand {
            output: Default::default(),
            id: UserCommandId::new("editor").unwrap(),
            name: "Editor".into(),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: root.join("editor.exe"),
                arguments: Vec::new(),
            },
        });
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(
                midnight_core::AcceptedPreferences {
                    preferences: UserPreferences {
                        thumbnails_enabled: true,
                        thumbnails_network: true,
                        panel_view_modes: [mode; 2],
                        user_commands: Arc::new(user_commands.into_iter().collect()),
                        ..Default::default()
                    },
                    comparators: Default::default(),
                },
            ))));
        let navigations = Rc::new(RefCell::new(Vec::new()));
        let recorded = navigations.clone();
        midnight_ui::install(
            &window,
            presenter.clone(),
            Rc::new(move |command| {
                if let Command::Navigate {
                    pane, navigation, ..
                } = command
                {
                    recorded.borrow_mut().push((pane, navigation));
                }
                true
            }),
        );
        for (pane, navigation) in navigations.take() {
            let entries: Arc<[Entry]> = (0..3)
                .map(|index| {
                    Entry::new(
                        EntryId(index),
                        roots[pane.index()].join(format!("image-{index}.png")),
                        EntryKind::File,
                    )
                })
                .collect();
            presenter.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            });
        }
        midnight_ui::refresh(&window, &presenter.borrow());
        window.show().unwrap();
        native.set_size(slint::PhysicalSize::new(1000, 700));
        if command {
            let row = window.get_user_commands().row_data(0).unwrap();
            window.invoke_user_command_icon_visible(row.identity, row.revision, true);
        }
        let icon_views = Rc::new(RefCell::new(Vec::new()));
        let refuse = Rc::new(Cell::new(false));
        let (views, refused) = (icon_views.clone(), refuse.clone());
        let icon_edges = Rc::new(RefCell::new(Vec::new()));
        let refuse_edges = Rc::new(Cell::new(false));
        let (edges, edges_refused) = (icon_edges.clone(), refuse_edges.clone());
        let cache = root.join("thumbnail cache");
        let event_loop = TestEventLoop::default();
        // An empty executable identity fails discovery on the first preview request.
        let controller = install_service_with(
            &window,
            presenter.clone(),
            Rc::new(Service::start(PathBuf::new(), cache.clone())),
            content::Factory {
                native: Arc::new(|_, _| panic!("no content is captured")),
                storage: Arc::new(|_| panic!("no storage is queried")),
            },
            Rc::new(move |command| {
                let accepted = !refused.get()
                    && !(matches!(command, Command::SetIconEdges(_)) && edges_refused.get());
                match command {
                    Command::SetIconEdges(sent) if accepted => edges.borrow_mut().push(sent),
                    Command::SetCommandIconView { targets, .. } => {
                        views.borrow_mut().push(targets.len());
                    }
                    _ => {}
                }
                accepted
            }),
            event_loop.post(),
        );
        let pulses = [controller.pulse()];
        window.on_presentation_changed(move || crate::pulse::wake_all(&pulses));
        let now = clock.clone();
        set_clock(Rc::new(move || now.get()));
        let fixture = Self {
            window,
            native,
            clock,
            presenter,
            controller,
            icon_views,
            refuse,
            icon_edges,
            refuse_edges,
            cache,
            event_loop,
        };
        fixture.frame();
        fixture
    }

    fn frame(&self) {
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1000);
        });
    }

    fn advance(&self) {
        self.clock.set(self.clock.get() + INTERVAL);
        self.frame();
    }

    fn running(&self) -> bool {
        self.controller.pulse.running()
    }

    /// Run passes until the pulse stops. The service answers on its own thread; the
    /// deadline only elapses on failure.
    fn settle(&self) {
        let deadline = Instant::now() + DEADLINE;
        while self.running() {
            assert!(
                Instant::now() < deadline,
                "the thumbnail pulse never went quiet"
            );
            std::thread::yield_now();
            self.advance();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.controller.stop();
        let _ = self.window.hide();
    }
}

#[test]
fn icon_edges_are_sent_once_retried_when_refused_and_rows_follow_the_view() {
    use midnight_core::{ActionIcon, IconEdges, IconKey, IconPixels};
    let fixture = Fixture::new(PanelViewMode::Thumbnails, false);
    fixture.settle();
    let tiles = IconEdges {
        entry: 20,
        tile: 96,
        location: 20,
        action: 32,
    };
    assert_eq!(fixture.icon_edges.borrow().last(), Some(&tiles));
    let sent = fixture.icon_edges.borrow().len();
    for _ in 0..20 {
        fixture.advance();
        assert!(!fixture.running());
    }
    assert_eq!(
        fixture.icon_edges.borrow().len(),
        sent,
        "unchanged edges are not sent again"
    );

    // The backend delivers the parent row's art at both edges it is drawn at.
    for edge in [20, 96] {
        let pixels = vec![255; usize::from(edge) * usize::from(edge) * 4];
        fixture.presenter.borrow_mut().apply(Event::IconLoaded {
            key: IconKey::action(ActionIcon::Parent).sized(edge),
            pixels: Arc::new(IconPixels::new(edge.into(), edge.into(), pixels).unwrap()),
        });
    }
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    fixture.settle();
    let parent_icon = || {
        let row = fixture.window.get_left_rows().row_data(0).unwrap();
        assert!(row.parent_entry);
        row.icon.size().width
    };
    assert_eq!(parent_icon(), 96, "a tile");

    // A compact window lists rows, while the backend's queue is full.
    fixture.refuse_edges.set(true);
    fixture.native.set_size(slint::PhysicalSize::new(600, 300));
    for _ in 0..5 {
        fixture.advance();
        assert!(
            fixture.running(),
            "refused edges are offered again on the next pass"
        );
    }
    assert_eq!(fixture.icon_edges.borrow().len(), sent);
    assert_eq!(parent_icon(), 20, "rows follow the window at once");
    fixture.refuse_edges.set(false);
    fixture.settle();
    assert_eq!(
        fixture.icon_edges.borrow()[sent..],
        [IconEdges { tile: 0, ..tiles }]
    );
}

#[test]
fn a_static_view_stops_polling_and_a_presentation_change_runs_one_pass() {
    let fixture = Fixture::new(PanelViewMode::Detailed, false);
    fixture.settle();
    for _ in 0..100 {
        fixture.advance();
        assert!(!fixture.running(), "ten idle seconds cost no pass");
    }
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    assert!(fixture.running(), "a presentation change wakes the pulse");
    fixture.advance();
    assert!(!fixture.running(), "one quiet pass stops it again");
}

#[test]
fn an_animated_thumbnail_keeps_the_pulse_until_it_is_deselected() {
    let fixture = Fixture::new(PanelViewMode::Thumbnails, false);
    fixture.settle();
    let left = std::env::temp_dir()
        .join("thumbnail pulse 文档")
        .join("left");
    let identity = window_targets(&fixture.window, &fixture.presenter)
        .0
        .into_iter()
        .find(|target| {
            matches!(&target.content, midnight_ui::ThumbnailContent::Native(path)
                if path == &left.join("image-0.png"))
        })
        .unwrap()
        .identity;
    let pixels = [14, 29, 58, 255].repeat(4);
    for frame in 0..2 {
        assert!(
            fixture
                .presenter
                .borrow_mut()
                .receive_thumbnail_at_resolution(&identity, frame, 2, 2, 96, &pixels)
        );
    }
    let rows = fixture.window.get_left_rows();
    let row = (0..rows.row_count())
        .find(|index| rows.row_data(*index).unwrap().name == "image-0.png")
        .unwrap();
    fixture
        .presenter
        .borrow_mut()
        .select_row_modified(PaneId::Left, row, true, false, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    for pass in 0..6 {
        fixture.advance();
        assert!(fixture.running(), "pass {pass} keeps animating");
    }
    // Nobody sees a minimized window's tiles: the pulse stops until a restore,
    // whose resize wakes it, and the tile then animates again.
    fixture.window.window().set_minimized(true);
    fixture.settle();
    fixture.window.window().set_minimized(false);
    wake_pulse();
    for pass in 0..3 {
        fixture.advance();
        assert!(fixture.running(), "restored pass {pass} keeps animating");
    }
    fixture
        .presenter
        .borrow_mut()
        .select_row_modified(PaneId::Left, row, true, false, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    fixture.settle();
}

/// The left `image-0.png` tile's identity and its row in the left rows.
fn first_tile(fixture: &Fixture) -> (String, usize) {
    let left = std::env::temp_dir()
        .join("thumbnail pulse 文档")
        .join("left");
    let identity = window_targets(&fixture.window, &fixture.presenter)
        .0
        .into_iter()
        .find(|target| {
            matches!(&target.content, midnight_ui::ThumbnailContent::Native(path)
                if path == &left.join("image-0.png"))
        })
        .unwrap()
        .identity;
    let rows = fixture.window.get_left_rows();
    let row = (0..rows.row_count())
        .find(|index| rows.row_data(*index).unwrap().name == "image-0.png")
        .unwrap();
    (identity, row)
}

/// Present cycle frame `frame`, told apart by its width: `frame + 1` pixels.
fn deliver_frame(fixture: &Fixture, identity: &str, frame: u8) {
    let width = u32::from(frame) + 1;
    let pixels = [14, 29, 58, 255].repeat(width as usize);
    assert!(
        fixture
            .presenter
            .borrow_mut()
            .receive_thumbnail_at_resolution(identity, frame, width, 1, 96, &pixels)
    );
    midnight_ui::refresh_thumbnail_rows(
        &fixture.window,
        &fixture.presenter.borrow(),
        &[identity.to_owned()],
    );
    // As a delivery pass does.
    wake_pulse();
}

/// The cycle frame the tile in `row` shows.
fn shown_frame(fixture: &Fixture, row: usize) -> u32 {
    let rows = fixture.window.get_left_rows();
    rows.row_data(row).unwrap().icon.size().width - 1
}

/// Each (frame, passes shown) run of `shown`, one entry per pass.
fn runs(shown: &[u32]) -> Vec<(u32, usize)> {
    let mut runs: Vec<(u32, usize)> = Vec::new();
    for frame in shown {
        match runs.last_mut() {
            Some((last, count)) if last == frame => *count += 1,
            _ => runs.push((*frame, 1)),
        }
    }
    runs
}

/// The steps of a cycle (`midnight_ui::THUMBNAIL_CYCLE_STEP`) in pulse passes.
const STEP_PASSES: usize = 8;

/// A selected video cycles through its frames in order, each for one cycle step, however
/// often presentation changes, deliveries and backend wakes wake the pulse: no pass rate,
/// restart or extra timer speeds it up.
#[test]
fn a_selected_video_steps_through_its_frames_in_order_at_a_steady_pace() {
    let fixture = Fixture::new(PanelViewMode::Thumbnails, false);
    fixture.settle();
    let (identity, row) = first_tile(&fixture);
    for frame in 0..6 {
        deliver_frame(&fixture, &identity, frame);
    }
    fixture
        .presenter
        .borrow_mut()
        .select_row_modified(PaneId::Left, row, true, false, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    let mut shown = Vec::new();
    for pass in 0..14 * STEP_PASSES {
        // Wakes of every kind, on every pass.
        midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
        crate::pulse::wake_all(&[fixture.controller.pulse()]);
        wake_pulse();
        fixture.event_loop.run();
        if pass % 3 == 0 {
            // A redelivered frame (an upgrade check) changes nothing.
            fixture
                .presenter
                .borrow_mut()
                .receive_thumbnail_at_resolution(&identity, 2, 3, 1, 96, &[0; 12]);
        }
        fixture.advance();
        assert!(fixture.running(), "pass {pass} keeps animating");
        shown.push(shown_frame(&fixture, row));
    }
    let runs = runs(&shown);
    let frames: Vec<u32> = runs.iter().map(|(frame, _)| *frame).collect();
    assert_eq!(
        frames[..13],
        [0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 0],
        "in order from the first frame: {runs:?}"
    );
    assert!(
        runs[1..runs.len() - 1]
            .iter()
            .all(|(_, passes)| *passes == STEP_PASSES),
        "one steady step per frame: {runs:?}"
    );
    fixture
        .presenter
        .borrow_mut()
        .select_row_modified(PaneId::Left, row, true, false, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    fixture.settle();
    assert_eq!(
        shown_frame(&fixture, row),
        0,
        "a deselected tile shows its first frame"
    );
}

/// Frames that arrive late and out of order wait for their turn: the tile never shows a
/// frame before its predecessor, never steps faster than the cycle step, and plays the
/// whole cycle in order once every frame is there.
#[test]
fn late_and_out_of_order_frames_play_only_in_order() {
    let fixture = Fixture::new(PanelViewMode::Thumbnails, false);
    fixture.settle();
    let (identity, row) = first_tile(&fixture);
    deliver_frame(&fixture, &identity, 0);
    fixture
        .presenter
        .borrow_mut()
        .select_row_modified(PaneId::Left, row, true, false, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    // Frame 3 arrives after its turn: frame 2 waits for it.
    let arrivals = [(3, 2), (5, 1), (12, 4), (27, 3), (30, 5)];
    let mut shown = Vec::new();
    for pass in 0..16 * STEP_PASSES {
        for (at, frame) in arrivals {
            if at == pass {
                let before = shown_frame(&fixture, row);
                deliver_frame(&fixture, &identity, frame);
                assert_eq!(shown_frame(&fixture, row), before, "an arrival never jumps");
            }
        }
        fixture.advance();
        shown.push(shown_frame(&fixture, row));
    }
    let runs = runs(&shown);
    for pair in runs.windows(2) {
        let ((from, passes), (to, _)) = (pair[0], pair[1]);
        assert!(
            to == from + 1 || to == 0,
            "{from} -> {to}: frames play in order, each round from the first: {runs:?}"
        );
        assert!(passes >= STEP_PASSES, "{from} stepped early: {runs:?}");
    }
    let frames: Vec<u32> = runs.iter().map(|(frame, _)| *frame).collect();
    assert!(
        frames.windows(6).any(|round| round == [0, 1, 2, 3, 4, 5]),
        "a whole round once every frame is there: {runs:?}"
    );
}

#[test]
fn a_refused_command_icon_view_is_retried_without_another_wake() {
    let fixture = Fixture::new(PanelViewMode::Detailed, true);
    fixture.refuse.set(true);
    for _ in 0..3 {
        fixture.advance();
        assert!(fixture.running(), "a refused view keeps the pulse");
    }
    let refused = fixture.icon_views.borrow().len();
    assert!(refused >= 3, "each pass retries the refused view");
    fixture.refuse.set(false);
    fixture.settle();
    assert!(fixture.icon_views.borrow().len() > refused);
    assert_eq!(fixture.icon_views.borrow().last(), Some(&1));
}

#[test]
fn refreshes_callbacks_and_layout_changes_invoke_presentation_changed() {
    let fixture = Fixture::new(PanelViewMode::Detailed, false);
    let calls = Rc::new(Cell::new(0));
    let counted = calls.clone();
    fixture
        .window
        .on_presentation_changed(move || counted.set(counted.get() + 1));
    fixture.frame();
    calls.set(0);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    assert_eq!(calls.get(), 1, "one refresh, one invocation");
    // A callback handler that changes presenter state refreshes once.
    fixture.window.invoke_activate(1);
    assert_eq!(calls.get(), 2);
    fixture.frame();
    let before = calls.get();
    // Zoom changes the visible thumbnail rows without any presenter refresh.
    fixture.window.set_panel_zoomed(true);
    fixture.frame();
    assert!(calls.get() > before, "a changed handler invokes it");
}

#[test]
fn the_ready_notifier_publishes_the_cache_location_to_a_quiet_pulse() {
    let fixture = Fixture::new(PanelViewMode::Detailed, false);
    // From here only the pass's own rules and the service's notifications wake the pulse.
    fixture.window.on_presentation_changed(|| {});
    // The pulse has gone quiet before the location is reported, as while a slow cache
    // root is resolved; no pass has seen the location.
    fixture.controller.pulse.stop();
    // The service stores its location, then its ready notifier posts a UI pass.
    fixture.event_loop.wait();
    for _ in 0..10 {
        fixture.advance();
    }
    assert!(!fixture.running(), "nothing polls for the location");
    assert!(fixture.window.get_thumbnail_cache_location().is_empty());
    fixture.event_loop.run();
    assert!(fixture.running(), "the notification wakes the pulse");
    fixture.settle();
    assert_eq!(
        fixture.window.get_thumbnail_cache_location().as_str(),
        fixture.cache.to_string_lossy()
    );
}

#[test]
fn an_awaited_cache_location_no_longer_keeps_the_pulse_polling() {
    let fixture = Fixture::new(PanelViewMode::Detailed, false);
    fixture.window.on_presentation_changed(|| {});
    // Take the stored location before the first pass, so that the bridge has not seen it,
    // as while a slow cache root is resolved. Its notification stays unrun.
    fixture.event_loop.wait();
    assert!(fixture.controller.service.take_cache_location().is_some());
    fixture.settle();
    for pass in 0..20 {
        fixture.advance();
        assert!(!fixture.running(), "pass {pass}: nothing polls");
    }
    assert!(fixture.window.get_thumbnail_cache_location().is_empty());
}

#[test]
fn only_a_pass_that_moves_a_row_range_wakes_the_retry_pass() {
    let fixture = Fixture::new(PanelViewMode::Thumbnails, false);
    let passes = Rc::new(Cell::new(0));
    let counted = passes.clone();
    let retry = Pulse::start(INTERVAL, move || {
        counted.set(counted.get() + 1);
        false
    });
    fixture.controller.follow_with(&retry);
    let settle = || {
        fixture.settle();
        // A retry pass woken by the last thumbnail pass runs one interval later.
        fixture.advance();
        assert!(!retry.running());
    };
    settle();
    let before = passes.get();
    // A presentation change that keeps every row range runs thumbnail passes only.
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    assert!(fixture.running());
    settle();
    assert_eq!(
        passes.get(),
        before,
        "an unmoved tree viewport needs no retry pass"
    );
    // Zooming the left panel takes the right panel's rows, and so its viewport, away.
    fixture.window.set_panel_zoomed(true);
    fixture.frame();
    assert!(fixture.running());
    settle();
    assert!(
        passes.get() > before,
        "a moved tree viewport runs the retry pass"
    );
}
