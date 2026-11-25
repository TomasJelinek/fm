//! The Checksums results' multi-selection, its copies, following the row being
//! calculated, and the look of selected and cursor rows.
use super::*;
use i_slint_core::{
    accessibility::AccessibleStringProperty as AccessibleProperty, item_tree::ItemRc,
    items::AccessibleRole, window::WindowInner,
};

type Native = slint::platform::software_renderer::MinimalSoftwareWindow;

/// The shown result rows: label, top and bottom, top to bottom.
fn shown_rows(native: &Native) -> Vec<(String, f32, f32)> {
    fn collect(item: ItemRc, found: &mut Vec<(String, f32, f32)>) {
        if item.accessible_role() == AccessibleRole::ListItem
            && let Some(label) = item.accessible_string_property(AccessibleProperty::Label)
            && label.contains(" · ")
        {
            let top = item.map_to_native_window(item.geometry().origin).y;
            found.push((label.to_string(), top, top + item.geometry().height()));
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            collect(next, found);
        }
    }
    let mut found = Vec::new();
    let root = WindowInner::from_pub(native.window()).component();
    collect(ItemRc::new_root(root), &mut found);
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    found
}

/// The shown result row of the file `name`.
fn result_row(native: &Native, name: &str) -> ItemRc {
    fn find(item: ItemRc, prefix: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::ListItem
            && item
                .accessible_string_property(AccessibleProperty::Label)
                .is_some_and(|label| label.starts_with(prefix))
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            if let Some(found) = find(next, prefix) {
                return Some(found);
            }
        }
        None
    }
    let root = WindowInner::from_pub(native.window()).component();
    find(ItemRc::new_root(root), &format!("{name} · "))
        .unwrap_or_else(|| panic!("no shown row for {name}"))
}

/// The files of the selected rows, as accessibility reports them, top to bottom.
fn selected_files(native: &Native) -> Vec<String> {
    shown_rows(native)
        .into_iter()
        .map(|(label, ..)| label.split(" · ").next().unwrap_or_default().to_owned())
        .filter(|name| {
            result_row(native, name)
                .accessible_string_property(AccessibleProperty::ItemSelected)
                .is_some_and(|selected| selected == "true")
        })
        .collect()
}

/// Click a result row's name, holding these modifiers.
fn click_row(native: &Native, name: &str, modifiers: &[Key]) {
    let row = result_row(native, name);
    let origin = row.map_to_native_window(row.geometry().origin);
    let position =
        slint::LogicalPosition::new(origin.x + 16.0, origin.y + row.geometry().height() / 2.0);
    for modifier in modifiers {
        native.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*modifier).into(),
        });
    }
    native
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        native.window().dispatch_event(event);
    }
    for modifier in modifiers.iter().rev() {
        native.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*modifier).into(),
        });
    }
}

/// The footer's Copy button, whatever its caption says.
fn copy_button(native: &Native) -> ItemRc {
    fn find(item: ItemRc) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleProperty::Label)
                .is_some_and(|label| label.starts_with("Copy") && label.ends_with("SHA-256"))
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            if let Some(found) = find(next) {
                return Some(found);
            }
        }
        None
    }
    let root = WindowInner::from_pub(native.window()).component();
    find(ItemRc::new_root(root)).expect("the Copy button")
}

