//! The About dialog lives inside the main window, can be dragged by its sky and
//! lists the third-party libraries; a library's notices come from the backend. Its
//! night sky loops every 72 s and holds still while the system asks for less motion,
//! Saturn and the other scenes keep clear of the title and of each other at every
//! size, and the version pill holds its text at every font size.
#[path = "support/owned_commands.rs"]
mod owned_commands;
#[path = "support/render.rs"]
mod render;
use crate::support::ui;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    cursor::MouseCursorInner,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{
        AccessibleRole, BuiltInMouseCursor, ClippedImage, ComplexText, Flickable, ImageItem, Item,
        Opacity, Path, SimpleText, TouchArea,
    },
    layout::Orientation,
    lengths::LogicalRect,
    window::{WindowInner, WindowKind},
};
use midnight_core::{Command, Event, LicenseNotice, LicenseNoticeRequest};
use midnight_ui::{AboutView, BeginOwned, MainWindow, Presenter};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, PointerEventButton, WindowEvent, software_renderer::RepaintBufferType},
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
fn the_sky_scenes_stay_clear_of_the_title_icon_and_saturn() {
    let mut about = About::open(1000, 800);
    let image =
        |about: &About, label: &str| ui::popup_item(about.window(), label, AccessibleRole::Image);
    // The icon is the Moon: the sky shows no second one.
    assert!(
        ui::try_popup_item(about.window(), "Crescent moon", AccessibleRole::Image).is_none(),
        "a crescent moon beside the Moon icon"
    );
    let mut satellite = Vec::new();
    // Eight moments across the sky's 72 s loop, every 9 s from 3 s.
    for second in [3_u64, 12, 21, 30, 39, 48, 57, 66] {
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
            ("Saturn", bounds(&image(&about, "Saturn"))),
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
    for scene in [
        "Cassiopeia",
        "Owl",
        "Satellite",
        "Saturn",
        "Midnight Explorer icon",
    ] {
        assert!(
            !image(&about, scene).is_visible(),
            "{scene} in a short dialog"
        );
    }
}

/// The sky at the top of the dialog: the item that holds the title and the scenes.
fn about_sky(window: &MainWindow) -> ItemRc {
    ui::popup_item(window, "Midnight Explorer", AccessibleRole::Text)
        .parent_item(ParentItemTraversalMode::StopAtPopups)
        .expect("the sky holds the title")
}

/// Calls `f` for `item` and every item inside it.
fn visit(item: &ItemRc, f: &mut impl FnMut(&ItemRc)) {
    f(item);
    let mut child = item.first_child();
    while let Some(next) = child {
        visit(&next, f);
        child = next.next_sibling();
    }
}

/// Whether `item` is a shooting star: one of the sky's 64 × 30 streak paths.
fn is_streak(item: &ItemRc) -> bool {
    item.downcast::<Path>().is_some() && {
        let size = item.geometry().size;
        (size.width - 64.0).abs() < 0.5 && (size.height - 30.0).abs() < 0.5
    }
}

/// The window bounds of the shooting stars, wherever they are in their flight or
/// between flights.
fn streaks(sky: &ItemRc) -> Vec<LogicalRect> {
    let mut found = Vec::new();
    visit(sky, &mut |item| {
        if is_streak(item) {
            found.push(bounds(item));
        }
    });
    found
}

/// Draw the whole window afresh: a partly repainted frame keeps anti-aliased edges
/// from the frame before it, so it depends on what was drawn earlier.
fn draw_afresh(test: &mut ui::TestUi) {
    slint::platform::update_timers_and_animations();
    test.native.request_redraw();
    test.native.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(&mut test.pixels, test.width);
        renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
    });
}

/// The pixels of a window rectangle in the last frame drawn.
fn pixels(test: &ui::TestUi, rect: LogicalRect) -> Vec<slint::Rgb8Pixel> {
    let columns = rect.min_x().max(0.0) as usize..(rect.max_x() as usize).min(test.width);
    let rows = rect.min_y().max(0.0) as usize..(rect.max_y() as usize).min(test.height);
    rows.flat_map(|y| {
        columns
            .clone()
            .map(move |x| test.pixels[y * test.width + x])
    })
    .collect()
}

