//! Explicit timings for retained history and stable-settings presentation work.
use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::time::Instant;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn summarize(label: &str, samples: &mut [f64]) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{label}: median_us={:.3} p95_us={:.3} n={}",
        samples[samples.len() / 2],
        samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
        samples.len(),
    );
}

fn commit_empty(presenter: &mut Presenter, pane: PaneId, path: PathBuf) {
    assert!(presenter.navigate(pane, path, None, &|_| true));
    let navigation = NavigationId(presenter.panes[pane.index()].generation);
    presenter.apply_with_sender(
        Event::ListingFinished {
            pane,
            navigation,
            entries: Arc::from([]),
            index: Arc::new(DirectoryIndex::from_entries(&[])),
        },
        &|_| true,
    );
}

fn retire_fixture(presenter: &mut Presenter) {
    let retired = presenter.take_retired();
    // Fixture cleanup is excluded from timings and retains the worker-drop boundary.
    std::thread::spawn(move || drop(retired)).join().unwrap();
}

#[test]
#[ignore = "explicit UI history/settings benchmark; reports timings without pass thresholds"]
fn stable_tabs_and_history_presentation_performance() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    // Absolute native paths are required by persisted tab validation. These are
    // synthetic identities only; the accepting sender never performs I/O.
    let base = std::env::temp_dir().join("midnight-audit-fixture");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        base.join("left"),
        base.join("right"),
    )));
    {
        let mut state = presenter.borrow_mut();
        state.set_tab_restore_overrides([true; 2]);
        state.apply_with_sender(
            Event::PreferencesLoaded(Ok(UserPreferences::default())),
            &|_| true,
        );
        for pane in [PaneId::Left, PaneId::Right] {
            for _ in 0..7 {
                assert!(state.new_tab(pane, &|_| true));
            }
            let path = state.panes[pane.index()].display_path().to_owned();
            commit_empty(&mut state, pane, path);
        }
        retire_fixture(&mut state);
    }
    window.set_left_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Left)));
    window.set_right_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Right)));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(1340.0, 820.0),
        });
    let mut previous_visits = 0;
    for visits in [0, 1_000, 10_000] {
        {
            let mut state = presenter.borrow_mut();
            // Populate via accepted navigation/commit events, so revision-based
            // caches observe the same changes as the application.
            state.clear_command_history(&|_| true);
            for visit in previous_visits..visits {
                let path = base.join(format!(
                    "visit-{visit:05}-文档-{}",
                    "path-component-".repeat(5),
                ));
                commit_empty(&mut state, PaneId::Left, path);
                if visit % 128 == 0 {
                    retire_fixture(&mut state);
                }
            }
            retire_fixture(&mut state);
        }
        previous_visits = visits;
        for history_bytes in [0, MAX_COMMAND_HISTORY_BYTES] {
            {
                let mut state = presenter.borrow_mut();
                state.preferences.values.command_history = if history_bytes == 0 {
                    Vec::new()
                } else {
                    vec![CommandHistoryEntry {
                        command: "x".repeat(history_bytes),
                        shell: ShellFlavor::PowerShell,
                    }]
                };
                // Fixture replacement is equivalent to a new accepted history
                // revision; no command text is submitted or launched.
                state.preferences.revision.0 += 1;
                state.preferences.values.validate().unwrap();
                for pane in [PaneId::Left, PaneId::Right] {
                    assert_eq!(state.tabs(pane).len(), 8);
                    assert_eq!(state.preferences.values.tabs[pane.index()].paths.len(), 8);
                }
            }
            for _ in 0..4 {
                refresh(&window, &presenter.borrow());
            }
            let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 1340);
            });
            let mut events = Vec::with_capacity(1_000);
            let sent = Cell::new(0);
            let send = |_: Command| {
                sent.set(sent.get() + 1);
                true
            };
            for _ in 0..1_000 {
                let started = Instant::now();
                presenter.borrow_mut().apply_with_sender(
                    Event::FolderSizesFinished {
                        pane: PaneId::Left,
                        navigation: NavigationId(0),
                        scan: FolderSizeScanId(u64::MAX),
                        cancelled: false,
                    },
                    &send,
                );
                events.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            }
            assert_eq!(
                sent.get(),
                0,
                "stable tabs must not enqueue new preferences"
            );
            let mut refreshes = Vec::with_capacity(100);
            for _ in 0..100 {
                let started = Instant::now();
                refresh(&window, &presenter.borrow());
                refreshes.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            }
            let state = presenter.borrow();
            println!(
                "workload: previous_paths={visits} command_history_bytes={history_bytes} \
                 tabs_per_pane={} stable_tabs={} renderer=software refresh_excludes_raster",
                state.tabs(PaneId::Left).len(),
                state.preferences.values.tabs[0].paths.len(),
            );
            summarize("apply_with_sender_size_event", &mut events);
            summarize("refresh_properties", &mut refreshes);
            drop(state);
            retire_fixture(&mut presenter.borrow_mut());
        }
    }
    window.hide().unwrap();
    presenter.borrow_mut().retire_all();
    retire_fixture(&mut presenter.borrow_mut());
}
