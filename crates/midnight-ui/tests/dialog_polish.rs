//! The UI polish gate's dialog findings of 20 November: the Help window has no second
//! close button (U-D01), close buttons have tooltips (U-D02), the operation dialogs'
//! own Tab cycle reaches the header's close button (U-D03), dialog-wide key routers
//! are no Tab stops (U-D04), the name fields select with the theme's colours (U-D07),
//! a notice with details and About fit 320×200 (U-D08), and Hot Paths shortens the
//! current folder in its middle (U-D11).
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
use crate::support::{modeless, ui};

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip, ComplexText, FocusScope, SimpleText, TextInput, TooltipArea},
    window::{WindowInner, WindowKind},
};
use midnight_ui::{
    ComparisonView, DecisionWindow, FileListPresentation, HelpWindow, HotPathsPresentation,
    JobProgressWindow, JobsWindow, NoticePresentation, OperationPresentation,
    PanelFilterPresentation, PanelFilterView, Theme,
};
use slint::{
    Color, ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

fn platform() -> Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    windows
}

fn counter() -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let count = Rc::new(Cell::new(0));
    (count.clone(), count)
}

fn focus_item(window: &slint::Window) -> Option<ItemRc> {
    WindowInner::from_pub(window).focus_item.borrow().upgrade()
}

/// The control that has the keyboard: the focused item or its nearest ancestor with an
/// accessible role, as (role, label).
fn focused(window: &slint::Window) -> Option<(AccessibleRole, String)> {
    let mut item = focus_item(window);
    while let Some(current) = item {
        let role = current.accessible_role();
        if role != AccessibleRole::None {
            let label = current
                .accessible_string_property(AccessibleStringProperty::Label)
                .unwrap_or_default();
            return Some((role, label.to_string()));
        }
        item = current.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

fn tab(native: &MinimalSoftwareWindow, backwards: bool) {
    if backwards {
        input::chord(native, &[Key::Shift], Key::Tab);
    } else {
        input::key(native, Key::Tab);
    }
    slint::platform::update_timers_and_animations();
}

/// No `visible: false` ancestor hides `item` (Slint lowers `visible` to a clipping
/// visibility clip); a control merely scrolled out of view still counts as shown.
fn not_hidden(item: &ItemRc) -> bool {
    let mut current = Some(item.clone());
    while let Some(item) = current {
        if item.downcast::<Clip>().is_some_and(|clip| {
            let clip = clip.as_pin_ref();
            clip.is_visibility_clip() && clip.clip()
        }) {
            return false;
        }
        current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    true
}

fn descendants(item: &ItemRc, found: &mut Vec<ItemRc>) {
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        found.push(current.clone());
        descendants(&current, found);
    }
}

fn text_of(item: &ItemRc) -> Option<String> {
    if let Some(text) = item.downcast::<SimpleText>() {
        return Some(text.as_pin_ref().text().to_string());
    }
    item.downcast::<ComplexText>()
        .map(|text| text.as_pin_ref().text().to_string())
}

/// An item's bounds in window coordinates: (left, top, right, bottom).
fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

// ---------------------------------------------------------------------------------
// U-D01: no second close button in the framed Help window.

#[test]
fn the_help_window_leaves_closing_to_its_title_bar_close_and_escape() {
    let windows = platform();
    let help = HelpWindow::new().unwrap();
    let (dismissed, count) = counter();
    help.on_request_dismiss(move || count.set(count.get() + 1));
    help.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    modeless::layout(&native, 740, 600);
    let shown = |label: &str| {
        modeless::accessible_items(&native, AccessibleRole::Button, label)
            .into_iter()
            .filter(|item| item.is_visible())
            .count()
    };
    assert_eq!(
        shown("Close dialog"),
        0,
        "the window's title bar already closes Help"
    );
    assert_eq!(shown("Close"), 1, "Close stays in the footer");
    input::key(&native, Key::Escape);
    assert_eq!(dismissed.get(), 1, "Escape still closes Help");
}

// ---------------------------------------------------------------------------------
// U-D02: the icon-only close buttons name their action and key in a tooltip.

/// Rest the pointer on a button of the open dialog until its tooltip shows, and return
/// the tooltip's texts.
fn hover_tooltip(test: &mut ui::TestUi, clock: &Cell<Duration>, label: &str) -> Vec<String> {
    let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let (left, top, right, bottom) = bounds(&item);
    drop(item);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new((left + right) / 2.0, (top + bottom) / 2.0),
        });
    ui::advance(clock, Duration::from_millis(600));
    test.draw();
    let inner = WindowInner::from_pub(test.window.window());
    let popups = inner.active_popups();
    let tooltip = popups
        .iter()
        .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap_or_else(|| panic!("hovering {label} shows no tooltip"));
    ui::text_values(ItemRc::new_root(tooltip.component.clone()))
        .into_iter()
        .map(|text| text.to_string())
        .filter(|text| !text.is_empty())
        .collect()
}

