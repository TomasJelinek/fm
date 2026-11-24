//! Directory comparison badges in the real window: hovering a badge (only the badge)
//! shows the shared tooltip with what differs, its accessible description says the
//! same, and Clear comparison (the × at the end of the result line, Escape in an idle
//! panel, Ctrl+Shift+F11 and the main menu) clears both panels' marks without
//! touching the selection. Layout at fonts 8, 12 and 32, Desktop and Touch.
use crate::{CompareBadges, file_keys::FileKey, test_harness::Harness};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use slint::{
    ComponentHandle, LogicalPosition, Model, SharedString,
    platform::{Key, WindowEvent},
};
use std::{
    cell::RefCell,
    ffi::OsStr,
    path::PathBuf,
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

const NEWER: &str = "Newer here: day 200000 12:00:00 vs day 27200 12:00:00 (2 d)";
const OLDER: &str = "Older here: day 27200 12:00:00 vs day 200000 12:00:00 (2 d)";
const INSIDE: &str = "Differs inside: 1 file differs, 1 only here";
const SAME: &str = "Matches: same size, time (within 2 s) and contents";

fn listing(root: &str, newer: u64) -> Vec<Entry> {
    [
        ("folder", EntryKind::Directory, 1_000),
        ("newer.txt", EntryKind::File, newer),
        ("same.txt", EntryKind::File, 5_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, kind, seconds))| {
        let mut entry = Entry::new(
            EntryId(index as u64 + 1),
            PathBuf::from(root).join(name),
            kind,
        );
        entry.metadata.size = (kind == EntryKind::File).then_some(10);
        entry.metadata.modified = Some(UNIX_EPOCH + Duration::from_secs(seconds));
        entry.metadata.localized_modified = Some(LocalizedDateTime {
            date: format!("day {seconds}"),
            time: "12:00:00".into(),
        });
        entry
    })
    .collect()
}

fn harness(preferences: UserPreferences) -> Harness {
    let mut h = Harness::with_locations(preferences, "left".into(), "right".into());
    for (pane, root, newer) in [
        (PaneId::Left, "left", 200_000),
        (PaneId::Right, "right", 27_200),
    ] {
        let navigation = h
            .sent
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: target,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: target,
                    navigation,
                    ..
                } if *target == pane => Some(*navigation),
                _ => None,
            })
            .expect("the panel asked for its listing");
        let entries = listing(root, newer);
        h.presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries: entries.into(),
        });
    }
    h.refresh();
    h
}

fn marks() -> CompareMarks {
    let mut marks = CompareMarks::new([PathBuf::from("left"), PathBuf::from("right")], true);
    let key = |name: &str| marks.child_key(OsStr::new(""), OsStr::new(name));
    let (folder, newer, same) = (key("folder"), key("newer.txt"), key("same.txt"));
    marks.record(
        &folder,
        None,
        CompareMark::Differs,
        &CompareDetail {
            inside: Some(CompareInside {
                files: 1,
                only: [1, 0],
                ..CompareInside::default()
            }),
            ..CompareDetail::default()
        },
    );
    marks.record(
        &newer,
        None,
        CompareMark::Differs,
        &CompareDetail {
            flags: CompareFlags::TIME,
            times: Some([200_000, 27_200]),
            ..CompareDetail::default()
        },
    );
    marks.record(&same, None, CompareMark::Same, &CompareDetail::default());
    marks
}

/// A comparison with subfolders and contents that selects the folder and newer.txt
/// in both panels and leaves its marks.
fn compare(h: &mut Harness) {
    let commands = RefCell::new(Vec::new());
    {
        let mut p = h.presenter.borrow_mut();
        p.open_comparison();
        let options = CompareOptions {
            recursive: true,
            by_content: true,
            ..CompareOptions::default()
        };
        p.submit_comparison(options, Default::default(), &|command| {
            commands.borrow_mut().push(command);
            true
        });
    }
    let request = commands
        .into_inner()
        .into_iter()
        .find_map(|command| match command {
            Command::CompareDirectories(request) => Some(Arc::new(request)),
            _ => None,
        })
        .expect("Compare sent its request");
    let selections = request.sides.each_ref().map(|side| {
        Arc::new(SelectionSnapshot::from_bits(side.view.clone(), vec![0b011]).unwrap())
    });
    let result = CompareResult {
        request: request.clone(),
        selections,
        tree_replacements: [None, None],
        marks: Some(Arc::new(marks())),
        unknown: 0,
        ambiguous: 0,
        differing_folders: 1,
        compared_folders: 1,
        content_differences: 0,
        content_files: 1,
        content_bytes: 20,
        walked_folders: 1,
        walked_files: 1,
        dst_shifts: 0,
        skipped: 0,
        errors: 0,
        limited: false,
        diagnostics: Vec::new(),
    };
    h.presenter.borrow_mut().apply(Event::DirectoriesCompared {
        id: request.id,
        result: Ok(Arc::new(result)),
    });
    h.refresh();
}