fn property(item: &ItemRc, which: AccessibleProperty) -> String {
    item.accessible_string_property(which)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// A Checksums window of the shown (temporary) folder after a calculation of `count`
/// files r1.bin…, each with the SHA-256 of 32 bytes of its number.
struct Finished {
    main: MainWindow,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    native: Rc<Native>,
    root: std::path::PathBuf,
}
impl Finished {
    fn new(count: u64) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        let root = std::env::temp_dir();
        let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        midnight_ui::install(
            &main,
            state.clone(),
            Rc::new({
                let commands = commands.clone();
                move |command| {
                    commands.borrow_mut().push(command);
                    true
                }
            }),
        );
        main.show().unwrap();
        midnight_ui::refresh(&main, &state.borrow());
        main.invoke_calculate_checksums(0);
        let native = windows.borrow()[1].clone();
        draw(&native, 1000, 620, "checksum-selection-start");
        let calculate = modeless::accessible_items(&native, AccessibleRole::Button, "Calculate");
        modeless::click_item(&native, calculate.last().unwrap(), "Calculate");
        let (job, generation) = commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::StartSelectedChecksums(request) => Some((request.job, request.generation)),
                _ => None,
            })
            .unwrap();
        let rows: Vec<_> = (1..=count)
            .map(|index| ChecksumRow {
                id: ChecksumRowId(index),
                name: std::path::Path::new(&format!("r{index}.bin")).into(),
                identity: None,
                kind: Some(EntryKind::File),
                state: ChecksumRowState::Calculated,
                bytes_read: 3,
                digests: ChecksumDigests::new(&[(ChecksumAlgorithm::Sha256, &[index as u8; 32])])
                    .unwrap(),
                error: None,
            })
            .collect();
        let retention = ChecksumRetention::new(());
        let chunk = Arc::new(ChecksumChunk {
            base: root.as_path().into(),
            rows: rows.into_boxed_slice(),
            retention: retention.clone(),
        });
        state
            .borrow_mut()
            .apply(Event::Checksum(Box::new(ChecksumEvent {
                job,
                generation,
                update: ChecksumUpdate::Finished(Arc::new(ChecksumRun {
                    job,
                    generation,
                    algorithms: ChecksumAlgorithms::default(),
                    captured_roots: vec![root.clone()].into(),
                    rows: rows_of([chunk]),
                    summary: ChecksumSummary {
                        completion: ChecksumCompletion::Completed,
                        progress: ChecksumProgress {
                            calculated: count,
                            ..Default::default()
                        },
                        diagnostics: vec![],
                    },
                    retention,
                })),
            })));
        midnight_ui::refresh(&main, &state.borrow());
        draw(&native, 1000, 620, "checksum-selection-finished");
        Self {
            main,
            state,
            commands,
            native,
            root,
        }
    }
    /// Lay the window out as a frame does, change handlers included.
    fn frame(&self) {
        modeless::layout(&self.native, 1000, 620);
    }
    fn key(&self, modifiers: &[Key], key: impl Into<slint::SharedString>) {
        input::chord(&self.native, modifiers, key);
        self.frame();
    }
    /// Do what the clipboard worker does with the copies asked for since the last call
    /// (build each, then report it back to the window), and return the newest text
    /// written.
    fn copied(&self) -> Option<String> {
        let sent = std::mem::take(&mut *self.commands.borrow_mut());
        let mut newest = None;
        for command in sent {
            let Command::CopyGeneratedText {
                request,
                content,
                cancel,
            } = command
            else {
                continue;
            };
            let result = match content.prepare(&cancel) {
                Ok(()) => {
                    newest = content.take_text();
                    Ok(DesktopActionResult::ClipboardCopied)
                }
                Err(error) => Err(FsError::from_io(
                    "clipboard",
                    std::path::Path::new(""),
                    error,
                )),
            };
            self.state
                .borrow_mut()
                .apply(Event::DesktopActionFinished { request, result });
        }
        midnight_ui::refresh(&self.main, &self.state.borrow());
        self.frame();
        newest
    }
    fn close(self) {
        self.main.invoke_close_checksum_windows();
        self.main.hide().unwrap();
        self.state.borrow_mut().retire_all();
    }
}

fn hash(number: u8) -> String {
    format!("{number:02x}").repeat(32)
}

/// Click, Ctrl+click and Shift+click select rows; Copy, Ctrl+C and Ctrl+Shift+C put
/// exactly the selected rows on the clipboard, in the order shown; Ctrl+A selects every
/// row and Ctrl+D none, which disables Copy again.
#[test]
fn rows_select_by_pointer_and_copies_take_exactly_the_selection() {
    let t = Finished::new(6);
    let copy = copy_button(&t.native);
    assert_eq!(property(&copy, AccessibleProperty::Enabled), "false");
    let why = property(&copy, AccessibleProperty::Description);
    assert!(why.contains("Ctrl+A"), "a disabled Copy says why: {why}");
    click_row(&t.native, "r2.bin", &[]);
    t.frame();
    assert_eq!(selected_files(&t.native), ["r2.bin"]);
    click_row(&t.native, "r4.bin", &[Key::Control]);
    t.frame();
    assert_eq!(selected_files(&t.native), ["r2.bin", "r4.bin"]);
    click_row(&t.native, "r6.bin", &[Key::Shift]);
    t.frame();
    assert_eq!(selected_files(&t.native), ["r4.bin", "r5.bin", "r6.bin"]);
    // Copy shows how many lines it copies, and copies exactly them.
    draw(&t.native, 1000, 620, "checksum-selection-three");
    let copy = copy_button(&t.native);
    assert_eq!(property(&copy, AccessibleProperty::Label), "Copy 3 SHA-256");
    assert_eq!(property(&copy, AccessibleProperty::Enabled), "true");
    modeless::click_item(&t.native, &copy, "Copy");
    assert_eq!(
        t.copied().unwrap(),
        format!(
            "{} *r4.bin\n{} *r5.bin\n{} *r6.bin\n",
            hash(4),
            hash(5),
            hash(6)
        )
    );
    // Ctrl+C: their checksums; Ctrl+Shift+C: their full rows.
    click_row(&t.native, "r5.bin", &[Key::Control]);
    t.frame();
    assert_eq!(selected_files(&t.native), ["r4.bin", "r6.bin"]);
    t.key(&[Key::Control], "c");
    assert_eq!(t.copied().unwrap(), format!("{}\n{}", hash(4), hash(6)));
    t.key(&[Key::Control, Key::Shift], "C");
    let full = |number: u8| {
        format!(
            "{}\nSHA-256: {}",
            t.root.join(format!("r{number}.bin")).display(),
            hash(number)
        )
    };
    assert_eq!(t.copied().unwrap(), format!("{}\n\n{}", full(4), full(6)));
    // Ctrl+A selects all; Ctrl+D none, and Copy is disabled again.
    t.key(&[Key::Control], "a");
    assert_eq!(selected_files(&t.native).len(), 6);
    t.key(&[Key::Control], "d");
    assert!(selected_files(&t.native).is_empty());
    let copy = copy_button(&t.native);
    assert_eq!(property(&copy, AccessibleProperty::Enabled), "false");
    t.close();
}