/// Leave the window with the pointer and close any shown tooltip before the dialog
/// that owns it closes, as the application does.
fn dismiss_tooltips(test: &mut ui::TestUi) {
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    let inner = WindowInner::from_pub(test.window.window());
    let tooltips: Vec<_> = inner
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| popup.popup_id)
        .collect();
    for tooltip in tooltips {
        inner.close_popup(tooltip);
    }
    test.draw();
}

#[test]
fn header_close_buttons_name_their_action_and_key_in_a_tooltip() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    {
        let form = test.window.global::<OperationPresentation>();
        form.set_token("operation".into());
        form.set_kind(0);
        form.set_title_text("Rename".into());
        form.set_confirm_text("Rename".into());
        form.set_has_name(true);
        form.set_input_label("New name".into());
        form.set_initial_name("draft.txt".into());
        form.invoke_begin();
    }
    test.draw();
    assert_eq!(
        hover_tooltip(&mut test, &clock, "Cancel dialog"),
        ["Cancel dialog (Escape)"]
    );
    dismiss_tooltips(&mut test);
    test.window
        .global::<OperationPresentation>()
        .set_active(false);
    test.draw();
    {
        let notice = test.window.global::<NoticePresentation>();
        notice.set_title("Cannot open folder".into());
        notice.set_body("Access to this folder was denied.".into());
        notice.set_generation(1);
        notice.set_token("notice".into());
    }
    test.draw();
    assert_eq!(
        hover_tooltip(&mut test, &clock, "Close dialog"),
        ["Close dialog (Escape)"]
    );
    dismiss_tooltips(&mut test);
}

// ---------------------------------------------------------------------------------
// U-D03: the operation dialogs' own Tab cycle includes the header's close button.

#[test]
fn operation_dialogs_reach_their_header_close_with_tab_and_shift_tab() {
    let host = form::Host::new();
    let form = host.main.global::<OperationPresentation>();
    let (cancelled, count) = counter();
    form.on_request_dismiss(move |_| count.set(count.get() + 1));
    let (confirmed, count) = counter();
    form.on_confirm(move |_, _| count.set(count.get() + 1));
    form.set_token("operation".into());
    let window = host.main.window();
    let button = |label: &str| Some((AccessibleRole::Button, label.to_owned()));

    // Rename: the name first, then Cancel, Rename and the header's ×, in that order.
    form.set_kind(0);
    form.set_title_text("Rename".into());
    form.set_confirm_text("Rename".into());
    form.set_has_name(true);
    form.set_input_label("New name".into());
    form.set_initial_name("old 文档.txt".into());
    form.set_selection_end("old 文档".len() as i32);
    form.invoke_begin();
    host.layout(640, 420);
    let cycle = [
        Some((AccessibleRole::TextInput, "New name".to_owned())),
        button("Cancel"),
        button("Rename"),
        button("Cancel dialog"),
    ];
    assert_eq!(focused(window), cycle[0], "Rename opens in its name");
    for step in 1..=cycle.len() {
        tab(&host.native, false);
        assert_eq!(focused(window), cycle[step % cycle.len()], "Tab {step}");
    }
    for (step, expected) in cycle.iter().enumerate().rev() {
        tab(&host.native, true);
        assert_eq!(&focused(window), expected, "Shift+Tab to {step}");
    }
    // Shift+Tab from the name reaches the ×, which Enter and Space press.
    tab(&host.native, true);
    assert_eq!(focused(window), button("Cancel dialog"));
    host.key(Key::Return);
    host.key(Key::Space);
    assert_eq!(cancelled.get(), 2, "Enter and Space on the × cancel");
    assert_eq!(confirmed.get(), 0, "the × never confirms");

    // Move to Trash: no name; the primary action first, then the ×, Cancel and back.
    form.set_kind(1);
    form.set_title_text("Move to Trash".into());
    form.set_confirm_text("Move to Trash".into());
    form.set_has_name(false);
    form.invoke_begin();
    host.layout(640, 420);
    // The hidden name field takes no room in the form.
    assert_eq!(
        host.item(AccessibleRole::TextInput, "New name")
            .geometry()
            .height(),
        0.0,
        "a form without a name keeps no room for one"
    );
    let cycle = [
        button("Move to Trash"),
        button("Cancel dialog"),
        button("Cancel"),
    ];
    assert_eq!(focused(window), cycle[0]);
    for step in 1..=cycle.len() {
        tab(&host.native, false);
        assert_eq!(focused(window), cycle[step % cycle.len()], "Tab {step}");
    }
    tab(&host.native, true);
    assert_eq!(focused(window), cycle[2], "Shift+Tab from the primary");
    tab(&host.native, true);
    assert_eq!(focused(window), cycle[1], "Shift+Tab from Cancel");
    host.key(Key::Escape);
    assert_eq!(cancelled.get(), 3, "Escape still cancels from the ×");
    assert_eq!(confirmed.get(), 0);
}