/// Each row's name, badge and badge words.
fn rows(h: &Harness, pane: usize) -> Vec<(String, i32, String)> {
    let model = if pane == 0 {
        h.window.get_left_rows()
    } else {
        h.window.get_right_rows()
    };
    model
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| {
            (
                row.name.to_string(),
                row.compare_mark,
                row.compare_detail.to_string(),
            )
        })
        .collect()
}

fn marked(h: &Harness) -> usize {
    [0, 1]
        .iter()
        .flat_map(|pane| rows(h, *pane))
        .filter(|(_, mark, _)| *mark != 0)
        .count()
}

fn string(item: &ItemRc, property: AccessibleStringProperty) -> String {
    item.accessible_string_property(property)
        .unwrap_or_default()
        .to_string()
}

/// The badge whose accessible description is `detail`.
fn badge(h: &Harness, detail: &str) -> ItemRc {
    h.find(|item| {
        item.accessible_role() == AccessibleRole::Text
            && string(item, AccessibleStringProperty::Label).starts_with("Compared: ")
            && string(item, AccessibleStringProperty::Description) == detail
    })
    .unwrap_or_else(|| panic!("no badge says {detail:?}"))
}

fn center(item: &ItemRc) -> LogicalPosition {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}

/// Texts of the tooltips now shown.
fn tooltips(h: &Harness) -> Vec<String> {
    fn walk(item: ItemRc, texts: &mut Vec<String>) {
        let label = string(&item, AccessibleStringProperty::Label);
        if !label.is_empty() {
            texts.push(label);
        }
        let mut child = item.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            walk(current, texts);
        }
    }
    let mut texts = Vec::new();
    for popup in WindowInner::from_pub(h.window.window())
        .active_popups()
        .iter()
    {
        if matches!(popup.window_kind, WindowKind::ToolTip) {
            walk(ItemRc::new_root(popup.component.clone()), &mut texts);
        }
    }
    texts
}

fn hover(h: &mut Harness, position: LogicalPosition) {
    // Away first, so that a tooltip shown before closes.
    h.window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1.0, 1.0),
    });
    h.advance(Duration::from_millis(50));
    h.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    h.advance(Duration::from_millis(400));
    h.advance(Duration::from_millis(400));
}

#[test]
fn hovering_a_badge_explains_the_verdict_and_its_description_says_the_same() {
    let mut h = harness(UserPreferences::default());
    compare(&mut h);
    assert_eq!(
        rows(&h, 0),
        [
            ("folder".into(), 2, INSIDE.into()),
            ("newer.txt".into(), 2, NEWER.into()),
            ("same.txt".into(), 1, SAME.into()),
        ]
    );
    assert_eq!(rows(&h, 1)[1], ("newer.txt".into(), 2, OLDER.into()));
    // The row's own description is the badge's text too.
    let row = h
        .find(|item| {
            item.accessible_role() == AccessibleRole::ListItem
                && string(item, AccessibleStringProperty::Label) == "newer.txt"
                && string(item, AccessibleStringProperty::Description) == NEWER
        })
        .expect("the row describes the verdict");
    let badge_item = badge(&h, NEWER);
    assert_eq!(
        string(&badge_item, AccessibleStringProperty::Label),
        "Compared: differs"
    );

    // The name of the same row has no comparison tooltip: only the badge has.
    let name = row.geometry();
    let origin = row.map_to_native_window(name.origin);
    hover(
        &mut h,
        LogicalPosition::new(origin.x + 120.0, origin.y + name.size.height / 2.0),
    );
    assert!(
        !tooltips(&h).iter().any(|text| text == NEWER),
        "{:?}",
        tooltips(&h)
    );
    let position = center(&badge_item);
    hover(&mut h, position);
    assert!(
        tooltips(&h).iter().any(|text| text == NEWER),
        "hovering the badge shows what differs: {:?}",
        tooltips(&h)
    );
    let folder = center(&badge(&h, INSIDE));
    hover(&mut h, folder);
    assert!(tooltips(&h).iter().any(|text| text == INSIDE));
}

