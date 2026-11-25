//! Linux drives show the short names the backend gives them (`USB`, `data on nas`,
//! `backup/data`) on the drive bar and in the drive menu. Their tooltips and accessible
//! texts spell out the whole mount path, and the menu row's accessible description
//! carries what its tooltip shows. A label that the root's own text starts with (a
//! Windows drive letter, the Linux root `/`) keeps the presentation it had.
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{LocationRole, MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    rc::Rc,
    time::Duration,
};

fn volume(id: &str, label: &str, root: &str) -> Volume {
    Volume {
        id: id.into(),
        label: label.into(),
        root: root.into(),
    }
}

/// 1 GiB free, and the volume name the details query found, if any.
fn details(name: Option<&str>) -> VolumeDetails {
    VolumeDetails {
        label: Ok(name.map(OsString::from)),
        space: Ok(SpaceInfo {
            available: 1 << 30,
            total: 2 << 30,
        }),
    }
}

/// A window showing `volumes`, with `named` details (id, root, volume name) arrived.
fn window_with(
    volumes: Vec<Volume>,
    named: &[(&str, &str, Option<&str>)],
) -> (ui::TestUi, Rc<RefCell<Presenter>>, Rc<Cell<Duration>>) {
    let time = Rc::new(Cell::new(Duration::ZERO));
    let mut test = ui::TestUi::bare_with_clock(time.clone());
    let start = std::env::temp_dir();
    let presenter = Rc::new(RefCell::new(Presenter::new(start.clone(), start)));
    install(&test.window, presenter.clone(), Rc::new(|_| true));
    presenter.borrow_mut().apply(Event::VolumesUpdated(volumes));
    for (id, root, name) in named {
        presenter.borrow_mut().apply(Event::VolumeDetailsUpdated {
            volume_id: (*id).into(),
            root: (*root).into(),
            details: details(*name),
        });
    }
    refresh(&test.window, &presenter.borrow());
    test.draw();
    (test, presenter, time)
}

/// The left drive bar's drives: (caption, tooltip).
fn bar(window: &MainWindow) -> Vec<(String, String)> {
    window
        .get_left_locations()
        .iter()
        .filter(|row| row.kind == LocationRole::Drive)
        .map(|row| (row.compact_label.to_string(), row.tooltip.to_string()))
        .collect()
}

/// The drive menu's drive rows: (label, tooltip).
fn menu(window: &MainWindow) -> Vec<(String, String)> {
    window
        .get_navigation_choices()
        .iter()
        .filter(|row| row.kind == LocationRole::Drive)
        .map(|row| (row.label.to_string(), row.tooltip.to_string()))
        .collect()
}

fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn open_left_menu(window: &MainWindow) {
    window.invoke_focus_files();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Alt.into(),
    });
    ui::key(window, Key::F1);
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Alt.into(),
    });
}

fn description(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Description)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// The texts of the tooltips the window shows.
fn tooltips(window: &MainWindow) -> Vec<String> {
    fn texts(item: ItemRc, found: &mut Vec<String>) {
        if let Some(text) = item.downcast::<SimpleText>() {
            found.push(text.as_pin_ref().text().to_string());
        }
        if let Some(text) = item.downcast::<ComplexText>() {
            found.push(text.as_pin_ref().text().to_string());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            texts(item, found);
        }
    }
    let inner = WindowInner::from_pub(window.window());
    let mut found = Vec::new();
    for popup in inner
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
    {
        texts(ItemRc::new_root(popup.component.clone()), &mut found);
    }
    found
}