/// The list works from the keyboard: the cursor keys select the row they reach, with
/// Shift extend the range and with Ctrl move the cursor alone; Space toggles the
/// cursor's row and Insert also moves down.
#[test]
fn the_keyboard_selects_extends_moves_and_toggles() {
    let t = Finished::new(6);
    click_row(&t.native, "r3.bin", &[]);
    t.frame();
    t.key(&[], Key::Home);
    assert_eq!(selected_files(&t.native), ["r1.bin"]);
    t.key(&[], Key::DownArrow);
    assert_eq!(selected_files(&t.native), ["r2.bin"]);
    t.key(&[Key::Shift], Key::DownArrow);
    t.key(&[Key::Shift], Key::DownArrow);
    assert_eq!(selected_files(&t.native), ["r2.bin", "r3.bin", "r4.bin"]);
    // Ctrl+Down moves the cursor alone; Space adds its row.
    t.key(&[Key::Control], Key::DownArrow);
    assert_eq!(selected_files(&t.native), ["r2.bin", "r3.bin", "r4.bin"]);
    t.key(&[], " ");
    assert_eq!(
        selected_files(&t.native),
        ["r2.bin", "r3.bin", "r4.bin", "r5.bin"]
    );
    // Space again removes it; Insert toggles it back and moves down.
    t.key(&[], " ");
    t.key(&[], Key::Insert);
    assert_eq!(
        selected_files(&t.native),
        ["r2.bin", "r3.bin", "r4.bin", "r5.bin"]
    );
    t.key(&[], Key::Insert);
    assert_eq!(
        selected_files(&t.native),
        ["r2.bin", "r3.bin", "r4.bin", "r5.bin", "r6.bin"]
    );
    t.key(&[], Key::End);
    assert_eq!(selected_files(&t.native), ["r6.bin"]);
    t.key(&[], Key::UpArrow);
    assert_eq!(selected_files(&t.native), ["r5.bin"]);
    t.key(&[Key::Shift], Key::PageUp);
    assert_eq!(selected_files(&t.native).len(), 5, "r1 to r5");
    t.key(&[], Key::PageDown);
    assert_eq!(selected_files(&t.native), ["r6.bin"]);
    t.close();
}

/// Select all and Deselect all on a long list scrolled to its end tell the list once
/// (`Rows::marks_changed` resets it): the same rows stay in view at the same place, all
/// marked, then none.
#[test]
fn select_all_on_a_scrolled_list_keeps_its_rows_in_place() {
    let t = Finished::new(3_000);
    click_row(&t.native, "r2.bin", &[]);
    t.frame();
    t.key(&[], Key::End);
    draw(&t.native, 1000, 620, "checksum-select-all-scrolled");
    let names = |rows: Vec<(String, f32, f32)>| {
        rows.into_iter()
            .map(|(label, top, _)| {
                let name = label.split(" · ").next().unwrap_or_default().to_owned();
                (name, top)
            })
            .collect::<Vec<_>>()
    };
    let shown = names(shown_rows(&t.native));
    assert!(
        shown.iter().any(|(name, _)| name == "r3000.bin"),
        "the end is in view: {shown:?}"
    );
    assert_eq!(selected_files(&t.native), ["r3000.bin"]);
    for (chord, every) in [("a", true), ("d", false)] {
        t.key(&[Key::Control], chord);
        draw(&t.native, 1000, 620, "checksum-select-all-scrolled");
        assert_eq!(
            names(shown_rows(&t.native)),
            shown,
            "Ctrl+{chord} keeps the view"
        );
        let marked = selected_files(&t.native).len();
        assert_eq!(marked, if every { shown.len() } else { 0 }, "Ctrl+{chord}");
    }
    t.close();
}

