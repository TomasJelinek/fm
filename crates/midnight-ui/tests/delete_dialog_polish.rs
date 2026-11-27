//! The Delete confirmations as the installed presenter shows them: the question first,
//! the warning under it and the details after, sized to their content, with padded
//! actions clear of the resize grip, at every density and font size; rendered at the
//! densities, font sizes, themes and scales they are reviewed at.
use crate::support::modeless::{self, bitmap};
#[path = "support/embedded_form.rs"]
mod embedded_form;
#[path = "support/owned_windows.rs"]
mod owned_windows;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, ComplexText, SimpleText},
    lengths::{LogicalPoint, LogicalSize},
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{MainWindow, OperationPresentation, Presenter, install, refresh};
use slint::platform::{
    Key, WindowAdapter, WindowEvent,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

/// The folder every fixture item is in.
fn folder() -> PathBuf {
    PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        .join("Data")
        .join("Downloads")
}

fn item(name: &str, kind: EntryKind, size: Option<u64>) -> MutationItem {
    MutationItem {
        path: folder().join(name),
        kind,
        metadata: EntryMetadata {
            size,
            localized_modified: Some(LocalizedDateTime {
                date: "23.11.2026".into(),
                time: "14:53:51".into(),
            }),
            ..EntryMetadata::default()
        },
    }
}

struct Fixture {
    ui: owned_windows::Windows,
    window: MainWindow,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

impl Fixture {
    fn new() -> Self {
        let (ui, windows) = owned_windows::with_buffer(RepaintBufferType::NewBuffer);
        let path = folder();
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sink = commands.clone();
        install(
            &ui.window,
            state.clone(),
            Rc::new(move |command| {
                sink.borrow_mut().push(command);
                true
            }),
        );
        let navigation = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .unwrap();
        let entries = vec![Entry::new(
            EntryId(1),
            path.join("experiments.rar"),
            EntryKind::File,
        )];
        state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries: entries.into(),
        });
        state
            .borrow_mut()
            .apply(Event::LocaleLoaded(Ok(NumberLocale {
                decimal_separator: ",".into(),
                group_separator: "\u{a0}".into(),
                grouping: vec![3],
                repeat_last_group: true,
            })));
        refresh(&ui.window, &state.borrow());
        ui.window.invoke_row_click(0, 1, false, false);
        Self {
            ui: windows,
            window: ui.window,
            state,
            commands,
        }
    }

    /// Cancels the open confirmation, then opens `preview` as F8 or Shift+F8 opens it.
    fn show(&self, mut preview: OperationPreview) {
        if !self.window.get_operation_token().is_empty() {
            owned_windows::child_key(&self.ui, Key::Escape);
            slint::platform::update_timers_and_animations();
            assert!(self.window.get_operation_token().is_empty());
        }
        self.commands.borrow_mut().clear();
        let permanent = preview.kind == MutationKind::Delete(DeleteMode::Permanent);
        self.window
            .invoke_mutation(0, if permanent { 2 } else { 1 });
        preview.id = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::PrepareOperation(request) => Some(request.id),
                _ => None,
            })
            .unwrap();
        self.state
            .borrow_mut()
            .apply(Event::Operation(OperationEvent::Preview(Arc::new(preview))));
        refresh(&self.window, &self.state.borrow());
        slint::platform::update_timers_and_animations();
    }

    fn native(&self) -> Rc<MinimalSoftwareWindow> {
        self.ui.borrow()[0].clone()
    }

    /// The open confirmation's popup root.
    fn popup(&self) -> ItemRc {
        let native = self.native();
        let popups = WindowInner::from_pub(native.window()).active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("the confirmation is open");
        ItemRc::new_root(popup.component.clone())
    }

    /// Theme, density and font as Options applies them (the next refresh restores
    /// the presenter's).
    fn appearance(&self, scheme: i32, density: i32, font: i32) {
        self.window.invoke_apply_fonts("".into(), font, font);
        self.window.invoke_apply_preferences(scheme, density);
    }

    /// One frame of `width` × `height` logical pixels at `scale`, kept as an artifact
    /// named `name` when `MIDNIGHT_UI_RENDER_DIR` is set.
    fn render(&self, width: u32, height: u32, scale: f32, name: &str) {
        let native = self.native();
        native
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
        let (physical_width, physical_height) = (
            (width as f32 * scale).round() as u32,
            (height as f32 * scale).round() as u32,
        );
        native.set_size(slint::PhysicalSize::new(physical_width, physical_height));
        native.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        slint::platform::update_timers_and_animations();
        let mut pixels =
            vec![slint::Rgb8Pixel::default(); physical_width as usize * physical_height as usize];
        native.window().request_redraw();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, physical_width as usize);
        });
        bitmap::snapshot(
            &pixels,
            physical_width as usize,
            physical_height as usize,
            name,
        );
    }
}