#[test]
fn linux_drives_show_short_names_with_the_whole_mount_path_in_tooltips() {
    let (mut test, _presenter, time) = window_with(
        vec![
            volume("mount:2f", "/", "/"),
            volume("mount:usb", "USB", "/media/alice/USB"),
            volume(
                "mount:share",
                "data on nas",
                "/run/user/1000/gvfs/smb-share:server=nas,share=data",
            ),
            volume("mount:data", "backup/data", "/mnt/backup/data"),
        ],
        &[
            ("mount:usb", "/media/alice/USB", Some("USB")),
            ("mount:data", "/mnt/backup/data", Some("Archive")),
        ],
    );
    let window = test.window.clone_strong();
    // The bar: the short name, and a tooltip naming it, its path and its details. A
    // volume name the short name already shows is not repeated.
    assert_eq!(
        bar(&window),
        pairs(&[
            ("/", "/\nLoading free space…"),
            ("USB", "USB\n/media/alice/USB\n1.0 GiB free"),
            (
                "data on nas",
                "data on nas\n/run/user/1000/gvfs/smb-share:server=nas,share=data\nLoading free space…"
            ),
            (
                "backup/data",
                "backup/data\n/mnt/backup/data\nArchive\n1.0 GiB free"
            ),
        ])
    );
    // The menu: the short name and its details, without the path; the same tooltips.
    assert_eq!(
        menu(&window),
        pairs(&[
            ("/ — Loading free space…", "/\nLoading free space…"),
            ("USB — 1.0 GiB free", "USB\n/media/alice/USB\n1.0 GiB free"),
            (
                "data on nas — Loading free space…",
                "data on nas\n/run/user/1000/gvfs/smb-share:server=nas,share=data\nLoading free space…"
            ),
            (
                "backup/data — Archive — 1.0 GiB free",
                "backup/data\n/mnt/backup/data\nArchive\n1.0 GiB free"
            ),
        ])
    );

    // The bar button shows only the short name; its accessible name holds the path.
    let usb = ui::root_item(
        &window,
        "USB\n/media/alice/USB\n1.0 GiB free",
        AccessibleRole::Button,
    );
    let caption: Vec<String> = ui::visible_text_values(usb.clone())
        .iter()
        .map(|text| text.to_string())
        .collect();
    assert_eq!(caption, ["USB"]);

    // Hovering it shows the tooltip with the whole mount path.
    let rect = usb.geometry();
    let origin = usb.map_to_native_window(rect.origin);
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        ),
    });
    test.draw();
    time.set(Duration::from_millis(1000));
    test.draw();
    let shown = tooltips(&window).join("\n");
    assert!(
        shown.contains("/media/alice/USB"),
        "hovering the USB button shows its mount path: {shown:?}"
    );
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    test.draw();

    // A menu row reads its short name; its accessible description spells out the path,
    // for the keyboard and screen readers alike.
    open_left_menu(&window);
    test.draw();
    for (label, path) in [
        ("USB — 1.0 GiB free", "/media/alice/USB"),
        (
            "data on nas — Loading free space…",
            "/run/user/1000/gvfs/smb-share:server=nas,share=data",
        ),
        ("backup/data — Archive — 1.0 GiB free", "/mnt/backup/data"),
    ] {
        let row = ui::popup_item(&window, label, AccessibleRole::Button);
        assert!(
            description(&row).contains(path),
            "{label}: {:?}",
            description(&row)
        );
    }
}

#[test]
fn drive_letters_keep_their_root_in_the_menu_and_tooltip() {
    let (test, _presenter, _time) = window_with(
        vec![
            volume("drive:C", "C:", "C:\\"),
            volume("drive:D", "D:", "D:\\"),
        ],
        &[("drive:D", "D:\\", Some("Work"))],
    );
    let window = test.window.clone_strong();
    assert_eq!(
        bar(&window),
        pairs(&[
            ("C:", "C:\\\nLoading free space…"),
            ("D:", "D:\\\nWork\n1.0 GiB free"),
        ])
    );
    assert_eq!(
        menu(&window),
        pairs(&[
            (
                "C: (C:\\) — Loading free space…",
                "C:\\\nLoading free space…"
            ),
            (
                "D: (D:\\) — Work — 1.0 GiB free",
                "D:\\\nWork\n1.0 GiB free"
            ),
        ])
    );
}
