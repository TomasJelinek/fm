//! Opt-in end-to-end panel probe: the production Engine and installed native
//! filesystem plugin feed a real Presenter, row model and software-rendered
//! main window, pumped as the application's event loop does (up to 16 events or
//! 3 ms per turn, then one refresh). It measures navigation, sorting, bulk
//! selection and quick search on caller-made folders; it asserts nothing about time.
//!
//! Usage (release, the native filesystem package installed in `filesystems\`
//! beside this executable):
//! `listing_probe generate <root>` makes `<root>\n1000`, `n10000` and `n100000`;
//! `listing_probe measure <rounds> <folder>...` measures each folder;
//! `listing_probe components <rounds> <folder>` times the listing's parts one by one.
//! User preferences are never read or written.
#[allow(dead_code)]
#[path = "../src/command_reclaim.rs"]
mod command_reclaim;

use midnight_core::*;
use midnight_engine::{Engine, reclaim};
use midnight_native_filesystem_host::InstalledProvider;
use midnight_ui::{
    BeginOwned, MainWindow, Presenter, install_with_owned, refresh, with_owned_commands,
};
use slint::{
    ComponentHandle, Model,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::RefCell,
    fs,
    io::Write,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

const WIDTH: usize = 1340;
const HEIGHT: usize = 820;
const FRAME: Duration = Duration::from_millis(16);

struct ProbePlatform(Rc<MinimalSoftwareWindow>);
impl Platform for ProbePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

struct Settings(UserPreferences);
impl SettingsService for Settings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(self.0.clone())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }
    fn save_preferences_prepared(
        &self,
        _: UserPreferences,
        _: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        Ok(())
    }
}

/// Names as a mixed working folder has them: numbered for natural sort, mixed
/// case, accents, CJK, several extensions, every tenth entry a folder.
fn fixture_name(index: usize) -> (String, bool) {
    const STEMS: [&str; 8] = [
        "Report",
        "photo_",
        "Čtvrtletí",
        "notes",
        "IMG-",
        "文档",
        "build.log",
        "Zebra",
    ];
    const EXTENSIONS: [&str; 10] = [
        "txt", "rs", "PNG", "jpeg", "tar.gz", "pdf", "docx", "", "md", "zip",
    ];
    let stem = STEMS[index % STEMS.len()];
    let folder = index % 10 == 9;
    let extension = EXTENSIONS[(index / 8) % EXTENSIONS.len()];
    let name = if folder || extension.is_empty() {
        format!("{stem} {index}")
    } else {
        format!("{stem} {index}.{extension}")
    };
    (name, folder)
}