/// Drag the About dialog's bottom right resize corner by `dx`, `dy`.
fn resize_about(about: &mut About, dx: f32, dy: f32) {
    let corner = {
        let inner = WindowInner::from_pub(about.window().window());
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("About's popup");
        let diagonal = MouseCursorInner::BuiltIn(BuiltInMouseCursor::NwseResize);
        let mut corners = Vec::new();
        visit(&ItemRc::new_root(popup.component.clone()), &mut |item| {
            if item
                .downcast::<TouchArea>()
                .is_some_and(|area| area.as_pin_ref().mouse_cursor() == diagonal)
            {
                corners.push(bounds(item));
            }
        });
        corners
            .into_iter()
            .max_by(|a, b| a.min_y().total_cmp(&b.min_y()))
            .expect("the dialog's resize corners")
    };
    let start = LogicalPosition::new(corner.center().x, corner.center().y);
    let end = LogicalPosition::new(start.x + dx, start.y + dy);
    ui::pointer_button(about.window(), start, PointerEventButton::Left, true);
    about
        .window()
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: end });
    ui::pointer_button(about.window(), end, PointerEventButton::Left, false);
    about.ui.draw();
}

#[test]
fn the_sky_loops_every_72_seconds() {
    let mut about = About::open(1000, 800);
    let area = bounds(&about_sky(about.window()));
    // The sky 3 s into its loop, then 24, 36 and 72 s later.
    let mut frames = Vec::new();
    for second in [3_u64, 27, 39, 75] {
        ui::advance(
            &about.clock,
            Duration::from_secs(second).saturating_sub(about.clock.get()),
        );
        draw_afresh(&mut about.ui);
        render::snapshot(&about.ui, &format!("about-sky-at-{second}s"));
        frames.push(pixels(&about.ui, area));
    }
    let changed = |later: usize| {
        frames[0]
            .iter()
            .zip(&frames[later])
            .filter(|(first, then)| first != then)
            .count()
    };
    assert_eq!(changed(3), 0, "the sky 72 s on is not where it was");
    // Every shorter loop that repeats after 72 s repeats after 24 or 36 s as well.
    for (later, seconds) in [(1, 24), (2, 36)] {
        assert!(
            changed(later) > 200,
            "the sky {seconds} s on is where it was: its loop is shorter than 72 s"
        );
    }
}

