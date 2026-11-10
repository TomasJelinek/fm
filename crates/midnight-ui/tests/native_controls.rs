//! Native navigation identities survive asynchronous metadata; full cwd stays readable.
use crate::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{ActionIcons, FileRow, MainWindow, Presenter, Theme, install, refresh};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use crate::platform_keys;

use crate::raw_paths;

struct TestPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}
fn key(window: &MainWindow, key: Key) {
    platform_keys::key(window, key.into());
}
fn pointer(window: &MainWindow, x: f32, y: f32, down: bool) {
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(if down {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}
fn open_picker(window: &MainWindow, pane: PaneId) {
    window.invoke_focus_files();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Alt.into(),
    });
    key(
        window,
        if pane == PaneId::Left {
            Key::F1
        } else {
            Key::F2
        },
    );
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Alt.into(),
    });
}
fn location(id: &str, kind: LocationKind, path: PathBuf) -> NavigationLocation {
    NavigationLocation {
        id: id.into(),
        label: id.into(),
        kind,
        path,
        drive_letter: None,
    }
}
fn quick_position(window: &MainWindow) -> slint::LogicalPosition {
    let hint = window.get_left_locations().row_data(0).unwrap().tooltip;
    ui::root_position(window, &hint, AccessibleRole::Button)
}
fn popup_position(window: &MainWindow, label: &str) -> slint::LogicalPosition {
    fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value.as_str() == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, label) {
                return Some(found);
            }
        }
        None
    }
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("an open navigation picker");
    let item = find(ItemRc::new_root(popup.component.clone()), label)
        .unwrap_or_else(|| panic!("no navigation row: {label}"));
    drop(popups);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}
fn latest_path(commands: &RefCell<Vec<Command>>) -> PathBuf {
    commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate { path, .. } | Command::Refresh { path, .. } => Some(path.clone()),
            _ => None,
        })
        .unwrap()
}
fn navigations(commands: &RefCell<Vec<Command>>) -> usize {
    commands
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::Navigate { .. } | Command::Refresh { .. }))
        .count()
}
fn details() -> VolumeDetails {
    VolumeDetails {
        label: Ok(Some(OsString::from("Work 文档 & <plain>"))),
        space: Ok(SpaceInfo {
            available: 1 << 30,
            total: 2 << 30,
        }),
    }
}

