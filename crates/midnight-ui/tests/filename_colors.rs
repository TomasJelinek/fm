#[path = "support/popup_scroll.rs"]
mod popup_scroll;
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    model::{ModelChangeListener, ModelChangeListenerBox},
};
use midnight_core::{
    AcceptedPreferences, Command, DirectoryIndex, Entry, EntryId, EntryKind, Event,
    FilenameColorsRequest, FilenameColorsResult, FilenameStyle, PaneId, PreferenceTheme, RgbColor,
    SelectionRevision, TreeCommand, TreeDisclosure, TreeDisplayRow, TreeEvent, TreeLayoutRevision,
    TreeProjection, TreePublication, TreeRevision, TreeRowId, TreeSelectionSnapshot, TreeSessionId,
    TreeViewStamp, UserPreferences,
};
use midnight_ui::{ColorOptionsView, FileRow, FilenameColorPresentation, Presenter, Theme};
use slint::{
    Brush, Color, ComponentHandle, Model, VecModel,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    sync::Arc,
};

fn contrast(first: Color, second: Color) -> f64 {
    let luminance = |color: Color| {
        let channels = [color.red(), color.green(), color.blue()];
        channels
            .into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(channel, weight)| {
                let value = f64::from(channel) / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum::<f64>()
    };
    let (a, b) = (luminance(first), luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

// At 11px the font's strokes need not cover any pixel completely. Recognize
// the expected ink composited over its actual background, rather than requiring
// a fully opaque glyph pixel. The old bright highlight follows a different RGB
// segment and cannot satisfy this test on the light custom background.
fn has_ink(pixel: slint::Rgb8Pixel, foreground: Color, background: Color) -> bool {
    let pixel = [pixel.r, pixel.g, pixel.b].map(f64::from);
    let foreground = [foreground.red(), foreground.green(), foreground.blue()].map(f64::from);
    let background = [background.red(), background.green(), background.blue()].map(f64::from);
    let direction = std::array::from_fn::<_, 3, _>(|index| foreground[index] - background[index]);
    let squared: f64 = direction.iter().map(|value| value * value).sum();
    if squared < 1.0 {
        return false;
    }
    let coverage: f64 = (0..3)
        .map(|index| (pixel[index] - background[index]) * direction[index])
        .sum::<f64>()
        / squared;
    (0.65..=1.01).contains(&coverage)
        && (0..3).all(|index| {
            (pixel[index] - background[index] - coverage * direction[index]).abs() <= 3.0
        })
}

const INLINE_TREE_PDF_NAMES: [&str; 3] = [
    "|-- Child displayed.txt",
    "|   +-- Grandchild displayed.txt",
    "Root displayed.txt",
];

struct InlineTreeColorFixture {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    send: Rc<dyn Fn(Command) -> bool>,
    source: Arc<[Entry]>,
    view: Arc<TreeProjection>,
}

impl InlineTreeColorFixture {
    fn new() -> Self {
        let mut ui = ui::TestUi::new();
        let root_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join("_build/verification/viewer-refresh-20261117/ui/tree-colors/inline-root");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            root_path.clone(),
            root_path.clone(),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        });
        midnight_ui::install(&ui.window, presenter.clone(), send.clone());
        let mut preferences = UserPreferences::default();
        for tabs in &mut preferences.tabs {
            tabs.paths = vec![root_path.clone()];
        }
        presenter.borrow_mut().apply_with_sender(
            Event::PreferencesLoaded(Ok(Box::new(AcceptedPreferences {
                preferences,
                comparators: midnight_core::FileComparatorSnapshot::default(),
            }))),
            send.as_ref(),
        );
        let navigation = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .expect("accepted preferences must request the left root listing");
        let mut root_pdf = Entry::new(EntryId(84), root_path.join("root.pdf"), EntryKind::File);
        root_pdf.metadata.size = Some(12_288);
        let source: Arc<[Entry]> = vec![
            Entry::new(EntryId(42), root_path.join("Reports"), EntryKind::Directory),
            root_pdf,
            Entry::new(
                EntryId(126),
                root_path.join("Unmatched payloads"),
                EntryKind::Directory,
            ),
        ]
        .into();
        let index = Arc::new(DirectoryIndex::from_entries(&source));
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: source.clone(),
            index: index.clone(),
        });
        midnight_ui::refresh(&ui.window, &presenter.borrow());
        ui.draw();
        let identity = ui
            .window
            .get_left_rows()
            .iter()
            .find(|row| row.name == "Reports")
            .unwrap()
            .identity;
        ui.window.invoke_tree_disclosure(0, identity);
        let (root, selection_revision) = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Tree(command) => match command.as_ref() {
                    TreeCommand::Enable {
                        root,
                        selection_revision,
                        ..
                    } => Some((root.clone(), *selection_revision)),
                    _ => None,
                },
                _ => None,
            })
            .expect("installed disclosure must capture the accepted root");
        assert!(Arc::ptr_eq(&root.entries, &source));
        assert!(Arc::ptr_eq(&root.index, &index));
        let mut reports = source[0].clone();
        reports.id = EntryId(101);
        let mut child = Entry::new(
            EntryId(102),
            reports.path.join("child.pdf"),
            EntryKind::File,
        );
        child.display_name = INLINE_TREE_PDF_NAMES[0].into();
        child.metadata.size = Some(4_096);
        let nested = Entry::new(
            EntryId(103),
            reports.path.join("Nested"),
            EntryKind::Directory,
        );
        let mut grandchild = Entry::new(
            EntryId(104),
            nested.path.join("grandchild.PDF"),
            EntryKind::File,
        );
        grandchild.display_name = INLINE_TREE_PDF_NAMES[1].into();
        grandchild.metadata.size = Some(16_384);
        let mut root_pdf = source[1].clone();
        root_pdf.id = EntryId(105);
        root_pdf.display_name = INLINE_TREE_PDF_NAMES[2].into();
        let row = |id, entry: Entry, parent, depth, original_root_entry| {
            let directory = entry.kind == EntryKind::Directory;
            TreeDisplayRow {
                id: TreeRowId(id),
                entry,
                parent,
                depth,
                disclosure: if directory {
                    TreeDisclosure::Expanded
                } else {
                    TreeDisclosure::None
                },
                expanded: directory,
                selectable: true,
                navigable: directory,
                context_only: false,
                error: None,
                error_details: None,
                original_root_entry,
                prefix_end: 0,
                search_match: false,
            }
        };
        let mut rows = vec![
            row(1_001, reports, None, 0, Some(EntryId(42))),
            row(1_002, child, Some(TreeRowId(1_001)), 1, None),
            row(1_003, nested, Some(TreeRowId(1_001)), 1, None),
            row(1_004, grandchild, Some(TreeRowId(1_003)), 2, None),
            row(1_005, root_pdf, None, 0, Some(EntryId(84))),
        ];
        let mut unmatched = source[2].clone();
        unmatched.id = EntryId(106);
        let unmatched_path = unmatched.path.clone();
        rows.push(row(1_006, unmatched, None, 0, Some(EntryId(126))));
        for position in 0..96 {
            let mut entry = Entry::new(
                EntryId(200 + position),
                unmatched_path.join(format!("payload-{position:03}.dat")),
                EntryKind::File,
            );
            entry.metadata.size = Some(1_024 + position);
            rows.push(row(
                2_000 + position,
                entry,
                Some(TreeRowId(1_006)),
                1,
                None,
            ));
        }
        let view = Arc::new(
            TreeProjection::new(
                root,
                TreeViewStamp {
                    pane: PaneId::Left,
                    navigation,
                    session: TreeSessionId(310),
                    revision: TreeRevision(1),
                },
                TreeLayoutRevision(1),
                rows,
            )
            .unwrap(),
        );
        let selection = Arc::new(
            TreeSelectionSnapshot::from_bits(view.clone(), vec![0; view.rows.len().div_ceil(64)])
                .unwrap(),
        );
        presenter.borrow_mut().apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(
                TreePublication::new(view.clone(), selection_revision, selection).unwrap(),
            ))),
            send.as_ref(),
        );
        midnight_ui::refresh(&ui.window, &presenter.borrow());
        ui.draw();
        Self {
            ui,
            presenter,
            commands,
            send,
            source,
            view,
        }
    }

    fn row_index(&self, name: &str) -> usize {
        self.ui
            .window
            .get_left_rows()
            .iter()
            .position(|row| row.name == name)
            .expect("published tree filename must have a visible model row")
    }

    fn request(&self) -> FilenameColorsRequest {
        self.presenter
            .borrow_mut()
            .flush_filename_colors(self.send.as_ref());
        self.commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::MatchFilenameColors(request) if request.pane == PaneId::Left => {
                    Some(request.clone())
                }
                _ => None,
            })
            .expect("the accepted tree snapshot must request backend filename colors")
    }

    fn receive_colors(&mut self, request: FilenameColorsRequest) {
        let pdf = FilenameStyle {
            foreground: Some(RgbColor(239, 90, 90)),
            background: None,
        };
        self.presenter.borrow_mut().apply_with_sender(
            Event::FilenameColorsReady {
                result: Ok(Arc::new(FilenameColorsResult {
                    request: request.clone(),
                    // Synthetic entry IDs differ from both TreeRowId and root aliases.
                    styles: HashMap::from([
                        (EntryId(102), pdf),
                        (EntryId(104), pdf),
                        (EntryId(105), pdf),
                    ]),
                })),
                request,
            },
            self.send.as_ref(),
        );
        midnight_ui::refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }

    fn publish_metadata(&mut self) {
        let mut rows = self.view.rows.to_vec();
        rows[3].entry.metadata.size = Some(32_768);
        self.view = Arc::new(
            TreeProjection::new(
                self.view.root.clone(),
                TreeViewStamp {
                    revision: TreeRevision(2),
                    ..self.view.stamp
                },
                self.view.layout_revision,
                rows,
            )
            .unwrap(),
        );
        let selection = Arc::new(
            TreeSelectionSnapshot::from_bits(
                self.view.clone(),
                vec![0; self.view.rows.len().div_ceil(64)],
            )
            .unwrap(),
        );
        self.presenter.borrow_mut().apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(
                TreePublication::new(self.view.clone(), SelectionRevision(0), selection).unwrap(),
            ))),
            self.send.as_ref(),
        );
        midnight_ui::refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }
}