// ---------------------------------------------------------------------------------
// U-D04: a dialog-wide key router (a FocusScope around the dialog) is no Tab stop.

/// The FocusScopes around `control`: the dialog-wide key routers that hold it.
fn routers(control: &ItemRc) -> Vec<ItemRc> {
    let mut found = Vec::new();
    let mut parent = control.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<FocusScope>().is_some() {
            found.push(ancestor.clone());
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    found
}

/// Tab and then Shift+Tab `presses` times each: the focus moves between controls and
/// never rests on a key router around `control`, which would show no focus.
fn assert_tab_skips_routers(
    native: &MinimalSoftwareWindow,
    control: &ItemRc,
    what: &str,
    presses: usize,
) {
    let routers = routers(control);
    assert!(
        !routers.is_empty(),
        "{what} routes its keys through a FocusScope"
    );
    for backwards in [false, true] {
        let key = if backwards { "Shift+Tab" } else { "Tab" };
        for press in 1..=presses {
            tab(native, backwards);
            let focus = focus_item(native.window())
                .unwrap_or_else(|| panic!("{what}: {key} {press} lost the keyboard focus"));
            assert!(
                !routers.contains(&focus),
                "{what}: {key} {press} rests on a key router, which shows no focus"
            );
        }
    }
}

/// Lay out the open embedded dialog, cycle through it with Tab both ways and close it
/// with Escape, which must still reach its router.
fn check_embedded(host: &form::Host, close: &str, what: &str, presses: usize) {
    host.layout(1000, 700);
    assert!(host.is_open(), "{what} is open");
    let control = host.item(AccessibleRole::Button, close);
    assert_tab_skips_routers(&host.native, &control, what, presses);
    drop(control);
    host.key(Key::Escape);
    assert!(!host.is_open(), "{what}: Escape still closes it");
}

#[test]
fn embedded_dialogs_route_their_keys_without_a_tab_stop_of_their_own() {
    let host = form::Host::new();
    let main = host.main.clone_strong();

    let notice = main.global::<NoticePresentation>();
    let weak = main.as_weak();
    notice.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<NoticePresentation>().set_token("".into());
        }
    });
    notice.set_title("Cannot open folder".into());
    notice.set_body("Access to this folder was denied.".into());
    notice.set_details("Error kind: PermissionDenied".into());
    notice.set_generation(1);
    notice.set_token("notice".into());
    check_embedded(&host, "Close dialog", "Notice", 12);

    main.invoke_show_about();
    check_embedded(&host, "Close", "About", 12);

    let hot_paths = main.global::<HotPathsPresentation>();
    let weak = main.as_weak();
    hot_paths.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<HotPathsPresentation>().set_token("".into());
        }
    });
    hot_paths.set_path("/home/user/文档".into());
    hot_paths.set_path_tail("/文档".into());
    hot_paths.set_token("hot-paths".into());
    check_embedded(&host, "Close dialog", "Hot Paths", 16);

    let filter = main.global::<PanelFilterPresentation>();
    let weak = main.as_weak();
    filter.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<PanelFilterPresentation>()
                .set_token("".into());
        }
    });
    let (help, count) = counter();
    filter.on_show_help(move |_| count.set(count.get() + 1));
    filter.set_model(PanelFilterView {
        open: true,
        expression: "*.txt".into(),
        ..Default::default()
    });
    filter.set_token("filter".into());
    host.layout(1000, 700);
    host.key(Key::F1);
    assert_eq!(help.get(), 1, "F1 still reaches the Filter dialog's router");
    check_embedded(&host, "Cancel dialog", "Panel filter", 20);

    let list = main.global::<FileListPresentation>();
    let weak = main.as_weak();
    list.on_cancel(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<FileListPresentation>().set_active(false);
        }
    });
    list.set_token("file-list".into());
    list.invoke_begin();
    check_embedded(&host, "Cancel file list", "Make File List", 30);

    let weak = main.as_weak();
    main.on_cancel_comparison(move || {
        if let Some(main) = weak.upgrade() {
            let mut model = main.get_comparison_view();
            model.open = false;
            main.set_comparison_view(model);
        }
    });
    main.set_comparison_view(ComparisonView {
        open: true,
        left_path: "/left 文档".into(),
        right_path: "/right".into(),
        ..Default::default()
    });
    check_embedded(&host, "Cancel dialog", "Compare directories", 30);

    let weak = main.as_weak();
    main.on_cancel_metadata(move |_| {
        if let Some(main) = weak.upgrade() {
            main.set_metadata_token("".into());
        }
    });
    main.set_metadata_can_windows(true);
    main.set_metadata_token("attributes:1".into());
    check_embedded(&host, "Cancel dialog", "Attributes", 40);

    main.set_options_open(true);
    check_embedded(&host, "Cancel dialog", "Options", 120);
}