fn generate(root: &Path) -> std::io::Result<()> {
    for count in [1_000, 10_000, 100_000] {
        let folder = root.join(format!("n{count}"));
        fs::create_dir_all(&folder)?;
        let started = Instant::now();
        for index in 0..count {
            let (name, directory) = fixture_name(index);
            let path = folder.join(name);
            if directory {
                fs::create_dir_all(path)?;
            } else {
                let mut file = fs::File::create(path)?;
                file.write_all(&vec![b'x'; index % 4096])?;
            }
        }
        println!(
            "generated {} entries in {} ({:.1} s)",
            count,
            folder.display(),
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

/// What one pumped turn saw.
#[derive(Default, Clone, Copy)]
struct Turn {
    events: usize,
    batches: usize,
    finished: bool,
    selection: bool,
    search: bool,
}

struct Probe {
    window: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    pixels: Vec<slint::Rgb8Pixel>,
    presenter: Rc<RefCell<Presenter>>,
    begin_owned: BeginOwned,
    command_reclaim: command_reclaim::CommandReclaim,
    events: mpsc::Receiver<Event>,
    runtime: tokio::runtime::Runtime,
    committed: [Option<NavigationId>; 2],
    /// Time spent applying events, refreshing and drawing, since the last reset.
    apply: Duration,
    refresh: Duration,
    draw: Duration,
    refreshes: usize,
    draws: usize,
    last_draw: Instant,
    /// Events by variant since the step began: count, first arrival after the step
    /// began (ms) and the time spent applying them.
    seen: std::collections::BTreeMap<String, (usize, f64, Duration)>,
    step_started: Instant,
}

impl Probe {
    fn draw(&mut self) {
        let started = Instant::now();
        slint::platform::update_timers_and_animations();
        let pixels = &mut self.pixels;
        self.native.draw_if_needed(|renderer| {
            renderer.render(pixels, WIDTH);
        });
        self.draw += started.elapsed();
        self.draws += 1;
        self.last_draw = Instant::now();
    }

    fn reset_counters(&mut self) {
        self.apply = Duration::ZERO;
        self.refresh = Duration::ZERO;
        self.draw = Duration::ZERO;
        self.refreshes = 0;
        self.draws = 0;
        self.seen.clear();
        self.step_started = Instant::now();
    }

    /// Where the step's time went: per event variant, its count, first arrival and apply time.
    fn breakdown(&self, step: &str) {
        let mut line = format!(
            "  {step}: refresh {:.1} ms x{}, draw {:.1} ms x{};",
            ms(self.refresh),
            self.refreshes,
            ms(self.draw),
            self.draws
        );
        for (name, (count, first, apply)) in &self.seen {
            line.push_str(&format!(
                " {name} x{count} first {first:.1} ms apply {:.1} ms;",
                ms(*apply)
            ));
        }
        eprintln!("{line}");
    }

    /// One turn of the application's event pump; waits up to `wait` for its first event.
    fn turn(&mut self, wait: Duration) -> Turn {
        let mut seen = Turn::default();
        let first = self.runtime.block_on(async {
            tokio::time::timeout(wait, self.events.recv())
                .await
                .ok()
                .flatten()
        });
        let Some(first) = first else {
            return seen;
        };
        let mut next = Some(first);
        let start = Instant::now();
        for _ in 0..16 {
            let event = match next.take() {
                Some(event) => event,
                None => match self.events.try_recv() {
                    Ok(event) => event,
                    Err(_) => break,
                },
            };
            seen.events += 1;
            let name = variant(&event);
            let arrived = ms(self.step_started.elapsed());
            self.seen
                .entry(name.to_owned())
                .or_insert((0, arrived, Duration::ZERO))
                .0 += 1;
            match &event {
                Event::EntriesBatch { pane, .. } if *pane == PaneId::Left => seen.batches += 1,
                Event::ListingFinished { pane, .. } if *pane == PaneId::Left => {
                    seen.finished = true
                }
                Event::SelectionChanged { pane, .. } if *pane == PaneId::Left => {
                    seen.selection = true
                }
                Event::SearchReady { pane, .. } if *pane == PaneId::Left => seen.search = true,
                _ => {}
            }
            let applied = Instant::now();
            with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                if let Event::DirectoryChanged { pane, navigation } = event {
                    presenter.refresh_if_changed(pane, navigation, send);
                } else {
                    presenter.apply_with_sender(event, send);
                }
            });
            self.command_reclaim.flush();
            self.apply += applied.elapsed();
            if let Some(slot) = self.seen.get_mut(name) {
                slot.2 += applied.elapsed();
            }
            if start.elapsed() >= Duration::from_millis(3) {
                break;
            }
        }
        let refreshed = Instant::now();
        refresh(&self.window, &self.presenter.borrow());
        self.sync_visible();
        self.refresh += refreshed.elapsed();
        self.refreshes += 1;
        if self.last_draw.elapsed() >= FRAME {
            self.draw();
        }
        seen
    }

    fn sync_visible(&mut self) {
        for pane in [PaneId::Left, PaneId::Right] {
            let committed = &mut self.committed;
            let _ = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
                if let Some(visible) = presenter.visible_directory(pane) {
                    let navigation = visible.navigation;
                    if committed[pane.index()] != Some(navigation)
                        && send(Command::PaneCommitted(visible))
                    {
                        committed[pane.index()] = Some(navigation);
                    }
                }
            });
        }
        let _ = with_owned_commands(&self.presenter, &self.begin_owned, |presenter, send| {
            presenter.flush_selection(send);
            presenter.flush_search(send);
        });
    }

    /// Pumps until `done` holds after a turn; then draws a frame.
    fn until(&mut self, step: &str, mut done: impl FnMut(&Self, Turn) -> bool) {
        let deadline =
            Instant::now() + Duration::from_secs(if step == "startup" { 10 } else { 60 });
        self.seen.clear();
        loop {
            let turn = self.turn(Duration::from_millis(5));
            if done(self, turn) {
                break;
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "step {step} timed out (left loading {}, right loading {}); events seen: {:?}",
                    self.window.get_left_loading(),
                    self.window.get_right_loading(),
                    self.seen
                );
                if step != "startup" {
                    std::process::exit(2);
                }
                break;
            }
        }
        self.draw();
    }

    fn quiet(&mut self) {
        loop {
            if self.turn(Duration::from_millis(50)).events == 0 {
                break;
            }
        }
        self.draw();
        let retired = self.presenter.borrow_mut().take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
    }

    fn left_rows(&self) -> usize {
        self.window.get_left_rows().row_count()
    }

    fn row_selected(&self, row: usize) -> bool {
        self.window
            .get_left_rows()
            .row_data(row)
            .is_some_and(|row| row.selected)
    }
}

