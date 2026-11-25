use super::*;
use crate::FindWindow;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, WindowItem},
    window::PopupWindowLocation,
};
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::Cell, rc::Rc, time::Duration};

type Natives = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct PlatformState {
    natives: Natives,
    time: Rc<Cell<Duration>>,
}
impl Platform for PlatformState {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        self.natives.borrow_mut().push(native.clone());
        Ok(native)
    }
    fn duration_since_start(&self) -> Duration {
        self.time.get()
    }
}
fn setup() -> (Natives, Rc<Cell<Duration>>) {
    let natives = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(PlatformState {
        natives: natives.clone(),
        time: time.clone(),
    }))
    .unwrap();
    (natives, time)
}
fn resize(window: &slint::Window, native: &MinimalSoftwareWindow, width: f32, height: f32) {
    native.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    window.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width, height),
    });
}
fn input(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::TextInput
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value.as_str() == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(item) = input(item, label) {
            return Some(item);
        }
    }
    None
}

fn hovered_tooltip_areas(item: ItemRc) -> usize {
    let mut count = usize::from(
        item.downcast::<TooltipArea>()
            .is_some_and(|area| area.as_pin_ref().has_hover()),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += hovered_tooltip_areas(item);
    }
    count
}

#[test]
fn find_checkbox_without_a_hint_does_not_open_an_empty_tooltip_after_hover_delay() {
    fn checkbox(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Checkbox
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value.as_str() == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = checkbox(item, label) {
                return Some(found);
            }
        }
        None
    }
    let (natives, time) = setup();
    let window = FindWindow::new().unwrap();
    install_find(&window).unwrap();
    window.invoke_apply_preferences(0, 0, "Segoe UI".into(), 13, 13);
    window.show().unwrap();
    let native = natives.borrow()[0].clone();
    resize(window.window(), &native, 860., 900.);
    let mut pixels = vec![slint::Rgb8Pixel::default(); 860 * 900];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 860);
        });
    };
    draw(&mut pixels);
    let root = || ItemRc::new_root(WindowInner::from_pub(window.window()).component());
    let target = checkbox(root(), "Match name case").unwrap();
    let rect = target.geometry();
    let origin = target.map_to_native_window(rect.origin);
    let mut ancestor = target.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = ancestor {
        if item.downcast::<Flickable>().is_some() {
            let area = item.geometry();
            let clip = item.map_to_native_window(area.origin);
            assert!(
                origin.y >= clip.y && origin.y + rect.height() <= clip.y + area.height(),
                "the real checkbox must be visible through the query viewport before hover"
            );
        }
        ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    let position =
        slint::LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    draw(&mut pixels);
    time.set(Duration::from_millis(200));
    draw(&mut pixels);
    time.set(Duration::from_millis(600));
    draw(&mut pixels);
    assert!(
        !WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
        "a checkbox with no hint must not open a blank tooltip after timed hover"
    );
    window.hide().unwrap();
}

#[test]
fn find_host_tracks_its_own_cursor_and_client_bounds_without_updating_main() {
    let (natives, _) = setup();
    let main = MainWindow::new().unwrap();
    install(&main).unwrap();
    main.show().unwrap();
    resize(main.window(), &natives.borrow()[0], 1340., 900.);
    main.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(20., 30.),
    });
    let find = FindWindow::new().unwrap();
    install_find(&find).unwrap();
    find.show().unwrap();
    resize(find.window(), &natives.borrow()[1], 360., 620.);
    find.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(350., 610.),
    });
    let placement = find.global::<TooltipPlacement>();
    assert_eq!(placement.get_surface_width(), 360.);
    assert_eq!(placement.get_surface_height(), 620.);
    assert_eq!(placement.get_cursor_x(), 350.);
    assert_eq!(placement.get_cursor_y(), 610.);
    assert_eq!(main.global::<TooltipPlacement>().get_cursor_x(), 20.);
    assert_eq!(main.global::<TooltipPlacement>().get_surface_width(), 1340.);
    resize(find.window(), &natives.borrow()[1], 320., 200.);
    assert_eq!(placement.get_surface_width(), 320.);
    assert_eq!(placement.get_surface_height(), 200.);
    find.window().dispatch_event(WindowEvent::PointerExited);
    assert_eq!(placement.get_cursor_x(), -1.);
    assert_eq!(placement.get_cursor_y(), -1.);
    find.hide().unwrap();
    main.hide().unwrap();
}