/// Show a tool window, cycle through it with Tab both ways and close it with Escape.
fn check_tool_window(
    windows: &Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    size: (u32, u32),
    close: &str,
    what: &str,
    dismissed: &Cell<usize>,
) {
    let native = windows.borrow().last().unwrap().clone();
    modeless::layout(&native, size.0, size.1);
    let control = modeless::accessible_item(&native, AccessibleRole::Button, close)
        .unwrap_or_else(|| panic!("{what} has no {close}"));
    assert_tab_skips_routers(&native, &control, what, 16);
    drop(control);
    let before = dismissed.get();
    input::key(&native, Key::Escape);
    assert_eq!(
        dismissed.get(),
        before + 1,
        "{what}: Escape still closes it"
    );
}

#[test]
fn tool_windows_route_their_keys_without_a_tab_stop_of_their_own() {
    let windows = platform();
    let (dismissed, count) = counter();

    let help = HelpWindow::new().unwrap();
    let sink = count.clone();
    help.on_request_dismiss(move || sink.set(sink.get() + 1));
    help.show().unwrap();
    check_tool_window(&windows, (740, 600), "Close", "Help", &dismissed);

    let decision = DecisionWindow::new().unwrap();
    let sink = count.clone();
    decision.on_request_dismiss(move || sink.set(sink.get() + 1));
    decision.set_token("1".into());
    decision.set_body("An item with this name already exists.".into());
    decision.set_initial_name("copy 文档.txt".into());
    decision.set_allow_alternate_name(true);
    decision.show().unwrap();
    check_tool_window(&windows, (700, 500), "Decide later", "Decision", &dismissed);

    let jobs = JobsWindow::new().unwrap();
    let sink = count.clone();
    jobs.on_request_dismiss(move || sink.set(sink.get() + 1));
    jobs.show().unwrap();
    check_tool_window(&windows, (720, 500), "Close", "Jobs", &dismissed);

    let progress = JobProgressWindow::new().unwrap();
    let sink = count.clone();
    progress.on_request_dismiss(move || sink.set(sink.get() + 1));
    progress.show().unwrap();
    check_tool_window(&windows, (640, 420), "Close", "Job progress", &dismissed);
}

// ---------------------------------------------------------------------------------
// U-D07: selected text in the name fields uses the theme's selection colours.