#[derive(Default)]
struct RowNotifications {
    changed: RefCell<Vec<usize>>,
    structural: Cell<usize>,
}

impl ModelChangeListener for RowNotifications {
    fn row_changed(self: Pin<&Self>, row: usize) {
        self.changed.borrow_mut().push(row);
    }
    fn row_added(self: Pin<&Self>, _: usize, _: usize) {
        self.structural.set(self.structural.get() + 1);
    }
    fn row_removed(self: Pin<&Self>, _: usize, _: usize) {
        self.structural.set(self.structural.get() + 1);
    }
    fn reset(self: Pin<&Self>) {
        self.structural.set(self.structural.get() + 1);
    }
}

fn filename_text(item: ItemRc, name: &str) -> Option<(ItemRc, Brush)> {
    if item.is_visible() {
        if let Some(text) = item.downcast::<SimpleText>() {
            let text = text.as_pin_ref();
            if text.text() == name {
                return Some((item.clone(), text.color()));
            }
        }
        if let Some(text) = item.downcast::<ComplexText>() {
            let text = text.as_pin_ref();
            if text.text() == name {
                return Some((item.clone(), text.color()));
            }
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = filename_text(item, name) {
            return Some(found);
        }
    }
    None
}

fn assert_tree_pdf_ink(test: &ui::TestUi, index: usize) {
    let row = test.window.get_left_rows().row_data(index).unwrap();
    let pdf = Color::from_rgb_u8(239, 90, 90);
    assert!(row.color_rule && row.custom_foreground);
    assert_eq!(row.foreground, pdf);
    assert!(!row.custom_background);
    let presentation = test.window.global::<FilenameColorPresentation>();
    let background = presentation.invoke_background(
        false,
        Color::default(),
        row.cursor,
        test.window.get_active_pane() == 0,
        row.selected,
        !index.is_multiple_of(2),
    );
    let ink = presentation.invoke_foreground(true, pdf, background, row.selected, false);
    assert!(contrast(ink, background) >= 4.5);
    let delegate = ui::root_item(&test.window, row.name.as_str(), AccessibleRole::ListItem);
    let (text, brush) = filename_text(delegate, row.name.as_str()).unwrap();
    assert_eq!(brush, Brush::SolidColor(ink), "delegate for {}", row.name);
    let rect = text.geometry();
    let origin = text.map_to_native_window(rect.origin);
    let painted = ((origin.y + 2.0).max(0.0) as usize
        ..(origin.y + rect.size.height - 2.0).min(test.height as f32) as usize)
        .any(|y| {
            (origin.x.max(0.0) as usize
                ..(origin.x + rect.size.width).min(test.width as f32) as usize)
                .any(|x| has_ink(test.pixels[y * test.width + x], ink, background))
        });
    assert!(painted, "actual descendant filename ink for {}", row.name);
}

#[test]
fn inline_tree_pdf_results_repaint_visible_descendants_without_rebuilding_rows() {
    let mut fixture = InlineTreeColorFixture::new();
    let rows = fixture.ui.window.get_left_rows();
    let pdf_rows = INLINE_TREE_PDF_NAMES.map(|name| fixture.row_index(name));
    let identities = pdf_rows.map(|index| rows.row_data(index).unwrap().identity);
    for index in pdf_rows {
        assert!(!rows.row_data(index).unwrap().color_rule);
    }
    let notifications = ModelChangeListenerBox::new(RowNotifications::default());
    rows.model_tracker()
        .attach_peer(notifications.as_ref().model_peer());
    let request = fixture.request();
    let view = request.tree.as_ref().unwrap();
    assert!(Arc::ptr_eq(view, &fixture.view));
    assert!(Arc::ptr_eq(&request.source, &fixture.source));
    assert!(Arc::ptr_eq(&request.source, &view.root.entries));
    assert_eq!(view.stamp.pane, request.pane);
    assert_eq!(view.stamp.navigation, view.root.navigation);
    assert_eq!(
        request.rules.as_ref(),
        midnight_core::default_filename_colors()
    );

    // Keep a real delegate pressed while a backend result replaces its row data.
    // A model reset would lose the release and leave this descendant unfocused.
    let position = ui::root_position(
        &fixture.ui.window,
        INLINE_TREE_PDF_NAMES[1],
        AccessibleRole::ListItem,
    );
    fixture
        .ui
        .window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    fixture
        .ui
        .window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
    notifications.changed.borrow_mut().clear();
    fixture.receive_colors(request.clone());
    assert!(std::ptr::eq(
        rows.as_any(),
        fixture.ui.window.get_left_rows().as_any()
    ));
    assert_eq!(notifications.structural.get(), 0);
    let first = fixture.ui.window.get_left_visible_row_start().max(0) as usize;
    let end = fixture.ui.window.get_left_visible_row_end().max(0) as usize;
    assert!(rows.row_count() > end, "loaded tree must extend offscreen");
    let changed = notifications.changed.borrow();
    assert!(pdf_rows.iter().all(|index| changed.contains(index)));
    assert!(changed.iter().all(|index| (first..end).contains(index)));
    assert!((end..rows.row_count()).all(|index| !changed.contains(&index)));
    drop(changed);
    for ((index, identity), depth) in pdf_rows.into_iter().zip(&identities).zip([1, 2, 0]) {
        let row = rows.row_data(index).unwrap();
        assert_eq!(&row.identity, identity);
        assert_eq!(row.tree_depth, depth);
        assert_tree_pdf_ink(&fixture.ui, index);
    }
    fixture
        .ui
        .window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    fixture.ui.draw();
    assert!(rows.row_data(pdf_rows[1]).unwrap().cursor);
    fixture.commands.borrow_mut().clear();
    fixture
        .presenter
        .borrow_mut()
        .flush_filename_colors(fixture.send.as_ref());
    assert!(
        !fixture
            .commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::MatchFilenameColors(_)))
    );

    // Metadata publications retain the root and layout, but need their own color result.
    let previous = fixture.view.clone();
    fixture.publish_metadata();
    let current = fixture.request();
    assert_ne!(current.id, request.id);
    assert!(Arc::ptr_eq(&current.source, &request.source));
    assert!(Arc::ptr_eq(&current.rules, &request.rules));
    assert!(!Arc::ptr_eq(&previous, &fixture.view));
    assert_eq!(previous.layout_revision, fixture.view.layout_revision);
    assert!(previous.same_structure(&fixture.view));
    assert!(Arc::ptr_eq(current.tree.as_ref().unwrap(), &fixture.view));
    fixture.receive_colors(request);
    for index in pdf_rows {
        assert!(!rows.row_data(index).unwrap().color_rule);
    }
    let identities = pdf_rows.map(|index| rows.row_data(index).unwrap().identity);
    notifications.changed.borrow_mut().clear();
    fixture.receive_colors(current);
    let first = fixture.ui.window.get_left_visible_row_start().max(0) as usize;
    let end = fixture.ui.window.get_left_visible_row_end().max(0) as usize;
    assert!(rows.row_count() > end, "loaded tree must remain offscreen");
    let changed = notifications.changed.borrow();
    assert!(pdf_rows.iter().all(|index| changed.contains(index)));
    assert!(changed.iter().all(|index| (first..end).contains(index)));
    assert!((end..rows.row_count()).all(|index| !changed.contains(&index)));
    drop(changed);
    for (index, identity) in pdf_rows.into_iter().zip(identities) {
        assert_eq!(rows.row_data(index).unwrap().identity, identity);
        assert_tree_pdf_ink(&fixture.ui, index);
    }
    assert_eq!(notifications.structural.get(), 0);

    let folder = fixture.row_index("Reports");
    for theme in [PreferenceTheme::Dark, PreferenceTheme::Light] {
        for active in [true, false] {
            for state in 0..4 {
                let selected = state >= 2;
                let cursor = state == 1 || state == 3;
                let index = pdf_rows[1];
                if rows.row_data(index).unwrap().selected != selected {
                    fixture
                        .ui
                        .window
                        .invoke_row_click(0, index as i32, true, false);
                }
                fixture.ui.window.invoke_row_click(
                    0,
                    if cursor { index } else { folder } as i32,
                    false,
                    false,
                );
                fixture.ui.window.invoke_apply_preferences(theme.index(), 0);
                fixture
                    .ui
                    .window
                    .set_active_pane(if active { 0 } else { 1 });
                fixture.ui.draw();
                let row = rows.row_data(index).unwrap();
                assert_eq!(row.selected, selected);
                assert_eq!(row.cursor, cursor);
                for index in pdf_rows {
                    assert_tree_pdf_ink(&fixture.ui, index);
                }
                render::snapshot(
                    &fixture.ui,
                    &format!(
                        "inline-tree-pdf-theme-{}-state-{state}-active-{active}-revision-{}-layout-{}",
                        theme.index(),
                        fixture.view.stamp.revision.0,
                        fixture.view.layout_revision.0,
                    ),
                );
            }
        }
    }
    assert_eq!(notifications.structural.get(), 0);
}