/// A bare window with 60 listed files f0… waiting.
fn waiting_rows(windows: &Rc<RefCell<Vec<Rc<Native>>>>) -> (ChecksumWindow, Rc<Native>) {
    use midnight_ui::ChecksumDisplayRow;
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 11, 15);
    window.set_generation_token("1".into());
    window.set_running(true);
    window.set_rows(
        Rc::new(slint::VecModel::from(
            (0..60)
                .map(|index| ChecksumDisplayRow {
                    token: format!("1:listed:{index}").into(),
                    name: format!("f{index}").into(),
                    path: "folder".into(),
                    status: "Waiting".into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    modeless::layout(&native, 860, 400);
    (window, native)
}

/// Whether the row of `name` is shown whole inside the result list.
fn in_view(native: &Native, name: &str) -> bool {
    let list = modeless::accessible_item(native, AccessibleRole::List, "Checksum results").unwrap();
    let top = list.map_to_native_window(list.geometry().origin).y;
    let bottom = top + list.geometry().height();
    shown_rows(native)
        .into_iter()
        .find(|(label, ..)| label.starts_with(&format!("{name} · ")))
        .is_some_and(|(_, row_top, row_bottom)| row_top >= top - 0.5 && row_bottom <= bottom + 0.5)
}

/// While nothing is selected the list follows the row being calculated, showing it at
/// its bottom edge, at most once per frame; a selection stops following, deselecting
/// resumes it, and a scroll by the user pauses it until the next run.
#[test]
fn the_list_follows_the_row_being_calculated_while_nothing_is_selected() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let (window, native) = waiting_rows(&windows);
    // A frame: the change handlers run (and may scroll), then the list is painted, which
    // instantiates the rows a scroll brought into view.
    let frame = || {
        modeless::layout(&native, 860, 400);
        modeless::render(&native, 860, 400, "checksum-follow");
    };
    window.set_active_row(0);
    frame();
    assert_eq!(window.get_follow_scrolls(), 0, "the first row is in view");
    // One file after another: whenever the next one would leave the view, the list
    // scrolls once, so that it shows at the bottom edge.
    for index in 1..=20 {
        let name = format!("f{index}");
        let before = (window.get_follow_scrolls(), in_view(&native, &name));
        window.set_active_row(index);
        frame();
        assert!(in_view(&native, &name), "{name} followed");
        assert_eq!(
            window.get_follow_scrolls(),
            before.0 + i32::from(!before.1),
            "{name}: a scroll only when it was out of view"
        );
    }
    assert!(
        !in_view(&native, "f21"),
        "the followed row is at the bottom edge"
    );
    assert!(window.get_follow_scrolls() > 0);
    // Five files finish between two frames: the list scrolls once, to the last.
    let scrolls = window.get_follow_scrolls();
    for index in 21..=25 {
        window.set_active_row(index);
    }
    assert!(!in_view(&native, "f25"), "nothing scrolls before the frame");
    frame();
    assert_eq!(window.get_follow_scrolls(), scrolls + 1);
    assert!(in_view(&native, "f25") && !in_view(&native, "f26"));
    // A selection stops following.
    window.set_selection_empty(false);
    for index in 26..=40 {
        window.set_active_row(index);
        frame();
    }
    assert_eq!(window.get_follow_scrolls(), scrolls + 1);
    assert!(!in_view(&native, "f40"));
    // Deselecting all resumes: the row being calculated comes into view again.
    window.set_selection_empty(true);
    frame();
    assert_eq!(window.get_follow_scrolls(), scrolls + 2);
    assert!(in_view(&native, "f40"));
    // A scroll by the user pauses following.
    let list =
        modeless::accessible_item(&native, AccessibleRole::List, "Checksum results").unwrap();
    let origin = list.map_to_native_window(list.geometry().origin);
    native
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(origin.x + 40.0, origin.y + 60.0),
            delta_x: 0.0,
            delta_y: 400.0,
        });
    frame();
    for index in 41..=50 {
        window.set_active_row(index);
        frame();
    }
    assert_eq!(
        window.get_follow_scrolls(),
        scrolls + 2,
        "paused by the scroll"
    );
    assert!(!in_view(&native, "f50"));
    // A new run follows again.
    window.set_generation_token("2".into());
    frame();
    window.set_active_row(55);
    frame();
    assert_eq!(window.get_follow_scrolls(), scrolls + 3);
    assert!(in_view(&native, "f55"));
    window.hide().unwrap();
}

fn luminance(color: slint::Color) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(first: slint::Color, second: slint::Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// In every theme a selected row (a square of the selection edge on the selection fill)
/// and the cursor's row (a full-height stroke of the cursor edge) look different without
/// relying on hue, and their marks and text keep their contrast.
#[test]
fn selected_rows_and_the_cursor_row_are_distinct_in_every_theme() {
    use midnight_ui::{ChecksumDisplayRow, Theme};
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
    let row = |name: &str, selected: bool| ChecksumDisplayRow {
        token: format!("1:{name}").into(),
        name: name.into(),
        path: "folder".into(),
        status: "Calculated".into(),
        calculated: true,
        selected,
        ..Default::default()
    };
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            row("selected", true),
            row("cursor", false),
            row("plain", false),
        ]))
        .into(),
    );
    window.set_cursor_row(1);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let mut findings = Vec::new();
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        window.invoke_apply_preferences(scheme, 0, "".into(), 12, 15);
        let pixels = modeless::render(&native, 700, 400, &format!("checksum-marks-{scheme}"));
        let pixel = |x: f32, y: f32| {
            let value = pixels[y as usize * 700 + x as usize];
            slint::Color::from_rgb_u8(value.r, value.g, value.b)
        };
        let theme = window.global::<Theme>();
        let place = |name: &str| {
            let item = result_row(&native, name);
            let origin = item.map_to_native_window(item.geometry().origin);
            (origin.x, origin.y + item.geometry().height() / 2.0)
        };
        let (x, selected_middle) = place("selected");
        let (_, cursor_middle) = place("cursor");
        let marks = [
            (
                "selection square",
                pixel(x + 5.0, selected_middle),
                theme.get_selected_edge(),
            ),
            (
                "no stroke on a selected row",
                pixel(x + 1.0, selected_middle - 6.0),
                theme.get_selected_background(),
            ),
            (
                "cursor stroke",
                pixel(x + 1.0, cursor_middle - 6.0),
                theme.get_cursor_edge(),
            ),
            (
                "no square on the cursor's row",
                pixel(x + 5.0, cursor_middle),
                theme.get_cursor_background(),
            ),
        ];
        for (what, shown, wanted) in marks {
            if shown != wanted {
                findings.push(format!(
                    "theme {scheme}: {what} is {shown:?}, not {wanted:?}"
                ));
            }
        }
        let text_minimum = match scheme {
            17..=20 => 7.0,
            21..=22 => 12.0,
            _ => 4.5,
        };
        for (what, first, second, minimum) in [
            (
                "square on the selection",
                theme.get_selected_edge(),
                theme.get_selected_background(),
                3.0,
            ),
            (
                "stroke on the cursor's row",
                theme.get_cursor_edge(),
                theme.get_cursor_background(),
                3.0,
            ),
            (
                "cursor marks on a selected row",
                theme.get_cursor_edge(),
                theme.get_selected_background(),
                3.0,
            ),
            (
                "selected text",
                theme.get_selected_text(),
                theme.get_selected_background(),
                text_minimum,
            ),
        ] {
            let ratio = contrast(first, second);
            if ratio < minimum {
                findings.push(format!("theme {scheme}: {what} {ratio:.2}:1 < {minimum}"));
            }
        }
    }
    assert!(findings.is_empty(), "{findings:#?}");
    window.hide().unwrap();
}

/// The cursor's row is brought into view whole wherever it goes, also after a jump of
/// more than a screen (End, Home, a far click), when the ListView re-anchors its rows.
#[test]
fn the_cursor_row_comes_into_view_whole_after_far_jumps() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let (window, native) = waiting_rows(&windows);
    let frame = || {
        modeless::layout(&native, 860, 400);
        modeless::render(&native, 860, 400, "checksum-cursor-reveal");
    };
    for row in [59, 0, 30, 31, 45, 2] {
        window.set_cursor_row(row);
        frame();
        let name = format!("f{row}");
        assert!(
            in_view(&native, &name),
            "the cursor's row {name} is shown whole"
        );
    }
    window.hide().unwrap();
}