#[test]
fn tooltip_frames_follow_the_window_scale_and_round_up_to_device_pixels() {
    let (natives, _) = setup();
    let main = MainWindow::new().unwrap();
    install(&main).unwrap();
    main.show().unwrap();
    resize(main.window(), &natives.borrow()[0], 1340., 900.);
    let placement = main.global::<TooltipPlacement>();
    main.window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.25 });
    main.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(40., 50.),
    });
    assert_eq!(placement.get_scale(), 1.25);
    for value in [100.9_f32, 100.8, 57.3, 120.0, 1.0] {
        let snapped = placement.invoke_snap_up(value);
        let device = snapped * 1.25;
        assert!(
            (device - device.round()).abs() < 1e-3,
            "{value} -> {snapped} is not on the device-pixel grid"
        );
        assert!(snapped + 1e-3 >= value, "{value} shrank to {snapped}");
        assert!(snapped - value < 0.8 + 1e-3, "{value} grew to {snapped}");
    }
    // Nearest rounding took 0.1px from a 100.9px hint: enough to wrap its last word.
    assert!(placement.invoke_snap(100.9) < 100.9);
    main.hide().unwrap();
}

#[test]
fn actual_find_hint_is_offset_and_clamped_to_the_find_host() {
    let (natives, time) = setup();
    let find = FindWindow::new().unwrap();
    install_find(&find).unwrap();
    find.invoke_apply_preferences(0, 0, "Segoe UI".into(), 13, 13);
    find.show().unwrap();
    let native = natives.borrow()[0].clone();
    resize(find.window(), &native, 360., 620.);
    let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 620];
    let mut draw = || {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 360);
        });
    };
    draw();
    let item = input(
        ItemRc::new_root(WindowInner::from_pub(find.window()).component()),
        "File name masks",
    )
    .unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let cursor = slint::LogicalPosition::new(
        // Fluent ScrollView overlays a 14px scrollbar on its viewport. Hover
        // the visible editor interior rather than the track over its far edge.
        origin.x + rect.size.width - 24.,
        origin.y + rect.size.height / 2.,
    );
    assert!(cursor.x > 180. && cursor.x < 360. && cursor.y < 270.);
    find.window()
        .dispatch_event(WindowEvent::PointerMoved { position: cursor });
    draw();
    assert!(
        hovered_tooltip_areas(ItemRc::new_root(
            WindowInner::from_pub(find.window()).component(),
        )) > 0,
        "The mask editor must receive hover before timing its hint: {origin:?}, {rect:?}, {cursor:?}"
    );
    time.set(Duration::from_millis(600));
    draw();
    let inner = WindowInner::from_pub(find.window());
    let (origin, component) = {
        let popups = inner.active_popups();
        let tooltip = popups
            .iter()
            .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("Real Find hint must show after timed hover");
        let PopupWindowLocation::ChildWindow(origin) = tooltip.location else {
            panic!("Expected embedded tooltip");
        };
        (origin, tooltip.component.clone())
    };
    let root = ItemRc::new_root(component);
    let popup = root.downcast::<WindowItem>().unwrap();
    let width = popup.as_pin_ref().width().get();
    let height = popup.as_pin_ref().height().get();
    assert!(width > 0. && height > 0.);
    assert!(
        origin.x >= 7.9
            && origin.y >= 7.9
            && origin.x + width <= 352.1
            && origin.y + height <= 612.1,
        "Find hint exceeds its 360x620 host: {origin:?}, {width}x{height}"
    );
    assert!(
        origin.x + width <= cursor.x - 13.9,
        "Right-edge tooltip must flip left with a 14px cursor gap"
    );
    assert!(
        cursor.y < origin.y || cursor.y >= origin.y + height,
        "Tooltip must not cover its cursor"
    );
    find.hide().unwrap();
}