/// The event's variant, without formatting its payload.
fn variant(event: &Event) -> &'static str {
    match event {
        Event::ListingStarted { .. } => "ListingStarted",
        Event::EntriesBatch { .. } => "EntriesBatch",
        Event::MetadataBatch { .. } => "MetadataBatch",
        Event::ListingChanges { .. } => "ListingChanges",
        Event::ListingFinished { .. } => "ListingFinished",
        Event::SelectionChanged { .. } => "SelectionChanged",
        Event::SearchReady { .. } => "SearchReady",
        Event::SpaceUpdated { .. } => "SpaceUpdated",
        Event::StorageKindUpdated { .. } => "StorageKindUpdated",
        Event::VolumesUpdated(_) => "VolumesUpdated",
        Event::VolumeDetailsUpdated { .. } => "VolumeDetailsUpdated",
        Event::DirectoryChanged { .. } => "DirectoryChanged",
        Event::PreferencesLoaded(_) => "PreferencesLoaded",
        Event::FilenameColorsReady { .. } => "FilenameColorsReady",
        Event::SelectedFolderSizes { .. } => "SelectedFolderSizes",
        Event::FolderSizeUpdated { .. } => "FolderSizeUpdated",
        Event::FolderSizesFinished { .. } => "FolderSizesFinished",
        Event::IconLoaded { .. } => "IconLoaded",
        Event::FileIconLoaded { .. } => "FileIconLoaded",
        Event::WatchStatus { .. } => "WatchStatus",
        _ => "other",
    }
}

struct Samples(Vec<f64>);
impl Samples {
    fn median(&self) -> f64 {
        let mut values = self.0.clone();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }
    fn range(&self) -> (f64, f64) {
        let min = self.0.iter().copied().fold(f64::INFINITY, f64::min);
        let max = self.0.iter().copied().fold(0.0, f64::max);
        (min, max)
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Default)]
struct Report(Vec<(String, Samples)>);
impl Report {
    fn add(&mut self, label: &str, value: f64) {
        if let Some((_, samples)) = self.0.iter_mut().find(|(name, _)| name == label) {
            samples.0.push(value);
        } else {
            self.0.push((label.to_owned(), Samples(vec![value])));
        }
    }
    fn print(&self) {
        for (label, samples) in &self.0 {
            let (min, max) = samples.range();
            println!(
                "{label:<44} median {:>10.3}  range {:>10.3} - {:>10.3}  n={}",
                samples.median(),
                min,
                max,
                samples.0.len()
            );
        }
    }
}

