//! The About dialog lives inside the main window, can be dragged by its sky and
//! lists the third-party libraries; a library's notices come from the backend.
#[path = "support/owned_commands.rs"]
mod owned_commands;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    cursor::MouseCursorInner,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, BuiltInMouseCursor, ClippedImage, ImageItem, TouchArea},
    lengths::LogicalRect,
    window::{WindowInner, WindowKind},
};
use midnight_core::{Command, Event, LicenseNotice, LicenseNoticeRequest};
use midnight_ui::{AboutView, BeginOwned, MainWindow, Presenter};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

/// The main window with its production callbacks, a recording command sender and
/// a virtual clock for Slint's tooltip delay.
struct About {
    ui: ui::TestUi,
    clock: Rc<Cell<Duration>>,
    presenter: Rc<RefCell<Presenter>>,
    begin: BeginOwned,
    commands: Rc<RefCell<Vec<Command>>>,
}

impl About {
    fn open(width: usize, height: usize) -> Self {
        let (mut ui, clock) = ui::TestUi::with_virtual_clock();
        ui.resize(width, height);
        let base = std::env::temp_dir().join("about dialog 文档");
        let presenter = Rc::new(RefCell::new(Presenter::new(
            base.clone(),
            base.join("other"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        let begin = owned_commands::install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        ui.window.set_application_version("1.2.3".into());
        ui.draw();
        ui.window.invoke_show_about();
        ui.draw();
        assert!(ui.window.get_about_open());
        Self {
            ui,
            clock,
            presenter,
            begin,
            commands,
        }
    }

    fn window(&self) -> &MainWindow {
        &self.ui.window
    }

    fn view(&self) -> AboutView<'_> {
        self.ui.window.global::<AboutView>()
    }

    /// The control's center. The item handle is released at once: a held handle
    /// would keep a destroyed control (and its tooltip) alive.
    fn center(&self, label: &str, role: AccessibleRole) -> LogicalPosition {
        let item = ui::popup_item(self.window(), label, role);
        let rect = item.geometry();
        let at = item.map_to_native_window(rect.origin);
        LogicalPosition::new(at.x + rect.size.width / 2.0, at.y + rect.size.height / 2.0)
    }

    fn click(&mut self, label: &str, role: AccessibleRole) {
        let at = self.center(label, role);
        ui::click(self.window(), at.x, at.y);
        self.ui.draw();
    }

    fn double_click(&mut self, label: &str) {
        let at = self.center(label, AccessibleRole::Button);
        for _ in 0..2 {
            ui::pointer_button(self.window(), at, PointerEventButton::Left, true);
            ui::pointer_button(self.window(), at, PointerEventButton::Left, false);
        }
        self.ui.draw();
    }

    fn key(&mut self, text: impl Into<slint::SharedString>) {
        ui::key(self.window(), text);
        self.ui.draw();
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.key(character.to_string());
        }
    }

    fn tooltips(&self) -> usize {
        WindowInner::from_pub(self.window().window())
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .count()
    }

    /// Rest the pointer on a control until its tooltip shows.
    fn hover_until_hint(&mut self, label: &str, role: AccessibleRole) -> LogicalPosition {
        let at = self.center(label, role);
        self.window()
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position: at });
        ui::advance(&self.clock, Duration::from_millis(600));
        self.ui.draw();
        assert!(self.tooltips() > 0, "{label}'s tooltip must show first");
        at
    }

    fn licenses(&mut self) {
        self.click("Licenses", AccessibleRole::Tab);
        assert!(
            ui::try_popup_item(self.window(), "Filter libraries", AccessibleRole::TextInput)
                .is_some()
        );
    }

    /// Replace the filter text by typing, as a user does.
    fn filter(&mut self, query: &str) {
        self.click("Filter libraries", AccessibleRole::TextInput);
        ui::chord(self.window(), &[Key::Control], "a");
        self.key(Key::Backspace);
        self.type_text(query);
    }

    fn filter_text(&self) -> String {
        ui::popup_item(self.window(), "Filter libraries", AccessibleRole::TextInput)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .to_string()
    }

    fn rows(&self) -> Vec<String> {
        self.view()
            .get_rows()
            .iter()
            .map(|row| format!("{} {}", row.name, row.version))
            .collect()
    }

    /// The rows' accessible names, which are also their detail titles.
    fn labels(&self) -> Vec<String> {
        self.view()
            .get_rows()
            .iter()
            .map(|row| {
                if row.version.is_empty() {
                    row.name.to_string()
                } else {
                    format!("{} {}", row.name, row.version)
                }
            })
            .collect()
    }