#[test]
fn clear_comparison_by_button_keys_and_menu_keeps_the_selection() {
    let mut h = harness(UserPreferences::default());
    let selected = |h: &Harness| {
        let p = h.presenter.borrow();
        [0, 1].map(|slot| p.panes[slot].selected.clone())
    };
    // The × at the end of the result line.
    compare(&mut h);
    assert_eq!(marked(&h), 6);
    let line = h.window.get_left_watch_status().to_string();
    assert!(line.contains("clears the marks"), "{line}");
    assert!(h.window.global::<CompareBadges>().get_shown());
    let before = selected(&h);
    assert_eq!(before[0].len(), 2);
    h.click(
        "Close comparison result and clear marks",
        AccessibleRole::Button,
    );
    assert_eq!(marked(&h), 0, "both panels' marks are gone");
    assert!(h.window.get_left_watch_status().is_empty(), "and the line");
    assert!(!h.window.global::<CompareBadges>().get_shown());
    let after = selected(&h);
    for slot in 0..2 {
        assert!(
            Arc::ptr_eq(&before[slot], &after[slot]),
            "the selection stays"
        );
    }
    assert_eq!(
        crate::file_key_bridge::dispatch(&h.window, 0, FileKey::ClearComparison),
        0,
        "with nothing shown the key stays unhandled, as before"
    );

    // Escape in the panel; first it ends a filename search, then clears the marks.
    compare(&mut h);
    h.window.invoke_focus_files();
    h.settle();
    h.chord(&[], "n");
    assert!(
        h.window.get_left_search_active(),
        "typing started a filename search"
    );
    h.key(Key::Escape);
    assert!(!h.window.get_left_search_active());
    assert_eq!(marked(&h), 6, "Escape ended the search only");
    h.key(Key::Escape);
    assert_eq!(marked(&h), 0);

    // Ctrl+Shift+F11.
    compare(&mut h);
    h.window.invoke_focus_files();
    h.chord(&[Key::Control, Key::Shift], &SharedString::from(Key::F11));
    assert_eq!(marked(&h), 0);

    // The main menu's action.
    compare(&mut h);
    h.window
        .invoke_function_action("panel.compare-clear".into(), 1);
    h.settle();
    assert_eq!(marked(&h), 0);
    let after = selected(&h);
    assert_eq!(after[0].len(), 2, "the comparison's selection stays");
}

/// The badge stays inside its row and the × inside the result line at fonts 8, 12
/// and 32, with Desktop and Touch density.
#[test]
fn badge_and_clear_button_fit_at_every_font_and_density() {
    let mut h = harness(UserPreferences::default());
    compare(&mut h);
    for density in [UiDensity::Desktop, UiDensity::Touch] {
        for font in [8, 12, 32] {
            {
                let mut p = h.presenter.borrow_mut();
                p.preferences.values.density = density;
                p.preferences.values.desktop_font_size = font;
                p.preferences.values.touch_font_size = font;
            }
            h.refresh();
            let case = format!("{density:?} font {font}");
            let badge_item = badge(&h, NEWER);
            let row = h
                .find(|item| {
                    item.accessible_role() == AccessibleRole::ListItem
                        && string(item, AccessibleStringProperty::Label) == "newer.txt"
                })
                .unwrap();
            let (b, r) = (badge_item.geometry(), row.geometry());
            let b_origin = badge_item.map_to_native_window(b.origin);
            let r_origin = row.map_to_native_window(r.origin);
            assert!(
                b_origin.y >= r_origin.y - 0.5
                    && b_origin.y + b.size.height <= r_origin.y + r.size.height + 0.5
                    && b_origin.x >= r_origin.x
                    && b_origin.x + b.size.width <= r_origin.x + r.size.width,
                "{case}: badge {b_origin:?} {:?} outside row {r_origin:?} {:?}",
                b.size,
                r.size
            );
            let line = h
                .find(|item| {
                    item.accessible_role() == AccessibleRole::Text
                        && string(item, AccessibleStringProperty::Label)
                            .starts_with("Compared: 2 selected")
                })
                .unwrap_or_else(|| panic!("{case}: no result line"));
            let button = h.item(
                "Close comparison result and clear marks",
                AccessibleRole::Button,
            );
            let (l, c) = (line.geometry(), button.geometry());
            let l_origin = line.map_to_native_window(l.origin);
            let c_origin = button.map_to_native_window(c.origin);
            // The gate's checks: square, centred below the line's 1px rule, its right
            // edge on the text's right content edge (8px in), inside the line.
            let centre = c_origin.y + c.size.height / 2.0;
            let wanted_centre = l_origin.y + 1.0 + (l.size.height - 1.0) / 2.0;
            assert!(
                (c.size.width - c.size.height).abs() < 0.5
                    && c.size.width >= 12.0
                    && (centre - wanted_centre).abs() <= 0.5
                    && (c_origin.x + c.size.width - (l_origin.x + l.size.width - 8.0)).abs() < 0.5
                    && c_origin.y >= l_origin.y + 1.0 - 0.01
                    && c_origin.y + c.size.height <= l_origin.y + l.size.height + 0.01,
                "{case}: × {c_origin:?} {:?} in the line {l_origin:?} {:?}",
                c.size,
                l.size
            );
        }
    }
}