fn luminance(color: Color) -> f64 {
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

fn contrast(first: Color, second: Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// `top` drawn with `alpha` over `under`.
fn blend(top: Color, alpha: f64, under: Color) -> Color {
    let mix = |top: u8, under: u8| {
        (f64::from(top) * alpha + f64::from(under) * (1.0 - alpha)).round() as u8
    };
    Color::from_rgb_u8(
        mix(top.red(), under.red()),
        mix(top.green(), under.green()),
        mix(top.blue(), under.blue()),
    )
}

/// The field's selection colours are the theme's, and its selected text reaches 4.5:1
/// as the polish gate measures it: the selection foreground at the text's opacity
/// over the selection background.
fn assert_themed_selection(input: &ItemRc, theme: &Theme<'_>, what: &str) {
    let input = input.downcast::<TextInput>().expect("a text input");
    let input = input.as_pin_ref();
    let foreground = input.selection_foreground_color();
    let background = input.selection_background_color();
    assert_eq!(
        (foreground, background),
        (theme.get_selection_text(), theme.get_selection_background()),
        "{what}: the selection uses the theme's colours"
    );
    let alpha = f64::from(input.color().color().alpha()) / 255.0;
    let shown = blend(foreground, alpha, background);
    let ratio = contrast(shown, background);
    assert!(
        ratio >= 4.5,
        "{what}: selected text {shown:?} on {background:?} is {ratio:.2}:1"
    );
}

#[test]
fn the_rename_copy_and_move_name_field_selects_with_theme_colours_in_every_theme() {
    let host = form::Host::new();
    let form = host.main.global::<OperationPresentation>();
    form.set_token("operation".into());
    form.set_kind(0);
    form.set_title_text("Rename".into());
    form.set_confirm_text("Rename".into());
    form.set_has_name(true);
    form.set_input_label("New name".into());
    form.set_initial_name("Picture 文档.jpg".into());
    form.set_selection_end("Picture 文档".len() as i32);
    for scheme in 0..26 {
        host.apply_preferences(scheme, 0, "", 12, 15);
        form.invoke_begin();
        host.layout(640, 420);
        let input = focus_item(host.main.window()).expect("Rename opens in its name");
        {
            let field = input
                .downcast::<TextInput>()
                .expect("the name's text input");
            let field = field.as_pin_ref();
            assert_ne!(
                field.anchor_position_byte_offset(),
                field.cursor_position_byte_offset(),
                "Rename opens with the name selected"
            );
        }
        assert_themed_selection(
            &input,
            &host.main.global::<Theme>(),
            &format!("Rename, theme {scheme}"),
        );
    }
}

#[test]
fn the_alternate_name_field_selects_with_theme_colours_in_every_theme() {
    let windows = platform();
    let window = DecisionWindow::new().unwrap();
    window.set_token("1".into());
    window.set_body("An item with this name already exists.".into());
    window.set_initial_name("copy 文档.txt".into());
    window.set_allow_alternate_name(true);
    window.show().unwrap();
    let native = windows.borrow().last().unwrap().clone();
    for scheme in 0..26 {
        window.invoke_apply_preferences(scheme, 0, "".into(), 12, 15);
        modeless::layout(&native, 700, 500);
        let field = modeless::accessible_item(
            &native,
            AccessibleRole::TextInput,
            "Copy under a different name",
        )
        .expect("the alternate name field");
        let mut items = vec![field.clone()];
        descendants(&field, &mut items);
        let input = items
            .into_iter()
            .find(|item| item.downcast::<TextInput>().is_some())
            .expect("the field's text input");
        assert_themed_selection(
            &input,
            &window.global::<Theme>(),
            &format!("Copy as, theme {scheme}"),
        );
    }
}

// ---------------------------------------------------------------------------------
// U-D08: a notice with its details and About in small dialogs.

const FOLDER: &str = r"C:\Users\example\Documents\Projects\Midnight Explorer\a folder whose name is long 文档\locked";
const DETAILS: &str = "Error kind: PermissionDenied\nOperation: list folder\nObserved issues: 1\nStored details: 1\nNative diagnostics remain available.";

/// A visible text of the open dialog contains `needle`.
fn shows_text(host: &form::Host, needle: &str) -> bool {
    let Some(root) = host.popup() else {
        return false;
    };
    let mut items = Vec::new();
    descendants(&root, &mut items);
    items.iter().any(|item| {
        item.is_visible()
            && not_hidden(item)
            && text_of(item).is_some_and(|text| text.contains(needle))
    })
}

#[test]
fn notice_details_keep_the_buttons_and_escape_in_small_dialogs() {
    let host = form::Host::new();
    let notice = host.main.global::<NoticePresentation>();
    let weak = host.main.as_weak();
    notice.on_dismiss(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<NoticePresentation>().set_token("".into());
        }
    });
    let mut generation = 0;
    // Blue Desktop and Dark Touch at the minimum size, and a short regular dialog.
    for (scheme, density, width, height) in [
        (0, 0, 320, 200),
        (1, 1, 320, 200),
        (0, 0, 480, 320),
        (1, 1, 480, 360),
    ] {
        let what = format!("{width}×{height}, theme {scheme}, density {density}");
        host.apply_preferences(scheme, density, "", 12, 15);
        generation += 1;
        notice.set_title("Cannot open folder".into());
        notice.set_error(true);
        notice.set_body("Access to this folder was denied.".into());
        notice.set_folder_path(FOLDER.into());
        notice.set_details(DETAILS.into());
        notice.set_generation(generation);
        notice.set_token(format!("notice-{generation}").into());
        host.layout(width, height);
        // With the details hidden, the folder box and the buttons fit as well.
        for label in ["Details", "Copy details", "Close"] {
            form::assert_inside(
                &host.item(AccessibleRole::Button, label),
                &format!("{what}, details hidden: {label}"),
            );
        }
        host.click("Details");
        host.layout(width, height);
        for label in ["Hide details", "Copy details", "Close"] {
            form::assert_inside(
                &host.item(AccessibleRole::Button, label),
                &format!("{what}: {label}"),
            );
        }
        assert!(
            shows_text(&host, "Error kind: PermissionDenied"),
            "{what}: the details are shown"
        );
        assert!(shows_text(&host, "locked"), "{what}: the folder is shown");
        host.key(Key::Escape);
        assert!(!host.is_open(), "{what}: Escape closes the notice");
    }
}