#[test]
fn default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state() {
    let mut test = ui::TestUi::new();
    let rules = midnight_core::default_filename_colors();
    let names = [
        "Source main.rs",
        "Video movie.mp4",
        "PDF report.pdf",
        "Document letter.docx",
        "Image photo.jpg",
        "Executable app.exe",
        "Spreadsheet table.csv",
        "Audio music.flac",
        "Archive backup.zip",
        "Presentation slides.pptx",
        "Plain text README.md",
        "Font type.ttf",
    ];
    assert_eq!(rules.len(), names.len());
    let window = test.window.clone_strong();
    let presentation = window.global::<FilenameColorPresentation>();
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        window.invoke_apply_preferences(theme, 0);
        for active in [true, false] {
            window.set_active_pane(if active { 0 } else { 1 });
            for state in 0..4 {
                let cursor = state == 1 || state == 3;
                let selected = state >= 2;
                let rows: Vec<_> = rules
                    .iter()
                    .zip(names)
                    .enumerate()
                    .map(|(index, (rule, name))| {
                        let rgb = rule.style.foreground.unwrap();
                        FileRow {
                            identity: index.to_string().into(),
                            name: name.into(),
                            color_rule: true,
                            custom_foreground: true,
                            foreground: Color::from_rgb_u8(rgb.0, rgb.1, rgb.2),
                            cursor,
                            selected,
                            ..Default::default()
                        }
                    })
                    .collect();
                window.set_left_rows(Rc::new(VecModel::from(rows)).into());
                test.draw();
                for (index, (rule, name)) in rules.iter().zip(names).enumerate() {
                    let rgb = rule.style.foreground.unwrap();
                    let base = Color::from_rgb_u8(rgb.0, rgb.1, rgb.2);
                    for alternate in [false, true] {
                        let background = presentation.invoke_background(
                            false,
                            Color::default(),
                            cursor,
                            active,
                            selected,
                            alternate,
                        );
                        let ink =
                            presentation.invoke_foreground(true, base, background, selected, false);
                        assert!(
                            contrast(ink, background) >= 4.5,
                            "{name}: theme {theme}, state {state}, active {active}, alternate {alternate}"
                        );
                        if matches!(index, 3 | 10)
                            && state == 0
                            && !window.global::<Theme>().get_light()
                        {
                            assert!(
                                contrast(ink, background) >= 7.0,
                                "ordinary document names must stay bright in dark theme {theme}"
                            );
                        }
                        // Check every category against both stripes, then inspect
                        // the ink on the stripe used by its rendered fixture row.
                        if alternate != (index % 2 != 0) {
                            continue;
                        }
                        let row = ui::root_item(&window, name, AccessibleRole::ListItem);
                        let rect = row.geometry();
                        let origin = row.map_to_native_window(rect.origin);
                        // Inspect the filename region, excluding artwork and focus edges.
                        let found = ((origin.y + 2.0) as usize
                            ..(origin.y + rect.size.height - 2.0) as usize)
                            .any(|y| {
                                ((origin.x + 64.0) as usize..(origin.x + 230.0) as usize).any(|x| {
                                    has_ink(test.pixels[y * test.width + x], ink, background)
                                })
                            });
                        assert!(
                            found,
                            "actual filename ink: {name}, theme {theme}, state {state}, active {active}"
                        );
                    }
                }
                if (active && state == 0 && matches!(theme, 0 | 2 | 17 | 21..=25))
                    || (matches!(theme, 0 | 23..=25) && !active && state == 3)
                {
                    render::snapshot(
                        &test,
                        &format!("default-palette-theme-{theme}-state-{state}-active-{active}"),
                    );
                }
            }
        }
    }
}