fn preview(
    mode: DeleteMode,
    files: u64,
    folders: u64,
    file_bytes: Option<u64>,
    sample: Vec<MutationItem>,
) -> OperationPreview {
    OperationPreview {
        id: OperationId(0),
        kind: MutationKind::Delete(mode),
        files,
        folders,
        file_bytes,
        sample,
        trash_unavailable: None,
    }
}

fn one_file(mode: DeleteMode) -> OperationPreview {
    preview(
        mode,
        1,
        0,
        Some(25_320_511),
        vec![item("experiments.rar", EntryKind::File, Some(25_320_511))],
    )
}

fn one_folder(mode: DeleteMode) -> OperationPreview {
    preview(
        mode,
        0,
        1,
        Some(0),
        vec![item("Holiday photos 2026", EntryKind::Directory, None)],
    )
}

fn many(mode: DeleteMode) -> OperationPreview {
    let sample = vec![
        item("experiments.rar", EntryKind::File, Some(25_320_511)),
        item("Holiday photos 2026", EntryKind::Directory, None),
        item("notes.txt", EntryKind::File, Some(4_211)),
        item("build-output", EntryKind::Directory, None),
        item("shortcut to projects", EntryKind::DirectoryLink, None),
        item("budget 2027.xlsx", EntryKind::File, Some(88_120)),
        item("readme.md", EntryKind::File, Some(1_502)),
        item("old-backup.zip", EntryKind::File, Some(912_004_113)),
    ];
    preview(mode, 9, 3, Some(937_420_457), sample)
}

fn long_name() -> String {
    format!(
        "Quarterly report – final version approved by the board {}end.rar",
        "very-long-name-without-any-spaces-".repeat(5)
    )
}

fn long(mode: DeleteMode) -> OperationPreview {
    preview(
        mode,
        1,
        0,
        Some(25_320_511),
        vec![item(&long_name(), EntryKind::File, Some(25_320_511))],
    )
}

/// A file deep in a folder whose path is wider than any dialog.
fn deep(mode: DeleteMode) -> OperationPreview {
    let mut path = folder();
    for part in [
        "Projects",
        "Midnight Explorer",
        "release candidates",
        "2026-11-23 nightly build with symbols",
        "artifacts",
        "x86_64-pc-windows-msvc",
        "packages",
    ] {
        path.push(part);
    }
    let mut file = item("experiments.rar", EntryKind::File, Some(25_320_511));
    file.path = path.join("experiments.rar");
    preview(mode, 1, 0, Some(25_320_511), vec![file])
}

/// The reviewed set: every case at the main window's usual size, and the Blue theme's
/// single file at the extremes of font size, at Touch density and at 200 % scale.
#[test]
fn delete_confirmations_render_for_review() {
    use DeleteMode::{Permanent, Trash};
    let fixture = Fixture::new();
    let cases: [(&str, OperationPreview, i32, i32, i32, f32); 14] = [
        ("one-file", one_file(Permanent), 0, 0, 12, 1.0),
        ("one-folder", one_folder(Permanent), 0, 0, 12, 1.0),
        ("many-mixed", many(Permanent), 0, 0, 12, 1.0),
        ("long-name", long(Permanent), 0, 0, 12, 1.0),
        ("font-8", one_file(Permanent), 0, 0, 8, 1.0),
        ("font-32", one_file(Permanent), 0, 0, 32, 1.0),
        ("touch", one_file(Permanent), 0, 1, 15, 1.0),
        ("scale-200", one_file(Permanent), 0, 0, 12, 2.0),
        ("light-one-file", one_file(Permanent), 2, 0, 12, 1.0),
        ("light-trash-many", many(Trash), 2, 0, 12, 1.0),
        ("dark-trash-one-file", one_file(Trash), 1, 0, 12, 1.0),
        ("dark-trash-folder", one_folder(Trash), 1, 0, 12, 1.0),
        ("high-contrast-light", one_file(Permanent), 22, 0, 12, 1.0),
        ("long-path", deep(Permanent), 0, 0, 12, 1.0),
    ];
    for (name, preview, scheme, density, font, scale) in cases {
        fixture.show(preview);
        fixture.appearance(scheme, density, font);
        let _ = fixture.popup();
        fixture.render(1100, 720, scale, name);
    }
    // The long path at a narrow window too.
    fixture.show(deep(Trash));
    fixture.appearance(0, 0, 12);
    fixture.render(560, 600, 1.0, "long-path-narrow");
}