#[test]
fn about_keeps_close_inside_its_dialog_in_the_smallest_window() {
    let host = form::Host::new();
    for (scheme, density) in [(0, 0), (1, 1)] {
        let what = format!("theme {scheme}, density {density}");
        host.apply_preferences(scheme, density, "", 12, 15);
        host.main.invoke_show_about();
        host.layout(320, 200);
        form::assert_inside(
            &host.item(AccessibleRole::Button, "Close"),
            &format!("{what}: About's Close"),
        );
        let licenses = host.item(AccessibleRole::Tab, "Licenses");
        modeless::click_item(&host.native, &licenses, "Licenses");
        drop(licenses);
        host.layout(320, 200);
        form::assert_inside(
            &host.item(AccessibleRole::Button, "Close"),
            &format!("{what}: Close beside the licenses"),
        );
        host.key(Key::Escape);
        assert!(!host.is_open(), "{what}: Escape closes About");
    }
}

// ---------------------------------------------------------------------------------
// U-D11: Hot Paths shortens the current folder in its middle, with the full text as
// its accessible name and tooltip.

#[test]
fn hot_paths_shortens_the_current_folder_in_its_middle_with_its_full_text() {
    let host = form::Host::new();
    let presentation = host.main.global::<HotPathsPresentation>();
    let path = "/home/user/projects/midnight explorer/a folder name long enough to shorten/文档/target folder";
    let tail = "/target folder";
    presentation.set_path(path.into());
    presentation.set_path_tail(tail.into());
    presentation.set_token("hot-paths".into());
    let label = format!("Current folder: {path}");
    for (scheme, density, width, height) in [(0, 0, 1340, 820), (1, 1, 960, 600), (0, 0, 320, 200)]
    {
        let what = format!("{width}×{height}, theme {scheme}, density {density}");
        host.apply_preferences(scheme, density, "", 12, 15);
        host.layout(width, height);
        // The accessible name holds the whole line.
        let line = host.item(AccessibleRole::Text, &label);
        let (left, _, right, _) = bounds(&line);
        let mut items = Vec::new();
        descendants(&line, &mut items);
        let shown = |text: &str| {
            items
                .iter()
                .filter(|item| not_hidden(item) && text_of(item).as_deref() == Some(text))
                .map(bounds)
                .collect::<Vec<_>>()
        };
        let dots = shown("...");
        assert!(
            !dots.is_empty(),
            "{what}: the shortened line shows an ellipsis"
        );
        let marker = dots[0];
        // The start, where it is shown, ends at the ellipsis; the tail follows it.
        for head in shown(&label) {
            assert!(
                (head.2 - marker.0).abs() <= 1.0,
                "{what}: the start ends at the ellipsis: {head:?} {marker:?}"
            );
        }
        let tails = shown(tail);
        assert_eq!(tails.len(), 1, "{what}: the folder's own name is kept");
        assert!(
            (tails[0].0 - marker.2).abs() <= 1.0
                && tails[0].2 <= right + 0.5
                && marker.0 >= left - 0.5,
            "{what}: the name follows the ellipsis inside the line: {marker:?} {:?} {left}..{right}",
            tails[0]
        );
        assert!(
            items
                .iter()
                .any(|item| item.downcast::<TooltipArea>().is_some()),
            "{what}: a tooltip shows the full folder"
        );
    }
}