    /// The window bounds of the library list's viewport.
    fn list(&self) -> LogicalRect {
        bounds(&ui::popup_item(
            self.window(),
            "Libraries",
            AccessibleRole::List,
        ))
    }

    /// The window bounds of a row the list shows, when it has one for `label`.
    fn row_bounds(&self, label: &str) -> Option<LogicalRect> {
        ui::try_popup_item(self.window(), label, AccessibleRole::Button).map(|row| bounds(&row))
    }

    fn authors(&self) -> Vec<String> {
        self.view()
            .get_detail_authors()
            .iter()
            .map(|line| line.to_string())
            .collect()
    }

    /// Whether the dialog's button with this label is enabled.
    fn enabled(&self, label: &str) -> bool {
        ui::popup_item(self.window(), label, AccessibleRole::Button)
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .is_some_and(|value| value == "true")
    }

    /// The open mark at the trailing end of a library's row.
    fn open_mark(&self, row: &str) -> LogicalPosition {
        let item = ui::popup_item(self.window(), row, AccessibleRole::Button);
        let rect = item.geometry();
        let at = item.map_to_native_window(rect.origin);
        LogicalPosition::new(at.x + rect.size.width - 10.0, at.y + rect.size.height / 2.0)
    }

    fn requests(&self) -> Vec<LicenseNoticeRequest> {
        self.commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::ReadLicenseNotice(request) => Some((**request).clone()),
                _ => None,
            })
            .collect()
    }

    /// Deliver the backend's answer the way the application's event loop does.
    fn answer(&mut self, notice: LicenseNotice) {
        owned_commands::run(&self.presenter, &self.begin, |presenter, send| {
            presenter.apply_with_sender(Event::LicenseNoticeReady(Box::new(notice)), send)
        });
        midnight_ui::refresh(self.window(), &self.presenter.borrow());
        self.ui.draw();
    }
}

#[test]
fn about_opens_in_the_main_window_and_both_tabs_render() {
    let mut about = About::open(1000, 800);
    assert!(ui::has_popup(about.window()));
    render::snapshot(&about.ui, "about-tab");
    about.licenses();
    assert!(about.rows().iter().any(|row| row == "slint 1.18.1"));
    let summary = about.view().get_summary();
    assert!(summary.ends_with(" libraries and components"), "{summary}");
    render::snapshot(&about.ui, "about-licenses");
}

#[test]
fn dragging_the_sky_moves_the_dialog_and_close_dismisses_it() {
    let mut test = ui::TestUi::new();
    test.resize(1000, 800);
    test.window.invoke_show_about();
    test.draw();
    let origin = |test: &ui::TestUi| {
        let item = ui::popup_item(&test.window, "Close", AccessibleRole::Button);
        item.map_to_native_window(item.geometry().origin)
    };
    let before = origin(&test);
    let title = ui::popup_item(&test.window, "Midnight Explorer", AccessibleRole::Text);
    let rect = title.geometry();
    let at = title.map_to_native_window(rect.origin);
    let start = LogicalPosition::new(at.x + rect.size.width / 2.0, at.y + rect.size.height / 2.0);
    let left = PointerEventButton::Left;
    ui::pointer_button(&test.window, start, left, true);
    for step in 1..=4 {
        let position =
            LogicalPosition::new(start.x - 10.0 * step as f32, start.y + 8.0 * step as f32);
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    ui::pointer_button(
        &test.window,
        LogicalPosition::new(start.x - 40.0, start.y + 32.0),
        left,
        false,
    );
    test.draw();
    let after = origin(&test);
    assert!(
        (after.x - before.x + 40.0).abs() < 2.0 && (after.y - before.y - 32.0).abs() < 2.0,
        "moved from {before:?} to {after:?}"
    );
    ui::click_popup(&test.window, "Close", AccessibleRole::Button);
    test.draw();
    assert!(!test.window.get_about_open());
}

#[test]
fn a_library_detail_requests_its_notice_from_the_backend_and_shows_the_answer() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.filter("zip");
    about.double_click("zip 8.6.0");
    assert!(about.view().get_detail_open());
    let requests = about.requests();
    let [request] = requests.as_slice() else {
        panic!("one notice request: {requests:?}")
    };
    assert_eq!(
        (
            request.name.as_str(),
            request.version.as_str(),
            &request.folder
        ),
        ("zip", "8.6.0", &None)
    );
    assert_eq!(about.view().get_detail_title(), "zip 8.6.0");
    assert!(about.view().get_detail_license().contains("MIT"));
    assert!(about.view().get_detail_text().starts_with("Loading"));
    // An answer to another request changes nothing.
    about.answer(LicenseNotice {
        id: midnight_core::LicenseNoticeId(request.id.0 + 1),
        text: "stale".into(),
        ..LicenseNotice::default()
    });
    assert!(about.view().get_detail_text().starts_with("Loading"));
    about.answer(LicenseNotice {
        id: request.id,
        authors: vec!["Ada Lovelace <ada@example.org>".into()],
        text: "──── LICENSE ────\n\nPermission is hereby granted".into(),
        truncated: false,
    });
    assert_eq!(about.view().get_detail_authors_heading(), "Authors");
    assert_eq!(about.authors(), ["Ada Lovelace <ada@example.org>"]);
    let shown = "──── LICENSE ────\n\nPermission is hereby granted";
    assert_eq!(about.view().get_detail_text(), shown);
    assert!(ui::try_popup_item(about.window(), shown, AccessibleRole::Text).is_some());
    render::snapshot(&about.ui, "about-license-detail");
    about.click("Back", AccessibleRole::Button);
    assert!(!about.view().get_detail_open());
    assert!(about.rows().iter().any(|row| row == "zip 8.6.0"));
    // A component whose notices ship with its plugin needs no request.
    about.filter("ffmpeg");
    about.double_click("FFmpeg 9.0.2");
    assert_eq!(about.requests().len(), 1);
    assert!(
        about
            .view()
            .get_detail_text()
            .contains("notices/ffmpeg-9.0.2/")
    );
    assert_eq!(about.authors(), ["The FFmpeg developers"]);
}