/// Every shown text of `item`, with the item that draws it.
fn texts(item: ItemRc, found: &mut Vec<(String, ItemRc)>) {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if !item.is_visible() {
        return;
    }
    if let Some(text) = text {
        // Names are drawn between isolation marks that keep them left to right.
        let text = text.replace(['\u{2066}', '\u{2069}'], "");
        found.push((text, item.clone()));
    }
    let mut child = item.first_child();
    while let Some(next) = child {
        child = next.next_sibling();
        texts(next, found);
    }
}

/// An item's rectangle in window coordinates: left, top, right, bottom.
fn edges(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    (at.x, at.y, at.x + rect.width(), at.y + rect.height())
}

fn button(popup: &ItemRc, label: &str) -> ItemRc {
    accessible(popup, AccessibleRole::Button, label).unwrap_or_else(|| panic!("no {label} button"))
}

fn accessible(popup: &ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|text| text == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            if let Some(found) = find(next, role, label) {
                return Some(found);
            }
        }
        None
    }
    find(popup.clone(), role, label)
}

impl Fixture {
    fn layout(&self, width: u32, height: u32) {
        let native = self.native();
        native
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        modeless::layout(&native, width, height);
        slint::platform::update_timers_and_animations();
    }

    /// The confirmation's question, warning, details and actions: none clipped, and the
    /// question first. With `whole`, the window gives the dialog its height and every
    /// detail shows; otherwise the details may scroll.
    fn check(&self, case: &str, whole: bool) -> Shown {
        let popup = self.popup();
        let form = self.window.global::<OperationPresentation>();
        let mut found = Vec::new();
        texts(popup.clone(), &mut found);
        let drawn = |text: &str| -> ItemRc {
            found
                .iter()
                .find(|(shown, _)| shown == text)
                .map(|(_, item)| item.clone())
                .unwrap_or_else(|| panic!("{case}: {text:?} is not drawn"))
        };
        let cancel = button(&popup, "Cancel");
        let (origin, size): (LogicalPoint, LogicalSize) = embedded_form::body(&cancel);
        let (left, right) = (origin.x, origin.x + size.width);
        // A dialog at its largest height (640 px) scrolls what does not fit.
        let whole = whole && size.height < 639.0;
        let question = drawn(form.get_question().as_str());
        let title = self.window.get_operation_title().to_string();
        // The question is the first text under the header, the warning right under it (a
        // long question in a small window scrolls the warning out of view).
        let (_, question_top, _, question_bottom) = edges(&question);
        let warning_text = self.window.get_operation_warning();
        let warning = found
            .iter()
            .find(|(shown, _)| *shown == warning_text.as_str())
            .map(|(_, item)| item.clone());
        assert!(
            warning.is_some() || !whole,
            "{case}: the warning is not drawn"
        );
        let mut bottom = question_bottom;
        if let Some(warning) = &warning {
            let (_, warning_top, _, warning_bottom) = edges(warning);
            assert!(
                warning_top >= question_bottom - 0.5 && warning_top - question_bottom <= 12.0,
                "{case}: the warning follows the question: {question_top} {question_bottom} {warning_top}"
            );
            bottom = warning_bottom;
        }
        for row in form.get_details().iter() {
            let value = if row.identity == "path" {
                let trail = accessible(&popup, AccessibleRole::Text, &row.value)
                    .unwrap_or_else(|| panic!("{case}: no path {:?}", row.value));
                let (x, _, end, _) = edges(&trail);
                assert!(
                    x >= left - 0.5 && end <= right + 0.5,
                    "{case}: the path is clipped at {x}..{end}, the dialog is {left}..{right}"
                );
                None
            } else {
                Some(row.value.as_str())
            };
            for text in [Some(row.label.as_str()), value].into_iter().flatten() {
                let Some((_, item)) = found.iter().find(|(shown, _)| shown == text) else {
                    assert!(!whole, "{case}: {text:?} is not drawn");
                    continue;
                };
                let (x, top, end, low) = edges(item);
                assert!(top > question_top, "{case}: {text:?} follows the question");
                assert!(
                    x >= left - 0.5 && end <= right + 0.5,
                    "{case}: {text:?} is clipped at {x}..{end}, the dialog is {left}..{right}"
                );
                bottom = bottom.max(low);
            }
        }
        // A small window scrolls a long question; a regular one shows it and the warning.
        if size.width >= 600.0 {
            embedded_form::assert_unclipped(&question, case);
            if let Some(warning) = &warning {
                embedded_form::assert_unclipped(warning, case);
            }
        }
        // Both actions keep their caption's padding inside the button that draws it (where
        // both do not fit beside each other, the primary one scrolls), and the primary one
        // stays clear of the resize grip in the corner (9 px, 3 px from the edges).
        let confirm = button(&popup, &title);
        for (action, label) in [(&cancel, "Cancel"), (&confirm, title.as_str())] {
            embedded_form::assert_inside(action, label);
            let caption = found
                .iter()
                .filter(|(shown, _)| shown == label)
                .max_by(|a, b| edges(&a.1).1.total_cmp(&edges(&b.1).1))
                .map(|(_, item)| item.clone())
                .unwrap();
            let drawn = caption
                .parent_item(ParentItemTraversalMode::StopAtPopups)
                .unwrap();
            let (start, _, end, _) = edges(&drawn);
            let caption = edges(&caption);
            assert!(
                caption.0 - start >= 8.0 && end - caption.2 >= 8.0,
                "{case}: the {label:?} caption {caption:?} needs padding in {start}..{end}"
            );
        }
        let (_, actions_top, confirm_right, _) = edges(&confirm);
        assert!(
            confirm_right <= right - 13.0,
            "{case}: the primary action reaches the resize grip: {confirm_right} of {right}"
        );
        Shown {
            height: size.height,
            band: actions_top - bottom,
        }
    }
}

