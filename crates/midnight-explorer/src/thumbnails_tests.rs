//! Exercise the real timer, presenter-backed models and Slint redraw together.
use super::*;
use midnight_core::{
    Command, DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId, PanelViewMode, UserCommand,
    UserCommandId, UserCommandInvocation, UserCommandLaunch, UserPreferences,
};
use slint::Model;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{path::PathBuf, sync::Arc, time::Instant};

pub(super) struct TestPlatform(pub(super) Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

pub(super) struct ThumbnailUi {
    pub(super) window: MainWindow,
    pub(super) presenter: Rc<RefCell<Presenter>>,
    pub(super) roots: [PathBuf; 2],
    native: Rc<MinimalSoftwareWindow>,
    size: Cell<(u32, u32)>,
}

impl ThumbnailUi {
    pub(super) fn new(counts: [usize; 2], thumbnail_size: u16) -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
        let window = MainWindow::new().unwrap();
        let root = std::env::current_dir()
            .unwrap()
            .join("_build/thumbnail-hidden-pane-fixture");
        let roots = [root.join("left"), root.join("right")];
        let presenter = Rc::new(RefCell::new(Presenter::new(
            roots[0].clone(),
            roots[1].clone(),
        )));
        let mut preferences = UserPreferences {
            thumbnails_enabled: true,
            thumbnails_network: true,
            panel_view_modes: [PanelViewMode::Thumbnails; 2],
            thumbnail_size,
            user_commands: Arc::new(vec![UserCommand {
                id: UserCommandId::new("preview-fixture").unwrap(),
                name: "Preview fixture".into(),
                enabled: true,
                toolbar_label: None,
                icon: None,
                working_directory: "${active.path}".into(),
                launch: UserCommandLaunch::Single,
                invocation: UserCommandInvocation::Program {
                    executable: root.join("command.exe"),
                    arguments: Vec::new(),
                },
            }]),
            ..Default::default()
        };
        for (tab, root) in preferences.tabs.iter_mut().zip(&roots) {
            tab.paths = vec![root.clone()];
        }
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        let pending = Rc::new(RefCell::new(Vec::new()));
        let recorded = pending.clone();
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
        let navigations = std::mem::take(&mut *pending.borrow_mut());
        assert_eq!(navigations.len(), 2);
        for (pane, navigation) in navigations {
            let entries: Arc<[Entry]> = (0..counts[pane.index()])
                .map(|index| {
                    Entry::new(
                        EntryId(index as u64),
                        roots[pane.index()].join(format!("image-{index:03}.png")),
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
        let ui = Self {
            window,
            presenter,
            roots,
            native,
            size: Cell::new((1200, 900)),
        };
        ui.resize(1200, 900);
        ui.command_artwork_visible();
        ui
    }

    pub(super) fn resize(&self, width: u32, height: u32) {
        self.size.set((width, height));
        self.native
            .set_size(slint::PhysicalSize::new(width, height));
        self.draw();
    }

    pub(super) fn draw(&self) {
        let (width, height) = self.size.get();
        let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
    }

    pub(super) fn targets(&self) -> Vec<ThumbnailTarget> {
        window_targets(&self.window, &self.presenter)
    }

    pub(super) fn panel_targets(
        &self,
        rows: &[ThumbnailTarget],
        pane: PaneId,
    ) -> Vec<ThumbnailTarget> {
        rows.iter()
            .filter(|row| {
                matches!(&row.content, midnight_ui::ThumbnailContent::Native(path)
                    if path.parent() == Some(self.roots[pane.index()].as_path()))
            })
            .cloned()
            .collect()
    }

    fn command_artwork_visible(&self) {
        let row = self.window.get_user_commands().row_data(0).unwrap();
        self.window
            .invoke_user_command_icon_visible(row.identity, row.revision, true);
    }
}

impl Drop for ThumbnailUi {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

#[test]
fn hidden_panes_do_not_schedule_thumbnail_targets_across_window_modes() {
    let ui = ThumbnailUi::new([3, 4], 96);
    let capture = content::Factory {
        native: Arc::new(|_, _| panic!("UI sampling must not capture source content")),
        storage: Arc::new(|_| panic!("UI sampling must not query storage")),
    };
    let cases = [
        ("normal", 1200, false, false, 0, [3, 4]),
        ("zoom-left", 1200, true, false, 0, [3, 0]),
        ("zoom-right", 1200, true, false, 1, [0, 4]),
        ("swapped-zoom-left", 1200, true, true, 0, [3, 0]),
        ("swapped-zoom-right", 1200, true, true, 1, [0, 4]),
        ("swapped-normal", 1200, false, true, 1, [3, 4]),
        ("compact-left", 600, false, false, 0, [3, 0]),
        ("compact-right", 600, false, false, 1, [0, 4]),
        ("swapped-compact-left", 600, false, true, 0, [3, 0]),
        ("swapped-compact-right", 600, false, true, 1, [0, 4]),
        ("restored", 1200, false, false, 0, [3, 4]),
    ];
    let mut observed = Vec::new();
    let mut expected = Vec::new();
    for (label, width, zoomed, swapped, active, wanted) in cases {
        ui.window.set_panel_zoomed(zoomed);
        ui.window.set_panels_swapped(swapped);
        ui.window.invoke_activate(active);
        ui.resize(width, 900);
        assert_eq!(ui.window.get_compact_window(), width < 640, "{label}");
        // A visible command in a popup uses the same callback even in compact mode.
        ui.command_artwork_visible();
        let rows = ui.targets();
        let counts = [
            ui.panel_targets(&rows, PaneId::Left).len(),
            ui.panel_targets(&rows, PaneId::Right).len(),
        ];
        let artwork = rows
            .iter()
            .filter(|row| {
                matches!(
                    row.content,
                    midnight_ui::ThumbnailContent::ExecutableIcon(_)
                )
            })
            .count();
        assert_eq!(artwork, 1, "command artwork survives {label}");
        let decoder_targets = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                capture.target(row, index as u64 + 1, physical_edge(row, 1.0))
            })
            .count();
        assert_eq!(decoder_targets, counts.iter().sum::<usize>());
        eprintln!(
            "thumbnail_window_work case={label} left={} right={} decoder_targets={decoder_targets} command_artwork={artwork}",
            counts[0], counts[1]
        );
        observed.push((label, counts));
        expected.push((label, wanted));
    }
    assert_eq!(
        observed, expected,
        "hidden panes must not enter thumbnail scheduling"
    );
}

#[test]
fn visible_thumbnail_pixels_reach_real_rows_while_the_other_pane_is_zoomed_out() {
    let ui = ThumbnailUi::new([2, 2], 96);
    ui.window.invoke_activate(1);
    ui.window.set_panel_zoomed(true);
    ui.draw();
    let rows = ui.targets();
    let visible = ui.panel_targets(&rows, PaneId::Right);
    assert_eq!(visible.len(), 2);
    let pixels = [14, 29, 58, 255].repeat(4);
    assert!(ui.presenter.borrow_mut().receive_thumbnail_at_resolution(
        &visible[0].identity,
        0,
        2,
        2,
        96,
        &pixels,
    ));
    midnight_ui::refresh_thumbnail_rows(
        &ui.window,
        &ui.presenter.borrow(),
        &[visible[0].identity.clone()],
    );
    let model = ui.window.get_right_rows();
    let row = (0..model.row_count())
        .filter_map(|index| model.row_data(index))
        .find(|row| row.name == "image-000.png")
        .unwrap();
    let image = row.icon.to_rgba8().unwrap();
    assert_eq!(image.width(), 2);
    assert_eq!(image.height(), 2);
    assert_eq!(image.as_bytes(), pixels.as_slice());
}

#[test]
fn panel_decode_edges_follow_dpi_while_custom_command_icons_keep_their_physical_size() {
    let mut target = ThumbnailTarget {
        identity: "panel:fixture".into(),
        content: midnight_ui::ThumbnailContent::Native("picture.png".into()),
        edge: 72,
        selected: false,
        hovered: false,
        confirmed_ssd: true,
    };
    assert_eq!(physical_edge(&target, 1.0), 96);
    assert_eq!(physical_edge(&target, 2.0), 192);
    target.edge = 256;
    assert_eq!(physical_edge(&target, 1.5), 384);
    assert_eq!(physical_edge(&target, 4.0), 512);
    assert_eq!(physical_edge(&target, f32::NAN), 256);
    target.identity = "command-icon:fixture".into();
    assert_eq!(physical_edge(&target, 4.0), 128);
}

#[test]
fn selection_and_size_keep_source_tokens_but_changed_content_replaces_them() {
    let old = ThumbnailTarget {
        identity: "row".into(),
        content: midnight_ui::ThumbnailContent::Native("picture.png".into()),
        edge: 96,
        selected: false,
        hovered: false,
        confirmed_ssd: true,
    };
    let mut changed = old.clone();
    changed.edge = 256;
    changed.selected = true;
    changed.hovered = true;
    let mut tokens = HashMap::from([(old.identity.clone(), 1)]);
    retain_source_tokens(&mut tokens, std::slice::from_ref(&old), &[changed.clone()]);
    assert_eq!(tokens.get("row"), Some(&1));
    changed.content = midnight_ui::ThumbnailContent::Native("replacement.png".into());
    retain_source_tokens(&mut tokens, &[old], &[changed]);
    assert!(
        tokens.is_empty(),
        "captured source must not survive a changed authority"
    );
}

#[test]
fn preview_initialization_failure_keeps_the_main_window_alive() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::install(&window, presenter.clone(), Rc::new(|_| true));
    native.set_size(slint::PhysicalSize::new(1000, 700));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1000);
    });

    // An empty executable identity is invalid on every platform. Discovery fails
    // before cache creation and delivers its normal asynchronous error notice.
    let service = Rc::new(Service::start(PathBuf::new(), PathBuf::new()));
    let controller = install_service(
        &window,
        presenter,
        service.clone(),
        content::Factory {
            native: std::sync::Arc::new(|_, _| panic!("no content is requested during discovery")),
            storage: std::sync::Arc::new(|_| panic!("no storage is queried during discovery")),
        },
        Rc::new(|_| true),
    );
    // Startup waits hidden for preferences. A timer pulse in that interval must
    // not permanently stop thumbnail workers before the first visible frame.
    std::thread::sleep(Duration::from_millis(150));
    slint::platform::update_timers_and_animations();
    assert!(service.set_view(View {
        generation: 1,
        targets: Vec::new()
    }));
    window.show().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.get_message_generation() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1000);
        });
    }
    assert_eq!(window.get_message_title(), "Previews");
    assert!(
        window
            .get_message_body()
            .contains("Previews are unavailable")
    );
    assert!(window.window().is_visible());
    controller.stop();
    window.hide().unwrap();
}
