//! Deterministic counts at the real FilePane/Rows boundary; no latency thresholds.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{FileRow, Presenter, install, refresh};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, ModelRc, ModelTracker};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct CountedRows {
    real: ModelRc<FileRow>,
    row_counts: Cell<usize>,
    reads: Cell<usize>,
    queried: RefCell<BTreeSet<usize>>,
}

#[derive(Debug)]
struct Counts {
    row_counts: usize,
    reads: usize,
    distinct: usize,
}

impl Counts {
    fn json(&self) -> Value {
        json!({"row_count_calls": self.row_counts, "row_data_calls": self.reads,
            "distinct_rows": self.distinct})
    }
}

impl CountedRows {
    fn new(real: ModelRc<FileRow>) -> Self {
        Self {
            real,
            row_counts: Cell::new(0),
            reads: Cell::new(0),
            queried: RefCell::new(BTreeSet::new()),
        }
    }

    fn take(&self) -> Counts {
        let distinct = self.queried.borrow().len();
        self.queried.borrow_mut().clear();
        Counts {
            row_counts: self.row_counts.replace(0),
            reads: self.reads.replace(0),
            distinct,
        }
    }
}

impl Model for CountedRows {
    type Data = FileRow;

    fn row_count(&self) -> usize {
        self.row_counts.set(self.row_counts.get() + 1);
        self.real.row_count()
    }

    fn row_data(&self, row: usize) -> Option<FileRow> {
        self.reads.set(self.reads.get() + 1);
        self.queried.borrow_mut().insert(row);
        self.real.row_data(row)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        self.real.model_tracker()
    }
}

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    directory: std::path::PathBuf,
}

impl Harness {
    fn new(count: usize, mode: PanelViewMode) -> Self {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let mut ui = ui::TestUi::with_clock(Some(clock));
        ui.resize(640, 480);
        let directory = std::env::current_dir()
            .unwrap()
            .join("_build/panel-scroll-costs-fixture");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            directory.clone(),
            directory.clone(),
        )));
        let mut preferences = UserPreferences {
            panel_view_modes: [mode; 2],
            thumbnails_enabled: true,
            thumbnails_network: true,
            ..Default::default()
        };
        for tab in &mut preferences.tabs {
            tab.paths = vec![directory.clone()];
        }
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(preferences))));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        let mut harness = Self {
            ui,
            presenter,
            commands,
            directory,
        };
        let initial: Vec<_> = harness
            .commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane, navigation, ..
                } => Some((*pane, *navigation)),
                _ => None,
            })
            .collect();
        assert_eq!(initial.len(), 2);
        for (pane, navigation) in initial {
            harness.complete(
                pane,
                navigation,
                if pane == PaneId::Left { count } else { 0 },
            );
        }
        refresh(&harness.ui.window, &harness.presenter.borrow());
        harness.ui.window.set_panel_zoomed(true);
        harness.draw();
        harness
    }

    fn complete(&self, pane: PaneId, navigation: NavigationId, count: usize) {
        let entries: Arc<[Entry]> = (0..count)
            .map(|index| {
                Entry::new(
                    EntryId(index as u64),
                    self.directory.join(format!("entry-{index:05}-文档.png")),
                    EntryKind::File,
                )
            })
            .collect();
        self.presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }

    fn reload(&mut self, count: usize) {
        self.commands.borrow_mut().clear();
        self.ui.window.invoke_action(0, 3);
        let navigation = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .expect("reload must use a real requested listing identity");
        self.complete(PaneId::Left, navigation, count);
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.window.invoke_scroll_selection(0, 1_000_000.0);
        self.draw();
    }

    fn draw(&mut self) {
        for _ in 0..3 {
            self.ui.window.window().request_redraw();
            self.ui.draw();
        }
    }

    fn content_pixels(&self) -> Vec<slint::Rgb8Pixel> {
        let left = self.ui.window.get_left_list_left().ceil().max(0.0) as usize;
        let top = self.ui.window.get_left_list_top().ceil().max(0.0) as usize;
        let width = (self.ui.window.get_left_list_width() - 24.0)
            .floor()
            .max(1.0) as usize;
        let height = self.ui.window.get_left_list_height().floor().max(1.0) as usize;
        assert!(left + width <= self.ui.width && top + height <= self.ui.height);
        (top..top + height)
            .flat_map(|y| {
                self.ui.pixels[y * self.ui.width + left..][..width]
                    .iter()
                    .copied()
            })
            .collect()
    }
}

fn sample(h: &mut Harness, count: usize) -> Vec<Counts> {
    let real = h.ui.window.get_left_rows();
    assert!(real.row_count() >= count);
    let counter = Rc::new(CountedRows::new(real.clone()));
    h.ui.window.set_left_rows(counter.clone().into());
    h.draw();
    let initial = counter.take();
    assert!(initial.reads > 0 && initial.distinct > 0);
    let before = h.content_pixels();
    let original = h.ui.window.get_left_list_content_y();
    h.draw();
    let stable = counter.take();
    assert_eq!(
        stable.reads, 0,
        "{count} entries: unchanged settled frames must reuse row projections: {stable:?}"
    );
    assert_eq!(h.ui.window.get_left_list_content_y(), original);
    assert_eq!(h.content_pixels(), before);
    let delta = h.ui.window.get_left_view_row_height() * 3.0;
    assert!(delta > 0.0);
    h.ui.window.invoke_scroll_selection(0, -delta);
    h.draw();
    let forward = counter.take();
    assert!(h.ui.window.get_left_list_content_y() < original - 1.0);
    assert_ne!(h.content_pixels(), before, "the actual FilePane must move");
    h.ui.window.invoke_scroll_selection(0, delta);
    h.draw();
    let reverse = counter.take();
    assert!((h.ui.window.get_left_list_content_y() - original).abs() < 0.01);
    assert_eq!(
        h.content_pixels(),
        before,
        "reversal restores rendered content"
    );
    // Presenter refresh downcasts its own model. The wrapper is only a render span.
    h.ui.window.set_left_rows(real);
    h.draw();
    vec![initial, stable, forward, reverse]
}