fn measure(rounds: usize, folders: &[PathBuf]) {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(ProbePlatform(native.clone()))).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()
        .unwrap();
    let _guard = runtime.enter();
    let home = folders[0].parent().unwrap().to_path_buf();
    let other = home.clone();
    let shared = InstalledProvider::shared_composition(None);
    let local = Arc::new(midnight_platform::LocalFileSystem::new(None));
    shared.service().set_size_directory_service(local.clone());
    let (events_send, events) = mpsc::channel(32);
    let mut engine = Engine::with_shared_native_filesystem(
        Engine::lifetime().unwrap(),
        shared,
        events_send,
        Arc::new(|| {}),
    );
    engine.configure_audited_locale().unwrap().install(local);
    let mut preferences = UserPreferences::default();
    preferences.tabs[0].paths = vec![home.clone()];
    preferences.tabs[1].paths = vec![other.clone()];
    engine
        .configure_settings()
        .unwrap()
        .install(Arc::new(Settings(preferences)));
    let (commands, receive) = engine.command_channel(32).unwrap();
    let retire_queue = reclaim::start(runtime.handle(), Arc::new(|| {}));
    let command_reclaim = command_reclaim::CommandReclaim::new(retire_queue.clone());
    let engine = runtime.spawn(engine.run(receive));

    let window = MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    midnight_ui::install_file_keys(&window);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let presenter = Rc::new(RefCell::new(Presenter::new(home.clone(), other.clone())));
    let begin_owned = command_reclaim.factory(commands.clone());
    let begin_controls = begin_owned.clone();
    let sender = commands.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        if let Some(scope) = begin_controls() {
            return scope.send(command);
        }
        sender.try_send(command).is_ok()
    });
    install_with_owned(&window, presenter.clone(), send, begin_owned.clone());
    let mut probe = Probe {
        window,
        native,
        pixels: vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT],
        presenter,
        begin_owned,
        command_reclaim,
        events,
        runtime,
        committed: [None; 2],
        apply: Duration::ZERO,
        refresh: Duration::ZERO,
        draw: Duration::ZERO,
        refreshes: 0,
        draws: 0,
        last_draw: Instant::now(),
        seen: Default::default(),
        step_started: Instant::now(),
    };
    probe.until("startup", |probe, _| {
        !probe.window.get_left_loading() && !probe.window.get_right_loading()
    });
    probe.quiet();

    let mut report = Report::default();
    for round in 0..rounds {
        for folder in folders {
            let label = folder.file_name().unwrap().to_string_lossy().into_owned();
            // Leave the folder first, so that every measured navigation lists anew.
            probe
                .window
                .invoke_navigate(0, home.to_string_lossy().as_ref().into());
            probe.until("leave", |probe, turn| {
                turn.finished && !probe.window.get_left_loading()
            });
            probe.quiet();
            probe.reset_counters();
            let started = Instant::now();
            probe
                .window
                .invoke_navigate(0, folder.to_string_lossy().as_ref().into());
            let mut first_batch = None;
            let mut first_rows = None;
            let mut finished = None;
            let mut batches = 0;
            probe.until("navigate", |probe, turn| {
                batches += turn.batches;
                if turn.batches > 0 && first_batch.is_none() {
                    first_batch = Some(started.elapsed());
                }
                if first_rows.is_none() && probe.left_rows() > 1 {
                    first_rows = Some(started.elapsed());
                }
                if turn.finished && finished.is_none() {
                    finished = Some(started.elapsed());
                }
                finished.is_some() && !probe.window.get_left_loading()
            });
            let shown = started.elapsed();
            let rows = probe.left_rows();
            report.add(
                &format!("{label} navigate: first batch applied ms"),
                ms(first_batch.unwrap_or_default()),
            );
            report.add(
                &format!("{label} navigate: first rows in model ms"),
                ms(first_rows.unwrap_or_default()),
            );
            report.add(
                &format!("{label} navigate: ListingFinished applied ms"),
                ms(finished.unwrap_or_default()),
            );
            report.add(&format!("{label} navigate: drawn ms"), ms(shown));
            report.add(&format!("{label} navigate: UI apply ms"), ms(probe.apply));
            report.add(
                &format!("{label} navigate: UI refresh ms"),
                ms(probe.refresh),
            );
            report.add(&format!("{label} navigate: UI draw ms"), ms(probe.draw));
            report.add(
                &format!("{label} navigate: refreshes"),
                probe.refreshes as f64,
            );
            if round == 0 {
                println!("{label}: {rows} rows, {batches} entry batches applied");
                probe.breakdown(&format!("{label} navigate"));
            }
            probe.quiet();

            for (name, column) in [("size", 1), ("date", 2), ("ext", 3), ("name", 0)] {
                probe.reset_counters();
                let started = Instant::now();
                probe.window.invoke_sort(0, column);
                probe.until(name, |probe, turn| {
                    turn.finished && !probe.window.get_left_loading()
                });
                report.add(&format!("{label} sort {name} ms"), ms(started.elapsed()));
                if round == 0 {
                    probe.breakdown(&format!("{label} sort {name}"));
                }
                probe.quiet();
                // The second click sorts descending; the next column starts ascending.
                probe.reset_counters();
                let started = Instant::now();
                probe.window.invoke_sort(0, column);
                probe.until(name, |probe, turn| {
                    turn.finished && !probe.window.get_left_loading()
                });
                report.add(
                    &format!("{label} sort {name} descending ms"),
                    ms(started.elapsed()),
                );
                probe.quiet();
            }
            // Name ascending again for the selection and search steps.
            probe.window.invoke_sort(0, 0);
            probe.until("name", |probe, turn| {
                turn.finished && !probe.window.get_left_loading()
            });
            probe.quiet();

            for (name, operation, expect) in [
                ("select all", 0, true),
                ("invert", 4, false),
                ("invert again", 4, true),
                ("clear", 1, false),
            ] {
                probe.reset_counters();
                let started = Instant::now();
                probe.window.invoke_bulk_selection(0, operation);
                let callback = started.elapsed();
                // Row 1 is a file in every fixture (row 0 is the parent row).
                probe.until(name, |probe, turn| {
                    turn.selection && probe.row_selected(1) == expect
                });
                report.add(&format!("{label} {name}: callback ms"), ms(callback));
                report.add(
                    &format!("{label} {name}: applied ms"),
                    ms(started.elapsed()),
                );
                if round == 0 {
                    probe.breakdown(&format!("{label} {name}"));
                }
                probe.quiet();
            }

            for query in ["r", "re", "rep"] {
                probe.reset_counters();
                let started = Instant::now();
                probe.window.invoke_search_edited(0, query.into());
                probe.until(query, |_, turn| turn.search);
                report.add(
                    &format!("{label} quick search {query:?} ms"),
                    ms(started.elapsed()),
                );
                if round == 0 {
                    probe.breakdown(&format!("{label} quick search {query:?}"));
                }
                probe.quiet();
            }
            probe.window.invoke_search_edited(0, "".into());
            probe.quiet();
        }
        eprintln!("round {} of {rounds} done", round + 1);
    }
    report.print();
    let _ = commands.try_send(Command::Shutdown);
    let Probe {
        runtime, window, ..
    } = probe;
    window.hide().unwrap();
    drop(window);
    let _ = runtime.block_on(engine);
    runtime.shutdown_timeout(Duration::from_secs(2));
}