#[test]
fn saturn_keeps_clear_of_the_title_and_the_shooting_stars_at_every_size() {
    let mut about = About::open(1000, 800);
    let mut problems = Vec::new();
    // The window, how far the dialog's corner is dragged, the sky that gives and whether
    // Saturn shows: the dialog at its smallest (280 × 180) and in the smallest window,
    // whose 40 px sky keeps only the title; the narrowest dialog with the whole sky
    // (280 × 660); the default size (560 × 660); and the dialog dragged out to 1260 px.
    for (width, height, corner, (sky_width, sky_height), shown) in [
        (1000, 800, Some((-280.0, -480.0)), (280.0, 40.0), false),
        (320, 200, None, (312.0, 40.0), false),
        (1000, 800, Some((-280.0, 0.0)), (280.0, 176.0), true),
        (1000, 800, None, (560.0, 176.0), true),
        (2000, 900, Some((700.0, 0.0)), (1260.0, 176.0), true),
    ] {
        // Reopened, About starts at its default size.
        about.window().set_about_open(false);
        about.ui.resize(width, height);
        assert!(!ui::has_popup(about.window()));
        about.window().invoke_show_about();
        about.ui.draw();
        if let Some((dx, dy)) = corner {
            resize_about(&mut about, dx, dy);
        }
        let case = format!("the {sky_width} × {sky_height} sky");
        render::snapshot(&about.ui, &format!("about-sky-{sky_width}x{sky_height}"));
        let sky = about_sky(about.window());
        let area = bounds(&sky);
        if (area.width() - sky_width).abs() > 0.5 || (area.height() - sky_height).abs() > 0.5 {
            problems.push(format!("{case} is {area:?}"));
            continue;
        }
        let Some(saturn) = ui::try_popup_item(about.window(), "Saturn", AccessibleRole::Image)
        else {
            problems.push(format!("{case}: no Saturn"));
            continue;
        };
        if saturn.is_visible() != shown {
            problems.push(format!("{case}: Saturn shown {}", saturn.is_visible()));
        }
        if !shown {
            continue;
        }
        let planet = bounds(&saturn);
        if !within(&planet, &area) {
            problems.push(format!("{case}: Saturn {planet:?} leaves the sky {area:?}"));
        }
        let image = |label: &str| ui::popup_item(about.window(), label, AccessibleRole::Image);
        let title = ui::popup_item(about.window(), "Midnight Explorer", AccessibleRole::Text);
        let pill = ui::popup_item(about.window(), "Version 1.2.3", AccessibleRole::Text)
            .parent_item(ParentItemTraversalMode::StopAtPopups)
            .expect("the version pill");
        // The halo glows a few pixels beyond the icon.
        for (name, rect) in [
            ("the title", bounds(&title)),
            ("the version", bounds(&pill)),
            (
                "the icon",
                bounds(&image("Midnight Explorer icon")).inflate(8.0, 8.0),
            ),
        ] {
            if rect.intersects(&planet) {
                problems.push(format!("{case}: Saturn {planet:?} meets {name} {rect:?}"));
            }
        }
        // Every half second of the loop: the shooting stars wherever they fly, and the
        // scenes of a roomy sky.
        let mut met = Vec::new();
        for step in 1..=144_u64 {
            ui::advance(&about.clock, Duration::from_millis(500));
            let mut moving = streaks(&sky)
                .into_iter()
                .map(|rect| ("a shooting star", rect))
                .collect::<Vec<_>>();
            if moving.len() != 3 {
                problems.push(format!("{case}: {} shooting stars", moving.len()));
                break;
            }
            for scene in ["Cassiopeia", "Owl", "Satellite"] {
                let item = image(scene);
                if item.is_visible() {
                    // The satellite's container spans its whole crossing.
                    let shape = if scene == "Satellite" {
                        item.first_child().expect("the satellite itself")
                    } else {
                        item
                    };
                    moving.push((scene, bounds(&shape)));
                }
            }
            for (name, rect) in moving {
                if rect.intersects(&planet) && !met.contains(&name) {
                    met.push(name);
                    let at = step as f32 / 2.0;
                    problems.push(format!(
                        "{case}: at {at} s Saturn {planet:?} meets {name} {rect:?}"
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn shooting_stars_pass_in_front_of_the_moon() {
    let about = About::open(1000, 800);
    let sky = about_sky(about.window());
    let moon = ui::popup_item(
        about.window(),
        "Midnight Explorer icon",
        AccessibleRole::Image,
    );
    // The sky draws its items in tree order, so a later item is in front.
    let mut order = Vec::new();
    visit(&sky, &mut |item| order.push(item.clone()));
    let icon = order
        .iter()
        .position(|item| *item == moon)
        .expect("the icon is in the sky");
    let meteors: Vec<usize> = order
        .iter()
        .enumerate()
        .filter(|(_, item)| is_streak(item))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(meteors.len(), 3, "the shooting stars");
    assert!(
        meteors.iter().all(|&at| at > icon),
        "shooting stars {meteors:?} are drawn behind the Moon, item {icon}"
    );
    // At the default size the right one's path does cross the Moon.
    let disc = bounds(&moon);
    let mut crossed = false;
    for _ in 0..48 {
        ui::advance(&about.clock, Duration::from_millis(500));
        crossed |= streaks(&sky).iter().any(|streak| streak.intersects(&disc));
    }
    assert!(crossed, "no shooting star crosses the Moon {disc:?}");
}

#[test]
fn saturn_is_drawn_the_same_in_every_theme() {
    let mut about = About::open(1000, 800);
    let planet = bounds(&ui::popup_item(
        about.window(),
        "Saturn",
        AccessibleRole::Image,
    ));
    let mut draw = |theme: i32| {
        about.window().set_theme_index(theme);
        draw_afresh(&mut about.ui);
        // The planet's middle is lit gold against the midnight sky.
        let middle = about.ui.pixel(planet.center().x, planet.center().y);
        assert!(
            middle.r > 150 && middle.g > 110 && middle.b < middle.r,
            "theme {theme}: the planet's middle is {middle:?}"
        );
        // The planet is round, here drawn by the software renderer: the corners of its
        // 16 px square that the rings miss show the blue sky, not its gold.
        for (x, y) in [(16.5, 16.5), (31.5, 31.5)] {
            let corner = about.ui.pixel(planet.min_x() + x, planet.min_y() + y);
            assert!(
                corner.b > corner.r,
                "theme {theme}: the planet's corner at {x}, {y} is {corner:?}"
            );
        }
        pixels(&about.ui, planet)
    };
    let first = draw(0);
    for theme in 1..midnight_core::PreferenceTheme::ALL.len() as i32 {
        assert!(
            draw(theme) == first,
            "theme {theme} draws Saturn differently"
        );
    }
}

/// What the About tab shows under the version: the author's label and name, and the
/// copyright line under the name.
const AUTHOR: &str = "Author";
const AUTHOR_NAME: &str = "Tomas Jelinek";
const COPYRIGHT: &str = "© 2026 Tomas Jelinek";

/// A text's colour.
fn text_color(item: &ItemRc) -> slint::Color {
    match item.downcast::<SimpleText>() {
        Some(text) => text.as_pin_ref().color().color(),
        None => {
            let text = item.downcast::<ComplexText>().expect("a text");
            text.as_pin_ref().color().color()
        }
    }
}

/// A text's preferred extent along `orientation`, given `cross` across it (-1: any).
fn preferred(item: &ItemRc, orientation: Orientation, cross: f32) -> f32 {
    let adapter = item.window_adapter().expect("a shown text");
    match item.downcast::<SimpleText>() {
        Some(text) => {
            let text = text.as_pin_ref();
            text.layout_info(orientation, cross, &adapter, item)
                .preferred
        }
        None => {
            let text = item.downcast::<ComplexText>().expect("a text");
            let text = text.as_pin_ref();
            text.layout_info(orientation, cross, &adapter, item)
                .preferred
        }
    }
}

/// The WCAG contrast between the lightest and the darkest pixel of a rectangle in the
/// last frame drawn: a text's against its background.
fn contrast(test: &ui::TestUi, rect: LogicalRect) -> f32 {
    let luminance = |pixel: &slint::Rgb8Pixel| {
        let linear = |channel: u8| {
            let value = f32::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(pixel.r) + 0.7152 * linear(pixel.g) + 0.0722 * linear(pixel.b)
    };
    let (darkest, lightest) = pixels(test, rect)
        .iter()
        .map(luminance)
        .fold((f32::MAX, f32::MIN), |(low, high), value| {
            (low.min(value), high.max(value))
        });
    (lightest + 0.05) / (darkest + 0.05)
}

/// The scroll view that holds `item`, if any.
fn scroll_view(item: &ItemRc) -> Option<ItemRc> {
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            return Some(ancestor);
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

/// Scroll `item` into its scroll view with the wheel, as a user does.
fn reveal(about: &mut About, item: &ItemRc) {
    let Some(view) = scroll_view(item) else {
        return;
    };
    let (shown, area) = (bounds(&view), bounds(item));
    let delta = if area.min_y() < shown.min_y() {
        shown.min_y() - area.min_y()
    } else if area.max_y() > shown.max_y() {
        shown.max_y() - area.max_y()
    } else {
        0.0
    };
    if delta != 0.0 {
        about
            .window()
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new(shown.min_x() + 4.0, shown.min_y() + 4.0),
                delta_x: 0.0,
                delta_y: delta,
            });
        about.ui.draw();
    }
}

#[test]
fn the_about_tab_names_its_author_and_copyright_in_every_theme() {
    let mut about = About::open(1000, 800);
    let text =
        |about: &About, name: &str| ui::popup_item(about.window(), name, AccessibleRole::Text);
    let (version_label, version_value) = (text(&about, "Version"), text(&about, "1.2.3"));
    let (label, name, line) = (
        bounds(&text(&about, AUTHOR)),
        bounds(&text(&about, AUTHOR_NAME)),
        bounds(&text(&about, COPYRIGHT)),
    );
    // Under the version: the author's label beside the name, in the version's columns,
    // then the copyright line under the name, then Built with.
    let version = bounds(&version_label);
    assert!(
        label.min_y() > version.max_y() - 0.5,
        "{AUTHOR} {label:?} is not below the version {version:?}"
    );
    assert!(
        (label.min_y() - name.min_y()).abs() < 0.5,
        "{AUTHOR} {label:?} and {AUTHOR_NAME} {name:?} are not one row"
    );
    let value_column = bounds(&version_value).min_x();
    assert!(
        (label.min_x() - version.min_x()).abs() < 0.5
            && (name.min_x() - value_column).abs() < 0.5
            && (line.min_x() - value_column).abs() < 0.5,
        "the author's rows leave the version's columns"
    );
    assert!(
        line.min_y() > name.max_y() - 0.5,
        "{COPYRIGHT} {line:?} is not under {AUTHOR_NAME} {name:?}"
    );
    let next = bounds(&text(&about, "Built with"));
    assert!(
        next.min_y() > line.max_y() - 0.5,
        "Built with {next:?} is above {COPYRIGHT} {line:?}"
    );
    // In every theme they take the version's colours and contrast at least as well.
    for theme in 0..midnight_core::PreferenceTheme::ALL.len() as i32 {
        about.window().set_theme_index(theme);
        draw_afresh(&mut about.ui);
        for (shown, like) in [
            (AUTHOR, &version_label),
            (AUTHOR_NAME, &version_value),
            (COPYRIGHT, &version_value),
        ] {
            let item = text(&about, shown);
            assert_eq!(
                text_color(&item),
                text_color(like),
                "theme {theme}: the colour of {shown}"
            );
            let reached = contrast(&about.ui, bounds(&item));
            let reference = contrast(&about.ui, bounds(like));
            assert!(
                reached >= reference.min(4.5) - 0.05,
                "theme {theme}: {shown} at {reached:.2}:1, the version at {reference:.2}:1"
            );
        }
    }
}

#[test]
fn the_author_and_copyright_stay_readable_at_the_smallest_size_and_large_fonts() {
    let mut about = About::open(1000, 800);
    let mut problems = Vec::new();
    // Density, desktop and touch font sizes and the drag of the dialog's corner: the
    // dialog at its smallest (280 × 180) with each density's default font, and at its
    // narrowest (280 × 660) with the largest font.
    for (density, desktop, touch, (dx, dy)) in [
        (0, 12, 15, (-280.0, -480.0)),
        (1, 12, 15, (-280.0, -480.0)),
        (0, 26, 26, (-280.0, 0.0)),
    ] {
        // Reopened, About starts at its default size.
        about.window().set_about_open(false);
        about.ui.draw();
        assert!(!ui::has_popup(about.window()));
        about.window().set_density_index(density);
        about.window().invoke_apply_fonts("".into(), desktop, touch);
        about.window().invoke_show_about();
        about.ui.draw();
        resize_about(&mut about, dx, dy);
        let case = format!("density {density}, fonts {desktop}/{touch}");
        render::snapshot(
            &about.ui,
            &format!("about-author-density-{density}-fonts-{desktop}-{touch}"),
        );
        // A label holds one line; the values wrap.
        for (name, wraps) in [(AUTHOR, false), (AUTHOR_NAME, true), (COPYRIGHT, true)] {
            let Some(item) = ui::try_popup_item(about.window(), name, AccessibleRole::Text) else {
                problems.push(format!("{case}: no {name:?}"));
                continue;
            };
            reveal(&mut about, &item);
            let area = bounds(&item);
            if let Some(view) = scroll_view(&item)
                && !within(&area, &bounds(&view))
            {
                problems.push(format!("{case}: {name:?} {area:?} is not whole in view"));
            }
            let (needs, has) = if wraps {
                let needs = preferred(&item, Orientation::Vertical, area.width());
                (needs, area.height())
            } else {
                (
                    preferred(&item, Orientation::Horizontal, -1.0),
                    area.width(),
                )
            };
            if needs > has + 0.5 {
                problems.push(format!(
                    "{case}: {name:?} needs {needs:.1} px, has {has:.1}"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

/// How opaque an item is drawn: its own and its ancestors' opacity together.
fn opacity(item: &ItemRc) -> f32 {
    let mut shown = 1.0;
    let mut current = Some(item.clone());
    while let Some(next) = current {
        if let Some(layer) = next.downcast::<Opacity>() {
            shown *= layer.as_pin_ref().opacity();
        }
        current = next.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    shown
}

/// The opacity of every layer inside `item` that sets one.
fn layer_opacities(item: &ItemRc) -> Vec<f32> {
    let mut found = Vec::new();
    visit(item, &mut |inner| {
        if inner.downcast::<Opacity>().is_some() {
            found.push(opacity(inner));
        }
    });
    found
}

/// Where every item of the sky is and the sky's pixels, drawn afresh.
fn sky_pose(about: &mut About) -> (Vec<LogicalRect>, Vec<slint::Rgb8Pixel>) {
    draw_afresh(&mut about.ui);
    let sky = about_sky(about.window());
    let mut places = Vec::new();
    visit(&sky, &mut |item| places.push(bounds(item)));
    (places, pixels(&about.ui, bounds(&sky)))
}

#[test]
fn the_sky_holds_still_while_the_system_asks_for_less_motion() {
    let mut about = About::open(1000, 800);
    ui::advance(&about.clock, Duration::from_secs(3));
    let moving = sky_pose(&mut about);
    // The setting changes while About is open: the sky stops at once and stays still.
    about.window().set_reduce_motion(true);
    let still = sky_pose(&mut about);
    assert!(
        still != moving,
        "the sky did not stop when the setting changed"
    );
    for _ in 0..4 {
        ui::advance(&about.clock, Duration::from_secs(7));
        assert!(
            sky_pose(&mut about) == still,
            "the sky moves while the system asks for less motion"
        );
    }
    // Every scene shows in the still sky, a shooting star and the satellite among them,
    // and the title and the version stay readable in front of them.
    let image = |label: &str| ui::popup_item(about.window(), label, AccessibleRole::Image);
    for scene in [
        "Cassiopeia",
        "Owl",
        "Satellite",
        "Saturn",
        "Midnight Explorer icon",
    ] {
        assert!(image(scene).is_visible(), "{scene} in the still sky");
    }
    for text in ["Midnight Explorer", "Version 1.2.3"] {
        let item = ui::popup_item(about.window(), text, AccessibleRole::Text);
        assert!(
            item.is_visible() && opacity(&item) > 0.99,
            "{text} in the still sky"
        );
    }
    let sky = about_sky(about.window());
    let mut meteors = Vec::new();
    visit(&sky, &mut |item| {
        if is_streak(item) {
            meteors.push(opacity(item));
        }
    });
    assert!(
        meteors.iter().any(|&shown| shown > 0.5),
        "no shooting star shows in the still sky: {meteors:?}"
    );
    let satellite = layer_opacities(&image("Satellite"));
    assert!(
        satellite.iter().any(|&shown| shown > 0.5),
        "the satellite is faded out in the still sky: {satellite:?}"
    );
    let mut lines = Vec::new();
    visit(&image("Cassiopeia"), &mut |item| {
        if item.downcast::<Path>().is_some() {
            lines.push((item.is_visible(), opacity(item)));
        }
    });
    assert!(
        !lines.is_empty() && lines.iter().all(|&(drawn, shown)| drawn && shown > 0.3),
        "Cassiopeia's lines are not drawn in the still sky: {lines:?}"
    );
    // It is the moving sky's pose in the middle of its loop, which goes on from there
    // when motion is allowed again, still without reopening About.
    about.window().set_reduce_motion(false);
    ui::advance(
        &about.clock,
        Duration::from_secs(36).saturating_sub(about.clock.get()),
    );
    assert!(
        sky_pose(&mut about) == still,
        "the still sky is not the loop's middle"
    );
    ui::advance(&about.clock, Duration::from_secs(2));
    assert!(
        sky_pose(&mut about) != still,
        "the sky stays still after the setting is cleared"
    );
}

#[test]
fn the_version_pill_holds_its_text_at_every_font_size() {
    let mut about = About::open(1000, 800);
    let mut problems = Vec::new();
    // Each density's default font, then the largest font Options accepts (32 px).
    for (density, font) in [(0, 12), (1, 15), (0, 32), (1, 32)] {
        about.window().set_density_index(density);
        about.window().invoke_apply_fonts("".into(), font, font);
        about.ui.draw();
        let case = format!("density {density}, font {font}");
        render::snapshot(
            &about.ui,
            &format!("about-pill-density-{density}-font-{font}"),
        );
        let text = ui::popup_item(about.window(), "Version 1.2.3", AccessibleRole::Text);
        let pill = text
            .parent_item(ParentItemTraversalMode::StopAtPopups)
            .expect("the version pill");
        let (pill_area, sky_area) = (bounds(&pill), bounds(&about_sky(about.window())));
        // The text's line with 2 px above and below it.
        let needs = preferred(&text, Orientation::Vertical, -1.0) + 4.0;
        if pill_area.height() + 0.5 < needs {
            problems.push(format!(
                "{case}: the pill is {:.1} px tall, its text needs {needs:.1}",
                pill_area.height()
            ));
        }
        if !within(&pill_area, &sky_area) {
            problems.push(format!(
                "{case}: the sky {sky_area:?} cuts the pill {pill_area:?}"
            ));
        }
        // The default desktop font keeps the 20 px pill and the 176 px sky.
        if (density, font) == (0, 12)
            && ((pill_area.height() - 20.0).abs() > 0.5 || (sky_area.height() - 176.0).abs() > 0.5)
        {
            problems.push(format!(
                "{case}: the pill {pill_area:?} or the sky {sky_area:?} changed"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}
