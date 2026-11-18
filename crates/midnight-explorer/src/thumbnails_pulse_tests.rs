//! The thumbnail pulse runs while a pass reports work and stops after a quiet pass;
//! presentation changes, refused sends and animation keep or restart it.
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
use std::path::PathBuf;

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
            .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
                thumbnails_enabled: true,
                thumbnails_network: true,
                panel_view_modes: [mode; 2],
                user_commands: Arc::new(user_commands.into_iter().collect()),
                ..Default::default()
            }))));
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
        // An empty executable identity fails discovery on the first preview request.
        let controller = install_service(
            &window,
            presenter.clone(),
            Rc::new(Service::start(PathBuf::new(), PathBuf::new())),
            content::Factory {
                native: Arc::new(|_, _| panic!("no content is captured")),
                storage: Arc::new(|_| panic!("no storage is queried")),
            },
            Rc::new(move |command| {
                if let Command::SetCommandIconView { targets, .. } = command {
                    views.borrow_mut().push(targets.len());
                }
                !refused.get()
            }),
        );
        let pulses = [controller.pulse()];
        window.on_presentation_changed(move || crate::pulse::wake_all(&pulses));
        let fixture = Self {
            window,
            native,
            clock,
            presenter,
            controller,
            icon_views,
            refuse,
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
        let deadline = Instant::now() + Duration::from_secs(10);
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
        .select_row(PaneId::Left, row, true, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    for pass in 0..6 {
        fixture.advance();
        assert!(fixture.running(), "pass {pass} keeps animating");
    }
    fixture
        .presenter
        .borrow_mut()
        .select_row(PaneId::Left, row, true, &|_| true);
    midnight_ui::refresh(&fixture.window, &fixture.presenter.borrow());
    fixture.settle();
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
