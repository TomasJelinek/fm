//! The production wake path on a platform with a cross-thread event-loop proxy and a
//! frozen clock: worker outputs are presented by wakes alone, an idle comparator arms no
//! timer (in the file form, and once a comparison and its font catalog are presented),
//! inputs held behind a full session queue are retried until delivered, inputs for a
//! worker that stopped are dropped without a retry, and a closed window is neither kept
//! alive by its worker nor touched by a late wake.
//!
//! Slint keeps the event-loop proxy process-wide (`set_platform` refuses a second platform
//! that offers one), so this is the only test in this binary that installs a platform with
//! one, and the only one whose worker can wake an event loop: the workers of other tests
//! have wakes that return false. The integration tests use proxy-less platforms, on which
//! windows poll instead.
use super::identity_tests::TestRuntime;
use super::*;
use midnight_compare_engine::INPUT_QUEUE;
use midnight_core::{
    Cancellation, FileComparatorSnapshot, FileCompareSource, FsError, PathPickerRequest,
    PathPickerService,
};
use slint::platform::{
    EventLoopProxy, Platform, PlatformError, WindowAdapter, WindowEvent,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Condvar, Mutex, mpsc},
    time::Instant,
};

type Callback = Box<dyn FnOnce() + Send>;

/// The deadline of every wait; it elapses only on failure.
const DEADLINE: Duration = Duration::from_secs(30);

/// What `invoke_from_event_loop` posts, for the test thread to run.
#[derive(Default)]
struct Posted {
    callbacks: Mutex<VecDeque<Callback>>,
    arrived: Condvar,
}

struct Proxy(Arc<Posted>);
impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, callback: Callback) -> Result<(), EventLoopError> {
        self.0.callbacks.lock().unwrap().push_back(callback);
        self.0.arrived.notify_all();
        Ok(())
    }
}

struct WakingPlatform {
    posted: Arc<Posted>,
    clock: Rc<Cell<Duration>>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}
impl Platform for WakingPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.posted.clone())))
    }
    /// Frozen: a timer fires only when the test moves the clock.
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
    /// No blinking text cursor in the file form: the comparator's timers are all that remain.
    fn cursor_flash_cycle(&self) -> Duration {
        Duration::ZERO
    }
}

/// The test thread is the event loop.
struct EventLoop {
    posted: Arc<Posted>,
    clock: Rc<Cell<Duration>>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}

impl EventLoop {
    fn install() -> Self {
        let posted = Arc::new(Posted::default());
        let clock = Rc::new(Cell::new(Duration::from_secs(1)));
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(WakingPlatform {
            posted: posted.clone(),
            clock: clock.clone(),
            windows: windows.clone(),
        }))
        .unwrap();
        Self {
            posted,
            clock,
            windows,
        }
    }

    /// The native window of the comparator opened last.
    fn native(&self) -> Rc<MinimalSoftwareWindow> {
        self.windows.borrow().last().unwrap().clone()
    }

    fn posted(&self) -> usize {
        self.posted.callbacks.lock().unwrap().len()
    }

    /// One iteration: the posted callbacks, then due timers and change handlers.
    fn run_once(&self) {
        loop {
            let callback = self.posted.callbacks.lock().unwrap().pop_front();
            let Some(callback) = callback else {
                break;
            };
            callback();
        }
        slint::platform::update_timers_and_animations();
    }

    /// Iterate until `done`, sleeping in between until a callback is posted. With
    /// `timers`, a sleep also ends when the next timer is due and the clock moves there,
    /// as in a real event loop; without, only the worker's wakes can make progress.
    fn wait_for(
        &self,
        window: &ComparatorWindow,
        what: &str,
        timers: bool,
        mut done: impl FnMut() -> bool,
    ) {
        let deadline = Instant::now() + DEADLINE;
        loop {
            self.run_once();
            if done() {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "Comparator did not {what}; status {:?}, choosing files {} ({:?})",
                window.get_status().as_str(),
                window.get_choosing_files(),
                window.get_choice_message().as_str(),
            );
            let due = slint::platform::duration_until_next_timer_update().filter(|_| timers);
            let callbacks = self.posted.callbacks.lock().unwrap();
            let (callbacks, waited) = self
                .posted
                .arrived
                .wait_timeout_while(callbacks, due.unwrap_or(left).min(left), |callbacks| {
                    callbacks.is_empty()
                })
                .unwrap();
            drop(callbacks);
            if let Some(due) = due
                && waited.timed_out()
            {
                self.clock.set(self.clock.get() + due);
            }
        }
    }

    /// Move the frozen clock and run what became due.
    fn advance(&self, by: Duration) {
        self.clock.set(self.clock.get() + by);
        self.run_once();
    }

    /// Sleep until the worker posts a callback, without running it.
    fn wait_posted(&self) {
        let callbacks = self.posted.callbacks.lock().unwrap();
        let (callbacks, _) = self
            .posted
            .arrived
            .wait_timeout_while(callbacks, DEADLINE, |callbacks| callbacks.is_empty())
            .unwrap();
        assert!(!callbacks.is_empty(), "the worker posted no wake");
    }

    /// Event-loop wakeups in `span` without input: one per due timer and posted callback.
    /// An event loop with neither sleeps until something is posted.
    fn wakeups(&self, span: Duration) -> usize {
        let end = self.clock.get() + span;
        let mut wakeups = self.posted();
        while let Some(due) = slint::platform::duration_until_next_timer_update() {
            let at = self.clock.get() + due.max(Duration::from_millis(1));
            if at > end {
                break;
            }
            self.clock.set(at);
            slint::platform::update_timers_and_animations();
            wakeups += 1;
        }
        self.clock.set(end);
        wakeups
    }
}