#[test]
fn native_choices_keep_gestures_tooltips_and_full_unicode_working_paths() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::temp_dir().join("midnight-ui-native-controls 文档");
    let home = root.join("Home e\u{301} 👩🏽‍💻");
    let downloads = home.join("Downloads & [plain]");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    presenter.borrow_mut().apply(Event::VolumesUpdated(vec![
        Volume {
            id: "disk".into(),
            label: "Disk".into(),
            root: root.clone(),
        },
        Volume {
            id: "home".into(),
            label: "Home".into(),
            root: home.clone(),
        },
    ]));
    presenter.borrow_mut().apply(Event::DesktopContext {
        shell: ShellFlavor::Posix,
        locations: vec![
            location("disk", LocationKind::Volume, root.clone()),
            location("home", LocationKind::Home, home.clone()),
            location("desktop", LocationKind::Desktop, home.clone()),
            location("documents", LocationKind::Documents, home.join("Documents")),
            location("downloads", LocationKind::Downloads, downloads.clone()),
        ],
    });
    refresh(&window, &presenter.borrow());
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    let draw = |pixels: &mut [slint::Rgb8Pixel], width| {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, width);
        });
    };
    draw(&mut pixels, 1340);
    let choices = window.get_navigation_choices();
    assert_eq!(
        choices.row_count(),
        6,
        "five locations plus the other-panel action"
    );
    let other_panel = choices.row_data(5).unwrap();
    assert_eq!(other_panel.identity, "other-panel");
    assert_eq!(other_panel.label, ". As Other Panel");
    for locations in [window.get_left_locations(), window.get_right_locations()] {
        assert_eq!(
            locations.row_count(),
            5,
            "synthetic Home is merged only once"
        );
        for index in 0..locations.row_count() {
            let choice = choices.row_data(index).unwrap();
            let quick = locations.row_data(index).unwrap();
            assert_eq!(quick.identity, choice.identity);
            assert_eq!(quick.tooltip, choice.tooltip);
        }
    }
    assert_ne!(
        choices.row_data(1).unwrap().identity,
        choices.row_data(2).unwrap().identity,
        "Home and Desktop remain separate even when their raw target is identical"
    );

    // Native metadata and icon replies must not destroy a held quick button.
    let quick = quick_position(&window);
    let original_locations = window.get_left_locations();
    pointer(&window, quick.x, quick.y, true);
    presenter.borrow_mut().apply(Event::VolumeDetailsUpdated {
        volume_id: "disk".into(),
        root: root.clone(),
        details: details(),
    });
    presenter.borrow_mut().apply(Event::IconLoaded {
        key: IconKey::for_drive(&root),
        pixels: Arc::new(IconPixels::new(1, 1, vec![232, 11, 29, 255]).unwrap()),
    });
    refresh(&window, &presenter.borrow());
    draw(&mut pixels, 1340);
    let before = navigations(&commands);
    pointer(&window, quick.x, quick.y, false);
    assert!(
        std::ptr::eq(
            original_locations.as_any(),
            window.get_left_locations().as_any()
        ),
        "metadata must preserve the quick model, not merely its count"
    );
    assert_eq!(
        navigations(&commands),
        before + 1,
        "metadata cannot lose a held click"
    );
    assert_eq!(latest_path(&commands), root);

    // Timed hover shows the real tooltip, not just model strings or accessibility.
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    draw(&mut pixels, 1340);
    let quick = quick_position(&window);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: quick });
    draw(&mut pixels, 1340);
    let quiet = pixels.clone();
    time.set(Duration::from_millis(1000));
    draw(&mut pixels, 1340);
    assert!(
        quiet.iter().zip(&pixels).filter(|(a, b)| a != b).count() > 100,
        "quick drives must display an actual hover tooltip"
    );
    let hint = window.get_left_locations().row_data(0).unwrap().tooltip;
    assert!(hint.contains("Work 文档 & <plain>") && hint.contains("1.0 GB free"));
    let with_tooltip = pixels.clone();
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    draw(&mut pixels, 1340);
    assert!(
        with_tooltip
            .iter()
            .zip(&pixels)
            .filter(|(a, b)| a != b)
            .count()
            > 100,
        "leaving a quick target dismisses its tooltip"
    );

    // Every special alias can be picked from the same drive dropdown.
    open_picker(&window, PaneId::Left);
    draw(&mut pixels, 1340);
    key(&window, Key::End);
    key(&window, Key::UpArrow); // Downloads precedes the final other-panel action.
    // Metadata refresh while open retains the pending semantic choice.
    presenter.borrow_mut().apply(Event::VolumeDetailsUpdated {
        volume_id: "disk".into(),
        root: root.clone(),
        details: VolumeDetails {
            label: Ok(Some("New native volume name 文档".into())),
            space: Ok(SpaceInfo {
                available: 2 << 30,
                total: 3 << 30,
            }),
        },
    });
    refresh(&window, &presenter.borrow());
    draw(&mut pixels, 1340);
    key(&window, Key::Return);
    assert_eq!(latest_path(&commands), downloads);
    draw(&mut pixels, 1340);
    assert_eq!(window.get_left_navigation_index(), 0);
    let caption = ui::text_values(ui::root_item(
        &window,
        "Drive or volume",
        AccessibleRole::Combobox,
    ));
    assert!(
        caption.iter().any(|text| text == "Disk") && caption.iter().all(|text| text != "Home"),
        "a folder below Home must show its containing drive in the closed picker: {caption:?}"
    );

    // The second panel uses the same semantic choices and row tooltips. A real
    // popup-row press survives a changed label/free-space reply, too.
    open_picker(&window, PaneId::Right);
    draw(&mut pixels, 1340);
    let theme = window.global::<Theme>();
    let home_label = window.get_navigation_choices().row_data(1).unwrap().label;
    let home_row = popup_position(&window, &home_label);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: home_row });
    draw(&mut pixels, 1340);
    let quiet_popup = pixels.clone();
    time.set(Duration::from_millis(2000));
    draw(&mut pixels, 1340);
    assert!(
        quiet_popup
            .iter()
            .zip(&pixels)
            .filter(|(a, b)| a != b)
            .count()
            > 100,
        "special-location dropdown rows display the same actual hover tooltips"
    );
    pointer(&window, home_row.x, home_row.y, true);
    presenter.borrow_mut().apply(Event::VolumeDetailsUpdated {
        volume_id: "disk".into(),
        root: root.clone(),
        details: details(),
    });
    refresh(&window, &presenter.borrow());
    draw(&mut pixels, 1340);
    let before = navigations(&commands);
    pointer(&window, home_row.x, home_row.y, false);
    assert_eq!(navigations(&commands), before + 1);
    assert_eq!(latest_path(&commands), home);

    // Equal captions with a new raw root invalidate a quick press.
    let quick = quick_position(&window);
    pointer(&window, quick.x, quick.y, true);
    let before = navigations(&commands);
    presenter
        .borrow_mut()
        .apply(Event::VolumesUpdated(vec![Volume {
            id: "disk".into(),
            label: "Disk".into(),
            root: root.join("replacement"),
        }]));
    refresh(&window, &presenter.borrow());
    draw(&mut pixels, 1340);
    pointer(&window, quick.x, quick.y, false);
    assert_eq!(
        navigations(&commands),
        before,
        "replacement cannot retarget a held click"
    );

    // Reordering two canonical targets while a popup row is held cannot turn
    // that gesture into navigation to a different mount, even with equal labels.
    let [first_raw, second_raw] = raw_paths::same_display(&root);
    assert_ne!(first_raw, second_raw);
    assert_eq!(first_raw.to_string_lossy(), second_raw.to_string_lossy());
    let drives = vec![
        Volume {
            id: "a".into(),
            label: "Same".into(),
            root: first_raw,
        },
        Volume {
            id: "b".into(),
            label: "Same".into(),
            root: second_raw,
        },
    ];
    presenter
        .borrow_mut()
        .apply(Event::VolumesUpdated(drives.clone()));
    refresh(&window, &presenter.borrow());
    assert_eq!(
        window.get_navigation_choices().row_data(0).unwrap().label,
        window.get_navigation_choices().row_data(1).unwrap().label,
        "the real canonical rows must have identical visible text"
    );
    open_picker(&window, PaneId::Right);
    draw(&mut pixels, 1340);
    let first_label = window.get_navigation_choices().row_data(0).unwrap().label;
    let first_row = popup_position(&window, &first_label);
    pointer(&window, first_row.x, first_row.y, true);
    let before = navigations(&commands);
    presenter
        .borrow_mut()
        .apply(Event::VolumesUpdated(drives.into_iter().rev().collect()));
    refresh(&window, &presenter.borrow());
    draw(&mut pixels, 1340);
    pointer(&window, first_row.x, first_row.y, false);
    key(&window, Key::Escape);
    assert_eq!(
        navigations(&commands),
        before,
        "reordering cancels the popup gesture"
    );

    // Native icon pixels retain their RGB; clearing the palette restores SVG fallback.
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(0.0, 0.0),
    });
    let buffer =
        slint::SharedPixelBuffer::<slint::Rgb8Pixel>::clone_from_slice(&[251, 7, 13], 1, 1);
    window.set_action_icons(ActionIcons {
        refresh: slint::Image::from_rgb8(buffer),
        ..Default::default()
    });
    draw(&mut pixels, 1340);
    let red = pixels
        .iter()
        .filter(|p| p.r == 251 && p.g == 7 && p.b == 13)
        .count();
    assert!(red > 100, "native action RGB must not be tinted");
    let native_coordinates: Vec<_> = pixels
        .iter()
        .enumerate()
        .filter_map(|(index, p)| (p.r == 251 && p.g == 7 && p.b == 13).then_some(index))
        .collect();
    let native_pixels = pixels.clone();
    window.set_action_icons(ActionIcons::default());
    draw(&mut pixels, 1340);
    assert!(
        pixels
            .iter()
            .zip(&native_pixels)
            .filter(|(a, b)| a != b)
            .count()
            > 100
    );

    let background = pixels[native_coordinates[0] - 1341];
    assert!(
        native_coordinates
            .iter()
            .filter(|index| **index % 1340 < 670 && pixels[**index] != background)
            .count()
            > 30,
        "missing native icons render visible semantic vector fallbacks"
    );

    // File and folder rows stay quiet on hover, including unfinished sizes.
    // Both panels retain the deliberate tooltips on their sort headers.
    let hover_rows = ModelRc::new(VecModel::from(vec![
        FileRow {
            identity: "hover-folder".into(),
            name: "Unfinished folder 文档".into(),
            size: "≈1.0 KB".into(),
            size_state: 1,
            size_hint: "Still counting; this size is not final.".into(),
            directory: true,
            ..Default::default()
        },
        FileRow {
            identity: "hover-file".into(),
            name: "Regular file 文档.txt".into(),
            size: "1.0 KB".into(),
            ..Default::default()
        },
    ]));
    window.set_left_rows(hover_rows.clone());
    window.set_right_rows(hover_rows);
    draw(&mut pixels, 1340);
    let mut hover_targets = Vec::new();
    for column in ["Name", "Size", "Modified"] {
        let header = ui::root_position(
            &window,
            &format!("Sort by {column}"),
            AccessibleRole::Button,
        );
        for (offset, file_top) in [
            (0.0, window.get_left_list_top()),
            (
                window.get_right_list_left() - window.get_left_list_left(),
                window.get_right_list_top(),
            ),
        ] {
            let x = header.x + offset;
            for row in [0.5, 1.5] {
                hover_targets.push((x, file_top + row * theme.get_row_height(), false));
            }
            hover_targets.push((x, header.y, true));
        }
    }
    let mut hover_time = 2000;
    for (hover_x, hover_y, is_header) in hover_targets {
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(hover_x, hover_y),
        });
        draw(&mut pixels, 1340);
        let quiet = pixels.clone();
        hover_time += 1000;
        time.set(Duration::from_millis(hover_time));
        draw(&mut pixels, 1340);
        let changed = quiet.iter().zip(&pixels).filter(|(a, b)| a != b).count();
        if is_header {
            assert!(
                changed > 100,
                "sort headers must retain actual timed hover tooltips"
            );
        } else {
            assert_eq!(
                changed, 0,
                "file and folder rows must not display hover tooltips"
            );
        }
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(0.0, 0.0),
        });
        draw(&mut pixels, 1340);
    }

    assert_eq!(
        window.get_command_bar_height(),
        theme.get_command_height(),
        "the cwd and editor must share exactly one command row"
    );

    // The full cwd shares one line with the editor. Short paths use their
    // natural width; extreme Unicode components pan within the remaining space.
    let long = format!("{}/文档{}👩🏽‍💻tail", root.display(), "e\u{301}".repeat(2400));
    let short = root
        .ancestors()
        .last()
        .unwrap()
        .join("文档")
        .to_string_lossy()
        .into_owned();
    window.set_left_path(long.clone().into());
    window.set_right_path(short.clone().into());
    window.set_active_pane(0);
    window.set_desktop_status("An important terminal or desktop failure with a long Unicode explanation 文档 👩🏽‍💻 that must remain available in full through the tooltip.".into());
    window.set_desktop_feedback_important(true);
    for (width, requested_height) in [(1340u32, 820u32), (1040, 480)] {
        for density in [0, 1] {
            for font in [if density == 0 { 11 } else { 15 }, 32] {
                window.set_desktop_font_size(font);
                window.set_touch_font_size(font);
                window.invoke_apply_preferences(0, density);
                let height =
                    requested_height.max(window.get_effective_minimum_height().ceil() as u32);
                native.set_size(slint::PhysicalSize::new(width, height));
                window.window().dispatch_event(WindowEvent::Resized {
                    size: slint::LogicalSize::new(width as f32, height as f32),
                });
                for error in ["", "A directory access error"] {
                    window.set_left_error(error.into());
                    draw(&mut pixels, width as usize);
                    assert_eq!(window.get_command_directory().as_str(), long);
                    assert_eq!(
                        window.get_command_bar_height(),
                        window.global::<Theme>().get_command_height()
                    );
                    assert!(
                        (window.get_command_path_top() - window.get_command_input_top()).abs()
                            <= 1.0,
                        "cwd and editor must occupy the same line"
                    );
                    assert_eq!(
                        window.get_command_path_height(),
                        window.get_command_input_height()
                    );
                    assert!(window.get_command_path_width() > 0.0);
                    assert!(
                        window.get_command_path_content_width() > window.get_command_path_width()
                    );
                    assert!(window.get_command_path_left() < window.get_command_input_left());
                    assert!(
                        window.get_command_path_left() + window.get_command_path_width()
                            <= window.get_command_input_left(),
                        "the cwd viewport must not overlap the command editor"
                    );
                    assert!(
                        window.get_command_input_width() > 300.0,
                        "full cwd must leave useful editor width at {width}x{height}, density{density}/font{font}, error={error:?}: editor={}px, cwd={}px, natural_cwd={}px, options={}px, run={}px, history={}px",
                        window.get_command_input_width(),
                        window.get_command_path_width(),
                        window.get_command_path_natural_width(),
                        ui::root_item(&window, "Options", AccessibleRole::Button)
                            .geometry()
                            .size
                            .width,
                        ui::root_item(&window, "Run", AccessibleRole::Button)
                            .geometry()
                            .size
                            .width,
                        window.get_command_history_width()
                    );
                    assert!(
                        window.get_left_list_visible_height()
                            >= window.global::<Theme>().get_row_height(),
                        "one complete file row must remain at {width}x{height}, density{density}/font{font}, error={error}: {}",
                        window.get_left_list_visible_height()
                    );
                    assert!(
                        window.get_command_input_top() + window.get_command_input_height()
                            < height as f32 - window.global::<Theme>().get_bottom_height()
                    );
                    let long_editor_width = window.get_command_input_width();
                    window.set_active_pane(1);
                    draw(&mut pixels, width as usize);
                    assert_eq!(window.get_command_directory().as_str(), short);
                    assert!(
                        (window.get_command_path_width() - window.get_command_path_natural_width())
                            .abs()
                            <= 1.0,
                        "ordinary paths must be printed in full at their natural width"
                    );
                    assert!(
                        window.get_command_input_width() > long_editor_width,
                        "short paths return their unused width to the editor"
                    );
                    window.set_active_pane(0);
                    draw(&mut pixels, width as usize);
                }
            }
        }
    }
    let path_left = window.get_command_path_text_left();
    let scroll_x = window.get_command_path_left() + window.get_command_path_width() / 2.0;
    let scroll_y = window.get_command_path_top() + window.get_command_path_height() / 2.0;
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(scroll_x, scroll_y),
            delta_x: 0.0,
            delta_y: -120.0,
        });
    draw(&mut pixels, 1040);
    assert!(
        window.get_command_path_text_left() < path_left,
        "an ordinary mouse wheel pans an extreme cwd horizontally"
    );
    let before_drag = window.get_command_path_text_left();
    pointer(&window, scroll_x, scroll_y, true);
    hover_time += 20;
    time.set(Duration::from_millis(hover_time));
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(scroll_x - 80.0, scroll_y),
    });
    draw(&mut pixels, 1040);
    hover_time += 20;
    time.set(Duration::from_millis(hover_time));
    pointer(&window, scroll_x - 120.0, scroll_y, false);
    draw(&mut pixels, 1040);
    assert!(
        window.get_command_path_text_left() < before_drag,
        "mouse dragging must pan the full path even with a wheel handler present"
    );
    window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(scroll_x, scroll_y),
            delta_x: -1_000_000.0,
            delta_y: 0.0,
        });
    draw(&mut pixels, 1040);
    let text_right =
        window.get_command_path_text_left() + window.get_command_path_natural_width() - 8.0;
    let viewport_right = window.get_command_path_left() + window.get_command_path_width();
    assert!(
        (text_right - (viewport_right - 4.0)).abs() <= 1.0,
        "the complete Unicode tail remains reachable without an ellipsis or data cap"
    );
    window.set_active_pane(1);
    draw(&mut pixels, 1040);
    assert_eq!(window.get_command_directory().as_str(), short);
    assert!(
        (window.get_command_path_text_left() - window.get_command_path_left() - 4.0).abs() <= 1.0,
        "switching cwd resets horizontal pan to the beginning"
    );
    window.hide().unwrap();
}