struct Shown {
    height: f32,
    /// From the last detail to the actions.
    band: f32,
}

/// At every density and font size, from 8 to 32 px, each confirmation shows its question
/// first and every detail unclipped, its actions padded and clear of the resize grip;
/// where the window gives it the height it asks for, it has no empty band.
#[test]
fn delete_confirmations_fit_their_content_at_every_font_and_density() {
    use DeleteMode::{Permanent, Trash};
    let fixture = Fixture::new();
    for (density, font) in [(0, 8), (0, 12), (0, 20), (0, 32), (1, 15), (1, 32)] {
        for (case, preview) in [
            ("file", one_file(Permanent)),
            ("folder", one_folder(Trash)),
            ("many", many(Permanent)),
            ("long-name", long(Permanent)),
            ("deep", deep(Trash)),
        ] {
            fixture.show(preview);
            fixture.appearance(0, density, font);
            let label = format!("{case} density {density} font {font}");
            for (width, height) in [(1340, 900), (520, 360)] {
                fixture.layout(width, height);
                let shown = fixture.check(&format!("{label} at {width}×{height}"), width == 1340);
                if width == 1340 && shown.height < 639.0 {
                    assert!(
                        (0.0..=36.0).contains(&shown.band),
                        "{label}: an empty band of {} px above the actions",
                        shown.band
                    );
                }
            }
        }
    }
}

/// A file in `folder`.
fn in_folder(folder: PathBuf, mode: DeleteMode) -> OperationPreview {
    let mut file = item("experiments.rar", EntryKind::File, Some(25_320_511));
    file.path = folder.join("experiments.rar");
    preview(mode, 1, 0, Some(25_320_511), vec![file])
}