#[test]
fn back_escape_and_tab_switches_survive_a_shown_tooltip() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.filter("zip");
    about.double_click("zip 8.6.0");
    let back = about.hover_until_hint("Back", AccessibleRole::Button);
    ui::click(about.window(), back.x, back.y);
    about.ui.draw();
    assert!(!about.view().get_detail_open());
    assert_eq!(about.tooltips(), 0);
    about.double_click("zip 8.6.0");
    about.hover_until_hint("Back", AccessibleRole::Button);
    about.key(Key::Escape);
    assert!(!about.view().get_detail_open());
    // A keyboard tab switch drops the filter while its own tooltip shows.
    about.click("Licenses", AccessibleRole::Tab);
    about.hover_until_hint("Filter libraries", AccessibleRole::TextInput);
    about.key(Key::LeftArrow);
    assert!(
        ui::try_popup_item(
            about.window(),
            "Filter libraries",
            AccessibleRole::TextInput
        )
        .is_none()
    );
    assert!(about.window().get_about_open());
}

#[test]
fn escape_still_works_after_the_focused_control_is_destroyed() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.filter("zip");
    about.double_click("zip 8.6.0");
    assert!(about.view().get_detail_open());
    about.key(Key::Escape);
    assert!(!about.view().get_detail_open(), "Escape leaves the library");
    // Clicking the About tab while the filter has focus.
    about.click("Filter libraries", AccessibleRole::TextInput);
    about.click("About", AccessibleRole::Tab);
    assert!(
        ui::try_popup_item(
            about.window(),
            "Filter libraries",
            AccessibleRole::TextInput
        )
        .is_none()
    );
    about.key(Key::Escape);
    assert!(!about.window().get_about_open(), "Escape closes About");
}

#[test]
fn the_filter_text_stays_with_the_list_it_filtered() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.filter("zip");
    // Names and licenses match: bzip2, libbz2-rs-sys (bzip2-1.0.6) and zip.
    let filtered = about.rows();
    assert!(
        filtered.contains(&"zip 8.6.0".to_owned()) && filtered.len() < 10,
        "{filtered:?}"
    );
    let summary = about.view().get_summary();
    assert!(summary.contains(" of "), "{summary}");
    assert_eq!(about.filter_text(), "zip");
    about.double_click(&filtered[0]);
    about.click("Back", AccessibleRole::Button);
    assert_eq!(about.filter_text(), "zip");
    about.click("About", AccessibleRole::Tab);
    about.click("Licenses", AccessibleRole::Tab);
    assert_eq!(about.filter_text(), "zip");
    about.click("Close", AccessibleRole::Button);
    assert!(!about.window().get_about_open());
    about.window().invoke_show_about();
    about.ui.draw();
    about.licenses();
    assert_eq!(about.filter_text(), "zip");
    assert_eq!(about.rows(), filtered);
    assert_eq!(about.view().get_summary(), summary);
}