/// Holds the worker in `Browse` until the test answers.
struct Picker {
    asked: mpsc::Sender<()>,
    answers: Mutex<mpsc::Receiver<Option<PathBuf>>>,
}
impl PathPickerService for Picker {
    fn pick_path(
        &self,
        _: &PathPickerRequest,
        _: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        let _ = self.asked.send(());
        Ok(self
            .answers
            .lock()
            .unwrap()
            .recv_timeout(DEADLINE)
            .ok()
            .flatten())
    }
}

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [240, 240, 240, 255],
        background: [20, 20, 20, 255],
        selection: [60, 80, 120, 255],
        accent: [100, 150, 220, 255],
    }
}

#[test]
fn worker_outputs_wake_the_window_and_an_idle_comparator_arms_no_timer() {
    let event_loop = EventLoop::install();
    let fixture =
        std::env::temp_dir().join(format!("midnight-compare-wake-{}", std::process::id()));
    std::fs::create_dir_all(&fixture).unwrap();
    let files = [fixture.join("left.txt"), fixture.join("right.txt")];
    std::fs::write(&files[0], "same\nold\nsame\n").unwrap();
    std::fs::write(&files[1], "same\nnew\nsame\n").unwrap();
    let runtime = TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let (asked, asks) = mpsc::channel();
    let (answer, answers) = mpsc::channel();
    let manager = ComparatorWindows::new(
        Service::new(runtime.handle().clone(), runtime.spawner()).with_path_picker(Arc::new(
            Picker {
                asked,
                answers: Mutex::new(answers),
            },
        )),
    );
    let policy = FileComparatorSnapshot::default();
    let open_choice = || {
        manager
            .open(
                FileCompareLaunch {
                    sources: Vec::new(),
                    disabled_plugins: Arc::from([]),
                    comparators: policy.preferences.clone(),
                    comparator_policy: policy.id,
                },
                appearance(),
                slint::LogicalSize::new(800.0, 600.0),
                Rc::new(|_, _| None),
            )
            .unwrap()
            .upgrade()
            .unwrap()
    };

    // Statuses and file choices arrive by wakes alone: the clock does not move. The
    // second window stays in its file form until the end.
    let window = open_choice();
    let native = event_loop.native();
    let waiting = open_choice();
    let open = manager.windows.borrow()[0].clone();
    assert!(
        manager
            .windows
            .borrow()
            .iter()
            .all(|open| !open.poll.running()),
        "a window that can be woken never polls"
    );
    event_loop.wait_for(&window, "offer the file choices", false, || {
        window.get_choosing_files() && waiting.get_choosing_files()
    });

    // Idle: once the first geometry's batch interval has passed, nothing is armed or posted.
    event_loop.advance(BATCH_INTERVAL);
    assert!(
        manager
            .windows
            .borrow()
            .iter()
            .all(|open| !open.batch.running() && open.pending.borrow().is_empty())
    );
    assert_eq!(
        event_loop.wakeups(Duration::from_secs(1)),
        0,
        "idle comparators must not wake the event loop"
    );

    // A full session queue: the worker waits in the picker while inputs pile up.
    window.invoke_browse_file(1);
    asks.recv_timeout(DEADLINE)
        .expect("a quiet window sends an input at once");
    let extra = 8;
    for _ in 0..INPUT_QUEUE + extra {
        window.invoke_difference(false);
    }
    native.set_size(slint::PhysicalSize::new(700, 500));
    event_loop.run_once();
    let revision = window.get_geometry_revision();
    assert_eq!(
        open.pending.borrow().len(),
        INPUT_QUEUE + extra,
        "inputs right after a sent batch wait for the next one"
    );
    event_loop.advance(BATCH_INTERVAL);
    assert_eq!(
        open.pending.borrow().len(),
        extra + 1,
        "the session queue takes what fits"
    );
    event_loop.advance(BATCH_INTERVAL);
    assert_eq!(open.pending.borrow().len(), extra + 1);
    assert!(open.batch.running(), "refused inputs are retried");
    answer.send(Some(files[1].clone())).unwrap();
    let picked = files[1].to_string_lossy().into_owned();
    event_loop.wait_for(&window, "show the picked file", false, || {
        window.get_file_b() == picked.as_str()
    });
    window.set_file_a(files[0].to_string_lossy().as_ref().into());
    event_loop.wait_for(&window, "deliver the queued geometry", true, || {
        open.pending.borrow().is_empty()
    });
    // During backpressure the chooser must report a refusal, so retry explicitly
    // if the worker is still finishing an earlier batch.
    event_loop.wait_for(&window, "accept the chosen files", true, || {
        if !window.get_choice_busy() && window.get_choosing_files() {
            window.invoke_compare_chosen_files();
        }
        window.get_choice_busy() || !window.get_choosing_files()
    });
    event_loop.wait_for(&window, "present the comparison", true, || {
        !window.get_choosing_files() && window.get_frame_revision() >= 0
    });
    assert_eq!(window.get_frame_revision(), revision);
    assert!(open.pending.borrow().is_empty());

    // A resize and a DPI change each send exactly one Resize.
    let before = window.get_geometry_revision();
    native.set_size(slint::PhysicalSize::new(640, 480));
    event_loop.run_once();
    event_loop.run_once();
    assert_eq!(window.get_geometry_revision(), before + 1);
    event_loop.wait_for(&window, "present the resized frame", true, || {
        window.get_frame_revision() == window.get_geometry_revision()
    });
    let logical = (window.get_content_width(), window.get_content_height());
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    // The same logical size at the new scale, as a backend reports it.
    native.set_size(slint::PhysicalSize::new(1280, 960));
    event_loop.run_once();
    event_loop.run_once();
    assert_eq!(
        (window.get_content_width(), window.get_content_height()),
        logical
    );
    assert_eq!(
        window.get_geometry_revision(),
        before + 2,
        "a DPI change alone re-renders, once"
    );
    event_loop.wait_for(&window, "present the frame at the new scale", true, || {
        window.get_frame_revision() == window.get_geometry_revision()
    });
    let surface = window.get_surface().size();
    assert_eq!(
        (surface.width, surface.height),
        ((logical.0 * 2.0) as u32, (logical.1 * 2.0) as u32)
    );

    // A presented comparison is idle too, once its font catalog has arrived (the worker
    // repaints with it) and its last batch interval has passed.
    event_loop.wait_for(&window, "present the font catalog", false, || {
        window.get_fonts_ready()
    });
    event_loop.advance(BATCH_INTERVAL);
    assert!(!open.batch.running() && !open.poll.running() && open.pending.borrow().is_empty());
    assert_eq!(
        event_loop.wakeups(Duration::from_secs(1)),
        0,
        "an idle comparison must not wake the event loop"
    );

    // A worker that stopped after an internal error takes no more input: what waits is
    // dropped and no timer retries it.
    let stopping_runtime = TestRuntime::new(tokio::runtime::Runtime::new().unwrap());
    let stopping = ComparatorWindows::new(Service::with_native_capture(
        stopping_runtime.handle().clone(),
        stopping_runtime.spawner(),
        Arc::new(|_, _| panic!("deliberate capture fault: the comparison worker stops")),
    ));
    let stopped = stopping
        .open(
            FileCompareLaunch {
                sources: files
                    .iter()
                    .cloned()
                    .map(FileCompareSource::Focused)
                    .collect(),
                disabled_plugins: Arc::from([]),
                comparators: policy.preferences,
                comparator_policy: policy.id,
            },
            appearance(),
            slint::LogicalSize::new(800.0, 600.0),
            Rc::new(|_, _| None),
        )
        .unwrap()
        .upgrade()
        .unwrap();
    let stopped_native = event_loop.native();
    let broken = stopping.windows.borrow()[0].clone();
    event_loop.wait_for(&stopped, "report the stopped worker", true, || {
        stopped.get_status().contains("internal error")
    });
    event_loop.advance(BATCH_INTERVAL);
    let before = stopped.get_geometry_revision();
    stopped.invoke_difference(false);
    stopped_native.set_size(slint::PhysicalSize::new(600, 400));
    event_loop.run_once();
    event_loop.run_once();
    assert!(
        stopped.get_geometry_revision() > before,
        "a resize was sent"
    );
    assert!(
        broken.pending.borrow().is_empty(),
        "inputs for a stopped worker are dropped"
    );
    assert!(!broken.batch.running(), "a stopped worker is not retried");
    event_loop.advance(BATCH_INTERVAL);
    assert!(!broken.batch.running() && broken.pending.borrow().is_empty());
    assert_eq!(
        event_loop.wakeups(Duration::from_secs(1)),
        0,
        "the window of a stopped worker must not wake the event loop"
    );
    stopping.close_all();
    drop((stopped, stopped_native, broken, stopping, stopping_runtime));

    // Closing: workers hold only weak handles, and a wake that arrives late touches
    // nothing. The second worker waits in its picker; the first posts a frame's wake.
    event_loop.advance(BATCH_INTERVAL);
    waiting.invoke_browse_file(0);
    asks.recv_timeout(DEADLINE)
        .expect("a quiet window sends an input at once");
    window.invoke_difference(false);
    event_loop.wait_posted();
    let gone = [window.as_weak(), waiting.as_weak()];
    manager.close_all();
    drop((window, waiting, open, native));
    assert!(
        gone.iter().all(|window| window.upgrade().is_none()),
        "a running worker must not keep its closed window alive"
    );
    event_loop.run_once();
    answer.send(None).unwrap();
    drop(manager);
    drop(runtime);
    std::fs::remove_dir_all(&fixture).unwrap();
}