/// A drawn path: its pieces left to right, the trail's start and end, and each piece's.
type Trail = (Vec<String>, (f32, f32), Vec<(f32, f32)>);

impl Fixture {
    /// The Location's components as drawn, left to right (dropped ones as one "..."),
    /// with the trail's box and the box of each drawn piece.
    fn trail(&self, case: &str) -> Trail {
        let popup = self.popup();
        let form = self.window.global::<OperationPresentation>();
        let row = form
            .get_details()
            .iter()
            .find(|row| row.identity == "path")
            .unwrap_or_else(|| panic!("{case}: no path row"));
        let trail = accessible(&popup, AccessibleRole::Text, &row.value)
            .unwrap_or_else(|| panic!("{case}: no path"));
        let (start, _, end, _) = edges(&trail);
        let mut found = Vec::new();
        texts(trail, &mut found);
        let mut pieces: Vec<_> = found
            .into_iter()
            .map(|(text, item)| {
                let (x, _, right, _) = edges(&item);
                (x, text, right)
            })
            .collect();
        // By their ends: a component cut at its start begins before the trail.
        pieces.sort_by(|a, b| a.2.total_cmp(&b.2));
        let boxes = pieces.iter().map(|piece| (piece.0, piece.2)).collect();
        (
            pieces.into_iter().map(|piece| piece.1).collect(),
            (start, end),
            boxes,
        )
    }
}

/// The path bar's rule, checked on what the Location shows: whole components only, the
/// whole path when it fits, otherwise at most two leading ones, one "..." for the dropped
/// middle ones and a trailing run that ends with the last component; nothing past the
/// trail's end. Returns how many components show.
fn assert_path_bar_rule(case: &str, labels: &[String], pieces: &[String]) -> usize {
    let Some(marker) = pieces.iter().position(|piece| piece == "...") else {
        assert_eq!(pieces, labels, "{case}: a path that fits shows whole");
        return labels.len();
    };
    assert_eq!(
        pieces.iter().filter(|piece| *piece == "...").count(),
        1,
        "{case}: one marker: {pieces:?}"
    );
    assert!(
        marker <= 2,
        "{case}: at most two leading components: {pieces:?}"
    );
    assert_eq!(&pieces[..marker], &labels[..marker], "{case}: {pieces:?}");
    let tail = &pieces[marker + 1..];
    assert!(
        !tail.is_empty(),
        "{case}: the last component shows: {pieces:?}"
    );
    assert_eq!(
        tail,
        &labels[labels.len() - tail.len()..],
        "{case}: whole trailing components: {pieces:?}"
    );
    assert!(
        marker + tail.len() < labels.len(),
        "{case}: the marker stands for dropped components: {pieces:?}"
    );
    marker + tail.len()
}

/// A folder's path that does not fit drops whole middle components behind one "...", as
/// the path bar does; it follows the dialog's width, font and density, and the whole path
/// stays its tooltip and accessible label. A path that fits shows whole.
#[test]
fn a_long_location_drops_whole_middle_components_as_the_path_bar_does() {
    use DeleteMode::{Permanent, Trash};
    let fixture = Fixture::new();
    for mode in [Permanent, Trash] {
        fixture.show(deep(mode));
        let form = fixture.window.global::<OperationPresentation>();
        let row = form
            .get_details()
            .iter()
            .find(|row| row.label == "Location")
            .unwrap();
        assert_eq!(row.identity, "path");
        let labels: Vec<String> = form
            .get_path_parts()
            .iter()
            .map(|part| part.label.to_string())
            .collect();
        let joined: String = labels.concat();
        assert_eq!(
            joined,
            row.value.as_str(),
            "the components join into the path"
        );
        for (density, font) in [(0, 8), (0, 12), (1, 15), (0, 20)] {
            fixture.appearance(0, density, font);
            let mut shown = Vec::new();
            // Narrower and narrower: the dialog follows the window below its 540 px.
            for width in [1340, 520, 460, 400, 360] {
                if width == 360 && font > 15 {
                    continue;
                }
                fixture.layout(width, 900);
                let case = format!("{mode:?} font {font} density {density} at {width}");
                fixture.check(&case, true);
                let (pieces, (start, end), boxes) = fixture.trail(&case);
                shown.push(assert_path_bar_rule(&case, &labels, &pieces));
                for (x, right) in boxes {
                    assert!(
                        x >= start - 0.5 && right <= end + 1.5,
                        "{case}: a piece {x}..{right} outside the trail {start}..{end}"
                    );
                }
            }
            assert!(
                shown.windows(2).all(|pair| pair[0] >= pair[1]),
                "font {font}: a narrower dialog shows no more components: {shown:?}"
            );
            assert!(
                font > 15 || shown.first() > shown.last(),
                "font {font}: the components follow the width: {shown:?}"
            );
            // Back to the wide window: shortened again from the same parts.
            fixture.layout(1340, 900);
            let (pieces, _, _) = fixture.trail("wide again");
            assert_eq!(
                assert_path_bar_rule("wide again", &labels, &pieces),
                shown[0]
            );
        }
    }
    fixture.show(one_file(Permanent));
    fixture.appearance(0, 0, 12);
    fixture.layout(1340, 900);
    let (pieces, _, _) = fixture.trail("a short path");
    assert!(
        !pieces.iter().any(|piece| piece == "..."),
        "a path that fits is not shortened: {pieces:?}"
    );
}