#[test]
fn the_tabs_and_the_list_work_from_the_keyboard() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.key(Key::LeftArrow);
    assert!(
        ui::try_popup_item(
            about.window(),
            "Filter libraries",
            AccessibleRole::TextInput
        )
        .is_none()
    );
    about.key(Key::RightArrow);
    about.click("Filter libraries", AccessibleRole::TextInput);
    about.type_text("zip");
    about.key(Key::DownArrow);
    about.key(Key::DownArrow);
    assert_eq!(about.view().get_current(), 1);
    let expected = about.rows()[1].clone();
    about.key(Key::Return);
    assert!(about.view().get_detail_open());
    let requests = about.requests();
    let request = requests.last().unwrap();
    assert_eq!(format!("{} {}", request.name, request.version), expected);
    about.key(Key::Escape);
    // Enter in the filter opens the keyboard row as well.
    about.click("Filter libraries", AccessibleRole::TextInput);
    about.key(Key::Return);
    assert!(about.view().get_detail_open());
    assert_eq!(about.requests().len(), 2);
}

#[test]
fn rows_grow_with_the_font() {
    let mut about = About::open(1000, 800);
    about.window().invoke_apply_fonts("".into(), 26, 26);
    about.ui.draw();
    about.licenses();
    let first = about.view().get_rows().row_data(0).unwrap();
    let row = ui::popup_item(
        about.window(),
        &format!("{} {}", first.name, first.version),
        AccessibleRole::Button,
    )
    .geometry()
    .height();
    // Rows show the 25 px small font, whose lines need about 30 px.
    assert!(row >= 32.0, "row height {row}");
}

#[test]
fn close_stays_in_view_in_the_smallest_window() {
    let mut about = About::open(320, 200);
    let bottom = |about: &About| {
        let close = ui::popup_item(about.window(), "Close", AccessibleRole::Button);
        let rect = close.geometry();
        close.map_to_native_window(rect.origin).y + rect.height()
    };
    assert!(bottom(&about) <= 200.0, "Close ends at {}", bottom(&about));
    about.licenses();
    assert!(bottom(&about) <= 200.0, "Close ends at {}", bottom(&about));
    about.click("Close", AccessibleRole::Button);
    assert!(!about.window().get_about_open());
}

#[test]
fn a_selected_library_opens_from_view_license_or_its_open_mark() {
    let mut about = About::open(1000, 800);
    about.licenses();
    // The list says how to open a license, and nothing is selected yet.
    let how =
        "Select a library, then double-click it or press Enter to see its license and authors.";
    assert!(ui::try_popup_item(about.window(), how, AccessibleRole::Text).is_some());
    assert_eq!(about.view().get_current(), -1);
    assert!(!about.enabled("View license"));
    about.filter("zip");
    {
        let row = ui::popup_item(about.window(), "zip 8.6.0", AccessibleRole::Button);
        let description = row
            .accessible_string_property(AccessibleStringProperty::Description)
            .unwrap_or_default();
        assert_eq!(description, "MIT. Shows its license and authors.");
        // The row and its open mark take a click under a hand pointer.
        let hand = MouseCursorInner::BuiltIn(BuiltInMouseCursor::Pointer);
        assert_eq!(touch_area_cursors(&row), [hand.clone(), hand]);
    }
    // A click only selects the row; View license then opens it.
    about.click("zip 8.6.0", AccessibleRole::Button);
    assert!(!about.view().get_detail_open());
    assert_eq!(
        about.rows()[about.view().get_current() as usize],
        "zip 8.6.0"
    );
    let selected = ui::popup_item(about.window(), "zip 8.6.0", AccessibleRole::Button)
        .accessible_string_property(AccessibleStringProperty::ItemSelected);
    assert_eq!(selected.as_deref(), Some("true"));
    assert!(about.enabled("View license"));
    about.click("View license", AccessibleRole::Button);
    assert!(about.view().get_detail_open());
    assert_eq!(about.view().get_detail_title(), "zip 8.6.0");
    about.click("Back", AccessibleRole::Button);
    // One click on another row's open mark opens that row.
    let other = about
        .rows()
        .into_iter()
        .find(|row| row != "zip 8.6.0")
        .expect("the filter matches more than zip");
    let mark = about.open_mark(&other);
    ui::click(about.window(), mark.x, mark.y);
    about.ui.draw();
    assert!(about.view().get_detail_open());
    assert_eq!(about.view().get_detail_title(), other);
    let opened: Vec<String> = about
        .requests()
        .iter()
        .map(|request| format!("{} {}", request.name, request.version))
        .collect();
    assert_eq!(opened, ["zip 8.6.0".to_owned(), other]);
}