/// The parts of a native listing one by one, each the median of `rounds`: the bare
/// enumeration, the platform and installed-plugin providers, then the engine's work
/// on the rows (time text, collation keys, sort, index, a copy of the rows).
fn components(rounds: usize, folder: &Path) {
    use midnight_core::DirectoryService;
    let mut report = Report::default();
    let mut time = |label: &str, run: &mut dyn FnMut()| {
        for _ in 0..rounds {
            let started = Instant::now();
            run();
            report.add(label, ms(started.elapsed()));
        }
    };
    time("std read_dir + DirEntry::metadata", &mut || {
        let mut count = 0;
        for entry in fs::read_dir(folder).unwrap() {
            let entry = entry.unwrap();
            std::hint::black_box(entry.metadata().unwrap());
            count += 1;
        }
        std::hint::black_box(count);
    });
    let local = midnight_platform::LocalFileSystem::new(None);
    let installed = InstalledProvider::shared_composition(None).service();
    let cancel = Cancellation::default();
    let mut rows = Vec::new();
    for (label, service) in [
        (
            "platform provider read_directory",
            &local as &dyn DirectoryService,
        ),
        (
            "installed plugin read_directory",
            &*installed as &dyn DirectoryService,
        ),
    ] {
        time(label, &mut || {
            let mut entries = Vec::new();
            service
                .read_directory(folder, &cancel, &mut |entry| {
                    entries.push(entry);
                    true
                })
                .unwrap();
            rows = entries;
        });
    }
    println!("{} rows", rows.len());
    time("format modification times", &mut || {
        for entry in &rows {
            if let Some(modified) = entry.metadata.modified {
                std::hint::black_box(LocaleService::format_datetime(&local, modified));
            }
        }
    });
    let mut localized = rows.clone();
    for entry in &mut localized {
        entry.metadata.localized_modified = entry
            .metadata
            .modified
            .and_then(|modified| LocaleService::format_datetime(&local, modified));
    }
    time("clone the localized rows", &mut || {
        std::hint::black_box(localized.clone());
    });
    time("collation keys", &mut || {
        // Includes one copy of the rows (see "clone the localized rows").
        let mut keyed = localized.clone();
        prepare_sort_keys(&mut keyed, &local, &cancel).unwrap();
        std::hint::black_box(keyed);
    });
    let mut keyed = localized.clone();
    prepare_sort_keys(&mut keyed, &local, &cancel).unwrap();
    for column in [
        SortColumn::Name,
        SortColumn::Size,
        SortColumn::Modified,
        SortColumn::Type,
    ] {
        let mut copy = keyed.clone();
        time(
            &format!("sort {column:?} (rows already keyed)"),
            &mut || {
                sort_entries(
                    &mut copy,
                    SortSpec {
                        column,
                        descending: false,
                    },
                );
                copy.reverse();
            },
        );
    }
    time("DirectoryIndex::from_entries", &mut || {
        std::hint::black_box(DirectoryIndex::from_entries(&keyed));
    });
    // What the index's maps cost: copies of the paths, hashing, and growth.
    use std::{collections::HashMap, hash::BuildHasher};
    let paths: Vec<PathBuf> = keyed.iter().map(|entry| entry.path.clone()).collect();
    time("clone every path into a Vec", &mut || {
        std::hint::black_box(paths.clone());
    });
    let state = std::collections::hash_map::RandomState::new();
    time("SipHash every Path (Path::hash)", &mut || {
        for path in &paths {
            std::hint::black_box(state.hash_one(path));
        }
    });
    time("SipHash every path's bytes", &mut || {
        for path in &paths {
            std::hint::black_box(state.hash_one(path.as_os_str().as_encoded_bytes()));
        }
    });
    time(
        "HashMap<PathBuf, EntryId> presized, cloned keys",
        &mut || {
            let mut map = HashMap::with_capacity(paths.len());
            for (row, path) in paths.iter().enumerate() {
                map.insert(path.clone(), EntryId(row as u64));
            }
            std::hint::black_box(map);
        },
    );
    time("HashMap<PathBuf, EntryId> grown, cloned keys", &mut || {
        let mut map = HashMap::new();
        for (row, path) in paths.iter().enumerate() {
            map.insert(path.clone(), EntryId(row as u64));
        }
        std::hint::black_box(map);
    });
    time("HashMap<EntryId, usize> presized", &mut || {
        let mut map = HashMap::with_capacity(keyed.len());
        for (row, entry) in keyed.iter().enumerate() {
            map.insert(entry.id, row);
        }
        std::hint::black_box(map);
    });
    time("HashMap<EntryId, usize> grown", &mut || {
        let mut map = HashMap::new();
        for (row, entry) in keyed.iter().enumerate() {
            map.insert(entry.id, row);
        }
        std::hint::black_box(map);
    });
    report.print();
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.first().and_then(|mode| mode.to_str()) {
        Some("generate") if args.len() == 2 => generate(Path::new(&args[1])).unwrap(),
        Some("measure") if args.len() >= 3 => {
            let rounds = args[1].to_str().and_then(|r| r.parse().ok()).unwrap_or(5);
            let folders: Vec<PathBuf> = args[2..].iter().map(PathBuf::from).collect();
            measure(rounds, &folders);
        }
        Some("components") if args.len() == 3 => {
            let rounds = args[1].to_str().and_then(|r| r.parse().ok()).unwrap_or(5);
            components(rounds, Path::new(&args[2]));
        }
        _ => eprintln!(
            "usage: listing_probe generate <root> | listing_probe measure <rounds> <folder>...              | listing_probe components <rounds> <folder>"
        ),
    }
}