/// A UNC share and an archive inside a folder split as the path bar splits them: the
/// share is one leading component, the archive one of the middle ones.
#[test]
fn unc_and_archive_locations_drop_whole_components() {
    let fixture = Fixture::new();
    let mut folders = vec![(
        "archive",
        folder()
            .join("backups")
            .join("photos 2026 with every album.zip")
            .join("2026")
            .join("summer holiday in the mountains")
            .join("beach and lake"),
    )];
    if cfg!(windows) {
        folders.push((
            "unc",
            PathBuf::from(r"\\fileserver\shared documents")
                .join("Projects")
                .join("Midnight Explorer")
                .join("release candidates")
                .join("2026-11-23 nightly build with symbols"),
        ));
    }
    for (name, path) in folders {
        fixture.show(in_folder(path, DeleteMode::Permanent));
        let form = fixture.window.global::<OperationPresentation>();
        let labels: Vec<String> = form
            .get_path_parts()
            .iter()
            .map(|part| part.label.to_string())
            .collect();
        if name == "unc" {
            assert_eq!(labels[0], r"\\fileserver\shared documents\", "{labels:?}");
        } else {
            assert!(
                labels
                    .iter()
                    .any(|label| label.ends_with("photos 2026 with every album.zip")),
                "{labels:?}"
            );
        }
        fixture.appearance(0, 0, 12);
        let mut dropped = false;
        for width in [1340, 460, 380] {
            fixture.layout(width, 900);
            let case = format!("{name} at {width}");
            let (pieces, _, _) = fixture.trail(&case);
            let shown = assert_path_bar_rule(&case, &labels, &pieces);
            dropped |= shown < labels.len();
        }
        assert!(dropped, "{name}: a narrow dialog drops components");
    }
}

/// A folder whose last component alone is wider than the trail: the path bar's marker,
/// then that component cut at its start, so that its end stays in view.
#[test]
fn a_single_very_long_component_keeps_its_end() {
    let fixture = Fixture::new();
    let long = format!("{}end-of-the-name", "a-very-long-folder-name-".repeat(8));
    fixture.show(in_folder(folder().join(&long), DeleteMode::Permanent));
    fixture.appearance(0, 0, 12);
    for width in [1340, 460] {
        fixture.layout(width, 900);
        let case = format!("at {width}");
        fixture.check(&case, true);
        let (pieces, (start, end), boxes) = fixture.trail(&case);
        assert_eq!(pieces[0], "...", "{case}: {pieces:?}");
        let last = pieces.last().unwrap();
        assert!(last.ends_with("end-of-the-name"), "{case}: {pieces:?}");
        let (x, right) = *boxes.last().unwrap();
        assert!(
            x < start && right <= end + 0.5 && end - right <= 6.0,
            "{case}: the cut component ends at the trail's end: {x}..{right} in {start}..{end}"
        );
    }
}