#[test]
fn all_themes_states_and_views_render_readable_filename_colors_with_preserved_cues() {
    let mut test = ui::TestUi::new();
    let foreground = Color::from_rgb_u8(170, 90, 230);
    let background = Color::from_rgb_u8(230, 220, 235);
    let rows: Vec<_> = (0..4)
        .map(|state| FileRow {
            identity: state.to_string().into(),
            name: format!("Color file{state}.txt").into(),
            color_rule: true,
            custom_foreground: true,
            foreground,
            custom_background: true,
            background,
            cursor: state == 1 || state == 3,
            selected: state >= 2,
            prefix_end: if state == 0 { 4 } else { 0 },
            size: "12.4 GB".into(),
            size_state: 1,
            size_running: true,
            ..Default::default()
        })
        .collect();
    test.window
        .set_left_rows(Rc::new(VecModel::from(rows)).into());
    let window = test.window.clone_strong();
    for theme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        test.window.invoke_apply_preferences(theme, 0);
        let presentation = window.global::<FilenameColorPresentation>();
        for base in [
            foreground,
            Color::from_rgb_u8(128, 128, 128),
            Color::from_rgb_u8(0, 0, 0),
            Color::from_rgb_u8(255, 255, 255),
        ] {
            for state in 0..4 {
                for active in [true, false] {
                    let bg = presentation.invoke_background(
                        true,
                        background,
                        state == 1 || state == 3,
                        active,
                        state >= 2,
                        false,
                    );
                    let fg = presentation.invoke_foreground(true, base, bg, state >= 2, false);
                    assert!(
                        contrast(fg, bg) >= 4.5,
                        "theme {theme}, state {state}: {fg:?} on {bg:?}"
                    );
                    if state > 0 {
                        assert_ne!(bg, background, "theme state must retain its background cue");
                    }
                }
            }
        }
        for mode in 0..3 {
            test.window.set_left_view_mode(mode);
            test.draw();
            let row = ui::root_item(&test.window, "Color file0.txt", AccessibleRole::ListItem);
            let rect = row.geometry();
            let origin = row.map_to_native_window(rect.origin);
            let count = |color: Color| {
                (origin.y.max(0.0) as usize
                    ..(origin.y + rect.size.height).min(test.height as f32) as usize)
                    .flat_map(|y| {
                        (origin.x.max(0.0) as usize
                            ..(origin.x + rect.size.width).min(test.width as f32) as usize)
                            .map(move |x| (x, y))
                    })
                    .filter(|&(x, y)| {
                        let pixel = test.pixels[y * test.width + x];
                        (pixel.r, pixel.g, pixel.b) == (color.red(), color.green(), color.blue())
                    })
                    .count()
            };
            assert!(
                count(background) > 20,
                "theme {theme}, mode {mode}: custom filename background must paint"
            );
            let search_color = presentation
                .invoke_readable(test.window.global::<Theme>().get_search_match(), background);
            render::snapshot(&test, &format!("filename-colors-theme-{theme}-mode-{mode}"));
            let search_pixels = (origin.y as usize..(origin.y + rect.size.height) as usize)
                .flat_map(|y| {
                    (origin.x as usize..(origin.x + rect.size.width) as usize).map(move |x| (x, y))
                })
                .filter(|&(x, y)| {
                    has_ink(test.pixels[y * test.width + x], search_color, background)
                })
                .count();
            assert!(
                search_pixels > 0,
                "theme {theme}, mode {mode}: prefix highlight must remain readable on the custom background; expected={search_color:?}, unadjusted={}, text={}",
                count(test.window.global::<Theme>().get_search_match()),
                count(foreground)
            );
            if mode == 0 {
                assert!(
                    count(test.window.global::<Theme>().get_pending_size()) > 0,
                    "pending size retains its semantic color"
                );
                let ordinary =
                    ui::root_item(&test.window, "Color file1.txt", AccessibleRole::ListItem);
                let geometry = ordinary.geometry();
                let point = ordinary.map_to_native_window(geometry.origin);
                let bg = presentation.invoke_background(true, background, true, true, false, false);
                let fg = presentation.invoke_foreground(true, foreground, bg, false, false);
                let first_glyph = (point.x as usize..(point.x + geometry.size.width) as usize)
                    .find(|&x| {
                        (point.y as usize..(point.y + geometry.size.height) as usize).any(|y| {
                            let pixel = test.pixels[y * test.width + x];
                            has_ink(pixel, fg, bg)
                        })
                    })
                    .expect("colored filename must render");
                assert!(
                    first_glyph < point.x as usize + 64,
                    "detailed filenames must remain aligned to the left"
                );
            }
            let combined = ui::root_item(&test.window, "Color file3.txt", AccessibleRole::ListItem);
            let geometry = combined.geometry();
            let point = combined.map_to_native_window(geometry.origin);
            let edge = test.window.global::<Theme>().get_cursor_edge();
            let pixel = test.pixel(point.x + geometry.size.width / 2.0, point.y + 0.5);
            assert_eq!(
                (pixel.r, pixel.g, pixel.b),
                (edge.red(), edge.green(), edge.blue()),
                "combined focus/selection cue must paint above filename backgrounds"
            );
            render::snapshot(&test, &format!("filename-colors-theme-{theme}-mode-{mode}"));
        }
    }
}