fn compare_counts(mode: PanelViewMode) {
    let mut h = Harness::new(100, mode);
    let small = sample(&mut h, 100);
    h.reload(5000);
    let large = sample(&mut h, 5000);
    let phases = ["initial", "stable_redraw", "forward", "reversal"];
    for ((phase, small), large) in phases.into_iter().zip(&small).zip(&large) {
        println!(
            "panel-scroll-costs {}",
            json!({"mode": format!("{mode:?}"), "phase": phase,
                "entries_100": small.json(), "entries_5000": large.json()})
        );
        // A 50-fold listing increase must not cause full-list query growth.
        // Allow extra boundary/layout queries; this is not a frame-time threshold.
        assert!(
            large.reads <= small.reads * 2 + 64,
            "{phase}: {small:?} → {large:?}"
        );
        assert!(
            large.distinct <= small.distinct * 2 + 16,
            "{phase}: {small:?} → {large:?}"
        );
    }
}

#[test]
fn detailed_real_model_query_scaling_and_reversal() {
    compare_counts(PanelViewMode::Detailed);
}

#[test]
fn brief_real_model_query_scaling_and_reversal() {
    compare_counts(PanelViewMode::Brief);
}

#[test]
fn thumbnail_real_model_query_scaling_and_reversal() {
    compare_counts(PanelViewMode::Thumbnails);
}

fn pixels(model: &ModelRc<FileRow>, row: usize) -> Option<Vec<u8>> {
    model
        .row_data(row)?
        .icon
        .to_rgba8()
        .map(|image| image.as_bytes().to_vec())
}

#[test]
fn cache_pressure_after_reversal_before_the_next_target_report() {
    let mut h = Harness::new(4097, PanelViewMode::Thumbnails);
    let model = h.ui.window.get_left_rows();
    let parent = model.row_count() - 4097;
    let first = [17, 83, 191, 255];
    let other = [91, 43, 7, 255];
    let mut identities = Vec::with_capacity(4096);
    // Fill the current record cap with tiny frames, independently of the byte cap.
    // Coverage acknowledges the requested 96px edge, not the 1x1 source dimensions.
    for index in 0..4096 {
        let row = parent + index;
        let targets = h
            .presenter
            .borrow_mut()
            .thumbnail_targets([(row, row + 1), (0, 0)], ["", ""]);
        assert_eq!(targets.len(), 1);
        identities.push(targets[0].identity.clone());
        assert!(h.presenter.borrow_mut().receive_thumbnail(
            &targets[0].identity,
            0,
            1,
            1,
            if index == 0 { &first } else { &other },
        ));
    }
    let first_identity = &identities[0];
    let resident_before = {
        let presenter = h.presenter.borrow();
        identities
            .iter()
            .filter(|identity| presenter.thumbnail_coverage(identity, 0) != 0)
            .count()
    };
    assert_eq!(resident_before, 4096);
    h.ui.window.invoke_scroll_selection(0, -1_000_000.0);
    h.draw();
    assert!(h.ui.window.get_left_list_content_y() < -1.0);
    assert!(h.ui.window.get_left_visible_row_start() as usize > parent);
    let last = parent + 4095;
    let targets = h
        .presenter
        .borrow_mut()
        .thumbnail_targets([(last, last + 2), (0, 0)], ["", ""]);
    assert_eq!(targets.len(), 2);
    assert_eq!(pixels(&model, last).as_deref(), Some(other.as_slice()));
    h.ui.window.invoke_scroll_selection(0, 1_000_000.0);
    h.draw();
    assert_eq!(h.ui.window.get_left_list_content_y(), 0.0);
    assert!(h.ui.window.get_left_visible_row_start() as usize <= parent);
    assert!(h.ui.window.get_left_visible_row_end() as usize > parent);
    assert_eq!(pixels(&model, parent).as_deref(), Some(first.as_slice()));
    assert_eq!(
        h.presenter.borrow().thumbnail_coverage(first_identity, 0),
        96
    );
    // Deliberately leave the last target report unchanged while completing C.
    assert!(h.presenter.borrow_mut().receive_thumbnail(
        &targets[1].identity,
        0,
        1,
        1,
        &[201, 37, 51, 255],
    ));
    let coverage = h.presenter.borrow().thumbnail_coverage(first_identity, 0);
    let returned = pixels(&model, parent);
    println!(
        "panel-scroll-retention {}",
        json!({"resident_records_before": resident_before, "decoded_bytes_before": resident_before * 4,
            "reported_visible_rows": [last, last + 2], "actual_returned_row": parent,
            "returned_coverage_before": 96, "returned_coverage_after": coverage,
            "returned_pixels_retained": returned.as_deref() == Some(first.as_slice())})
    );
    if coverage != 0 {
        assert_eq!(returned.as_deref(), Some(first.as_slice()));
    }
    assert_eq!(pixels(&model, last).as_deref(), Some(other.as_slice()));
    assert_eq!(
        pixels(&model, last + 1).as_deref(),
        Some([201, 37, 51, 255].as_slice())
    );
    // Evicting the returned row is an observation, not a required policy outcome.
    // This model readback does not prove that an already painted row visibly blinked.
}