/// The pointer shapes of the touch areas directly inside `item`.
fn touch_area_cursors(item: &ItemRc) -> Vec<MouseCursorInner> {
    let mut cursors = Vec::new();
    let mut child = item.first_child();
    while let Some(next) = child {
        if let Some(area) = next.downcast::<TouchArea>() {
            cursors.push(area.as_pin_ref().mouse_cursor());
        }
        child = next.next_sibling();
    }
    cursors
}

/// An item's bounds in window coordinates.
fn bounds(item: &ItemRc) -> LogicalRect {
    let rect = item.geometry();
    LogicalRect::new(item.map_to_native_window(rect.origin), rect.size)
}

/// The window bounds of a library row's open mark, its "›" image.
fn mark_bounds(row: &ItemRc) -> LogicalRect {
    fn image(item: ItemRc) -> Option<ItemRc> {
        if item.downcast::<ImageItem>().is_some() || item.downcast::<ClippedImage>().is_some() {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            if let Some(found) = image(next) {
                return Some(found);
            }
        }
        None
    }
    bounds(&image(row.clone()).expect("a library row has an open mark"))
}

/// Whether `inner` lies within `outer`, give or take half a pixel.
fn within(inner: &LogicalRect, outer: &LogicalRect) -> bool {
    inner.min_x() >= outer.min_x() - 0.5
        && inner.min_y() >= outer.min_y() - 0.5
        && inner.max_x() <= outer.max_x() + 0.5
        && inner.max_y() <= outer.max_y() + 0.5
}

#[test]
fn the_open_mark_opens_its_library_beside_the_scrollbar_at_every_size() {
    let mut about = About::open(1000, 800);
    about.licenses();
    // The whole inventory scrolls, so the list's 14 px overlay scrollbar covers the
    // end of every row; the mark must stay clear of it to take its click.
    let themes = midnight_core::PreferenceTheme::ALL.len() as i32;
    for (width, height, density, theme, font) in [
        (1000, 800, 0, 0, 12),
        (1000, 800, 1, 1 % themes, 15),
        (480, 640, 1, 2 % themes, 15),
        // So narrow that the version and license columns give way to the mark.
        (360, 760, 0, 3 % themes, 26),
    ] {
        about.window().set_theme_index(theme);
        about.window().set_density_index(density);
        about.window().invoke_apply_fonts("".into(), font, font);
        about.ui.resize(width, height);
        about.ui.draw();
        let case = format!("{width}x{height}, density {density}, theme {theme}, font {font}");
        let list = about.list();
        // A row shown whole, below the first one.
        let label = about
            .labels()
            .into_iter()
            .take(40)
            .filter(|label| {
                about
                    .row_bounds(label)
                    .is_some_and(|row| within(&row, &list))
            })
            .nth(1)
            .unwrap_or_else(|| panic!("{case}: the list shows two whole rows"));
        let (row, mark) = {
            let item = ui::popup_item(about.window(), &label, AccessibleRole::Button);
            (bounds(&item), mark_bounds(&item))
        };
        assert!(
            within(&mark, &row),
            "{case}: the mark {mark:?} leaves its row {row:?}"
        );
        assert!(
            mark.max_x() <= list.max_x() - 14.0,
            "{case}: the mark {mark:?} reaches under the scrollbar of the list {list:?}"
        );
        // The hovered mark beside the scrollbar and, from the case before, a selected row.
        let position = LogicalPosition::new(mark.center().x, mark.center().y);
        about
            .window()
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        about.ui.draw();
        render::snapshot(
            &about.ui,
            &format!("about-open-mark-{width}x{height}-density-{density}"),
        );
        for x in [mark.center().x, mark.min_x() + 1.0] {
            ui::click(about.window(), x, mark.center().y);
            about.ui.draw();
            assert!(
                about.view().get_detail_open(),
                "{case}: a click at x {x} on the mark {mark:?} of {label:?} opens nothing"
            );
            assert_eq!(about.view().get_detail_title(), label, "{case}");
            about.click("Back", AccessibleRole::Button);
            assert!(!about.view().get_detail_open(), "{case}");
        }
    }
}