#[test]
fn hovering_a_folder_link_lazily_reads_its_native_target() {
    use midnight_core::{Command, DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId};
    use std::path::PathBuf;

    fn named_row(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::ListItem
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value.as_str() == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(row) = named_row(item, label) {
                return Some(row);
            }
        }
        None
    }

    let (natives, time) = setup();
    let window = MainWindow::new().unwrap();
    let native = natives.borrow()[0].clone();
    let root = std::env::temp_dir().join("midnight-link-tooltip-ui");
    let path = root.join("hovered link");
    let pane = PaneId::Left;
    let presenter = Rc::new(RefCell::new(crate::Presenter::new(root.clone(), root)));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let send = {
        let commands = commands.clone();
        Rc::new(move |command| {
            commands.borrow_mut().push(command);
            true
        })
    };
    crate::install(&window, presenter.clone(), send);
    let navigation = commands
        .borrow()
        .iter()
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
        .unwrap();
    commands.borrow_mut().clear();
    let entries = vec![
        Entry::new(EntryId(17), path.clone(), EntryKind::DirectoryLink),
        Entry::new(
            EntryId(19),
            path.with_file_name("ordinary"),
            EntryKind::Directory,
        ),
    ];
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane,
        navigation,
        index: std::sync::Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.into(),
    });
    window.show().unwrap();
    resize(window.window(), &native, 1_000., 660.);
    crate::refresh(&window, &presenter.borrow());
    assert!(
        !window.get_left_loading(),
        "the install-triggered listing must finish before hover"
    );
    let pixels = RefCell::new(vec![slint::Rgb8Pixel::default(); 1_000 * 660]);
    let draw = || {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels.borrow_mut(), 1_000);
        });
    };
    draw();
    let move_to = |label| {
        let row = named_row(
            ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
            label,
        )
        .unwrap();
        let bounds = row.geometry();
        let origin = row.map_to_native_window(bounds.origin);
        eprintln!("link hover target={label} origin={origin:?} bounds={bounds:?}");
        window.window().dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(
                origin.x + (bounds.size.width / 2.).min(110.),
                origin.y + bounds.size.height / 2.,
            ),
        });
    };
    let lookups = || {
        commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::ReadDirectoryLinkTarget {
                    request,
                    pane,
                    navigation,
                    path,
                } => Some((*request, *pane, *navigation, path.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(lookups().is_empty());
    move_to("hovered link · Folder link");
    time.set(Duration::from_millis(100));
    draw();
    assert!(
        hovered_tooltip_areas(ItemRc::new_root(
            WindowInner::from_pub(window.window()).component()
        )) > 0,
        "the completed native directory-link row must receive tooltip hover"
    );
    assert!(lookups().is_empty());
    time.set(Duration::from_millis(600));
    draw();
    time.set(Duration::from_millis(601));
    draw();
    assert!(
        WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
        "the native read is tied to an embedded tooltip"
    );
    let lookups_after_hover = lookups();
    let [(request, actual_pane, actual_navigation, actual_path)] = lookups_after_hover.as_slice()
    else {
        panic!("expected one delayed link lookup, got {lookups_after_hover:?}")
    };
    let request = *request;
    assert_eq!(
        (*actual_pane, *actual_navigation, actual_path),
        (pane, navigation, &path)
    );

    let target = concat!(
        r"D:\development\release-packages\samples\collision-detection\compiled-dependencies\",
        r"windows-x86_64-release\runtime-extensions\physics-simulation\extension-resources"
    );
    presenter
        .borrow_mut()
        .apply(Event::DirectoryLinkTargetRead {
            request,
            pane,
            navigation,
            target: Some(PathBuf::from(target)),
        });
    crate::refresh(&window, &presenter.borrow());
    assert_eq!(
        window.get_directory_link_tooltip().text.as_str(),
        format!("Link target\n{target}")
    );
    draw();
    let popups = WindowInner::from_pub(window.window()).active_popups();
    let tooltip = popups
        .iter()
        .find(|p| matches!(p.window_kind, WindowKind::ToolTip))
        .unwrap();
    let PopupWindowLocation::ChildWindow(origin) = tooltip.location else {
        panic!("Expected embedded folder link tooltip");
    };
    let popup = ItemRc::new_root(tooltip.component.clone())
        .downcast::<WindowItem>()
        .unwrap();
    let width = popup.as_pin_ref().width().get();
    assert!(width > 560., "long target is still limited to {width}px");
    assert!(origin.x >= 0. && origin.x + width <= 1_000.1);
    drop(popups);

    move_to("ordinary");
    time.set(Duration::from_millis(1_250));
    draw();
    assert_eq!(lookups().len(), 1);
    assert!(window.get_directory_link_tooltip().row.is_empty());
    window.hide().unwrap();
}