#[test]
fn colors_topic_edits_reorders_and_waits_for_backend_validation_before_saving() {
    let mut test = ui::TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            midnight_core::AcceptedPreferences {
                preferences: UserPreferences {
                    // Exercise creation and reordering from an explicitly cleared preset list.
                    filename_colors: Vec::new().into(),
                    ..UserPreferences::default()
                },
                comparators: midnight_core::FileComparatorSnapshot::default(),
            },
        ))));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        captured.borrow_mut().push(command);
        true
    });
    midnight_ui::install(&test.window, presenter.clone(), send.clone());
    commands.borrow_mut().clear();
    test.window.invoke_show_options();
    test.draw();
    popup_scroll::click(&mut test, "Colors", AccessibleRole::Button);
    test.draw();
    assert_eq!(test.window.get_options_topic_index(), 8);
    popup_scroll::click(&mut test, "Add rule", AccessibleRole::Button);
    test.draw();
    let window = test.window.clone_strong();
    let view = window.global::<ColorOptionsView>();
    let mut draft = view.get_draft();
    draft.expression = "*.rs".into();
    draft.foreground = "#b749de".into();
    view.invoke_edit(draft);
    popup_scroll::click(&mut test, "Add rule", AccessibleRole::Button);
    test.draw();
    let mut draft = view.get_draft();
    draft.expression = "文档*".into();
    draft.background = "#ece5cd".into();
    view.invoke_edit(draft);
    popup_scroll::click(&mut test, "Move up", AccessibleRole::Button);
    test.draw();
    assert!(view.get_labels().row_data(0).unwrap().contains("文档"));
    assert_eq!(view.get_selected(), 0);
    popup_scroll::click(&mut test, "Apply colors", AccessibleRole::Button);
    test.draw();
    assert!(view.get_busy());
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.rules[0].expression, "文档*");
    presenter.borrow_mut().apply_with_sender(
        Event::FilenameColorsValidated {
            request,
            result: Err("Invalid expression".into()),
        },
        send.as_ref(),
    );
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(view.get_error().contains("Invalid expression"));
    assert!(!view.get_busy(), "a failed validation must allow retry");
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    commands.borrow_mut().clear();
    popup_scroll::click(&mut test, "Apply colors", AccessibleRole::Button);
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ValidateFilenameColors(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    presenter.borrow_mut().apply_with_sender(
        Event::FilenameColorsValidated {
            request,
            result: Ok(()),
        },
        send.as_ref(),
    );
    midnight_ui::refresh(&test.window, &presenter.borrow());
    test.draw();
    assert!(!view.get_busy());
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
    assert_eq!(view.get_labels().row_count(), 2);
    assert!(view.get_labels().row_data(0).unwrap().contains("文档*"));
    render::snapshot(&test, "filename-colors-options");
    popup_scroll::click(&mut test, "Remove rule", AccessibleRole::Button);
    test.draw();
    assert_eq!(view.get_labels().row_count(), 1);
    popup_scroll::click(&mut test, "Discard changes", AccessibleRole::Button);
    test.draw();
    assert_eq!(view.get_labels().row_count(), 2);
    popup_scroll::click(&mut test, "OK", AccessibleRole::Button);
    let saved = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::SavePreferences { preferences, .. } => Some(preferences.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(saved.filename_colors.len(), 2);
    assert_eq!(saved.filename_colors[0].expression, "文档*");
}