#[test]
fn end_and_the_other_keys_keep_the_selected_library_whole_in_view() {
    let mut about = About::open(1000, 800);
    about.licenses();
    let labels = about.labels();
    let count = labels.len();
    assert!(count > 100, "{count} libraries");
    // Shorter windows shorten the dialog and its list, which then shows a part of a
    // row below its whole ones at most of these heights.
    for height in [640, 627, 611, 598] {
        about.ui.resize(1000, height);
        about.ui.draw();
        if about.row_bounds(&labels[0]).is_none() {
            about.key(Key::Home);
        }
        // Past the double-click interval: the first row is where it was clicked before.
        ui::advance(&about.clock, Duration::from_secs(1));
        about.click(&labels[0], AccessibleRole::Button);
        assert!(!about.view().get_detail_open());
        assert_eq!(about.view().get_current(), 0);
        for (key, expected) in [
            (Key::End, Some(count - 1)),
            (Key::UpArrow, Some(count - 2)),
            (Key::Home, Some(0)),
            (Key::PageDown, None),
            (Key::End, Some(count - 1)),
            (Key::PageUp, None),
            (Key::DownArrow, None),
        ] {
            about.key(key);
            about.ui.draw();
            let current = about.view().get_current() as usize;
            if let Some(expected) = expected {
                assert_eq!(current, expected, "{key:?} at height {height}");
            }
            let label = &labels[current];
            let list = about.list();
            let row = about
                .row_bounds(label)
                .unwrap_or_else(|| panic!("{key:?} at height {height}: {label} is not shown"));
            assert!(
                within(&row, &list),
                "{key:?} at height {height}: {label} at {row:?} is cut off by the list {list:?}"
            );
        }
    }
}

#[test]
fn a_library_without_a_version_is_named_by_its_name_alone() {
    let mut about = About::open(1000, 800);
    about.licenses();
    about.filter("Adobe CMaps");
    assert_eq!(about.labels(), ["Adobe CMaps"]);
    about.double_click("Adobe CMaps");
    assert!(about.view().get_detail_open());
    assert_eq!(about.view().get_detail_title(), "Adobe CMaps");
}

#[test]
fn the_sky_scenes_stay_clear_of_the_title_icon_and_crescent() {
    let mut about = About::open(1000, 800);
    let image =
        |about: &About, label: &str| ui::popup_item(about.window(), label, AccessibleRole::Image);
    let mut satellite = Vec::new();
    for second in [1_u64, 4, 7, 10, 13, 16, 19, 22] {
        ui::advance(
            &about.clock,
            Duration::from_secs(second).saturating_sub(about.clock.get()),
        );
        about.ui.draw();
        let title = ui::popup_item(about.window(), "Midnight Explorer", AccessibleRole::Text);
        let pill = ui::popup_item(about.window(), "Version 1.2.3", AccessibleRole::Text)
            .parent_item(ParentItemTraversalMode::StopAtPopups)
            .expect("the version pill");
        // The halo glows a few pixels beyond the icon.
        let keep_clear = [
            ("title", bounds(&title)),
            ("version", bounds(&pill)),
            (
                "icon",
                bounds(&image(&about, "Midnight Explorer icon")).inflate(8.0, 8.0),
            ),
            ("crescent", bounds(&image(&about, "Crescent moon"))),
        ];
        let dot = image(&about, "Satellite")
            .first_child()
            .expect("the satellite itself");
        satellite.push(bounds(&dot).origin);
        let scenes = [
            ("Cassiopeia", bounds(&image(&about, "Cassiopeia"))),
            ("Owl", bounds(&image(&about, "Owl"))),
            ("Satellite", bounds(&dot)),
        ];
        for (scene, rect) in scenes {
            for (area, clear) in &keep_clear {
                assert!(
                    !rect.intersects(clear),
                    "at {second} s {scene} {rect:?} meets the {area} {clear:?}"
                );
            }
        }
        for scene in ["Cassiopeia", "Owl", "Satellite"] {
            assert!(image(&about, scene).is_visible(), "{scene} at {second} s");
        }
    }
    // The satellite crosses the sky in one direction, high above the title.
    assert!(
        satellite.windows(2).all(|pair| pair[1].x >= pair[0].x),
        "{satellite:?}"
    );
    assert!(satellite.first().unwrap().x < satellite.last().unwrap().x);
    // A short dialog keeps only the title.
    about.ui.resize(1000, 300);
    for scene in ["Cassiopeia", "Owl", "Satellite", "Midnight Explorer icon"] {
        assert!(
            !image(&about, scene).is_visible(),
            "{scene} in a short dialog"
        );
    }
}
