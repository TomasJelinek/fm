#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::{PopupWindowLocation, WindowInner, WindowKind},
};
use midnight_core::{ARCHIVE_ASSOCIATIONS, ARCHIVE_FORMATS, ArchiveFormat, Command, Event};
use midnight_ui::{ArchiveOptionsView, Presenter};
use slint::{ComponentHandle, Model, platform::WindowEvent};
use std::{cell::RefCell, rc::Rc};
use ui::{TestUi, click_popup};

fn settle(ui: &mut TestUi) {
    // Slint deliberately updates popup placement on the next event-loop tick.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        ui.draw();
    }
}

fn assert_options_inside_window(ui: &TestUi) {
    let inner = WindowInner::from_pub(ui.window.window());
    let (item, origin) = {
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .unwrap();
        let PopupWindowLocation::ChildWindow(origin) = popup.location else {
            panic!("software popup must be embedded")
        };
        (ItemRc::new_root(popup.component.clone()), origin)
    };
    let size = item.geometry().size;
    assert!(
        origin.x >= 0.0 && origin.y >= 0.0,
        "popup origin {origin:?}"
    );
    assert!(
        origin.x + size.width <= ui.width as f32 + 0.5,
        "popup right {} exceeds {}",
        origin.x + size.width,
        ui.width
    );
    assert!(
        origin.y + size.height <= ui.height as f32 + 0.5,
        "popup bottom {} exceeds {}",
        origin.y + size.height,
        ui.height
    );
    let done = ui::popup_item(&ui.window, "Cancel", AccessibleRole::Button);
    let rect = done.geometry();
    let origin = done.map_to_native_window(rect.origin);
    assert!(origin.x >= 0.0 && origin.y >= 0.0);
    assert!(origin.x + rect.size.width <= ui.width as f32 + 0.5);
    assert!(
        origin.y + rect.size.height <= ui.height as f32 + 0.5,
        "Done origin {origin:?}, size {:?}, window {}x{}",
        rect.size,
        ui.width,
        ui.height
    );
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.size.width,
        origin.y + rect.size.height,
    )
}

fn scroll_to_end(ui: &mut TestUi, target: &ItemRc, bottom: bool) {
    let mut parent = target.parent_item(ParentItemTraversalMode::StopAtPopups);
    let viewport = loop {
        let item = parent.expect("archive controls must belong to the Options scroll view");
        if item.downcast::<Flickable>().is_some() {
            break item;
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    };
    let clip = bounds(&viewport);
    let content_height = viewport
        .downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_height()
        .get();
    if content_height <= clip.3 - clip.1 {
        return;
    }
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(clip.0 + 3.0, clip.1 + 3.0),
            delta_x: 0.0,
            delta_y: if bottom { -100_000.0 } else { 100_000.0 },
        });
    settle(ui);
    let target_bounds = bounds(target);
    assert!(
        target_bounds.1 >= clip.1 && target_bounds.3 <= clip.3 - 14.0,
        "wheel must expose the complete control above the scrollbar overlay: control={target_bounds:?}, viewport={clip:?}"
    );
}

fn archive_options() -> TestUi {
    let mut ui = TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::install(&ui.window, presenter, Rc::new(|_| true));
    ui.window.invoke_show_options();
    ui.window.set_options_topic_index(7);
    settle(&mut ui);
    ui
}

fn find_visible(item: ItemRc, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    if item.is_visible()
        && item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find_visible(item, label, role) {
            return Some(found);
        }
    }
    None
}

fn has_archive_control(ui: &TestUi, label: &str, role: AccessibleRole) -> bool {
    WindowInner::from_pub(ui.window.window())
        .active_popups()
        .iter()
        .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .any(|popup| find_visible(ItemRc::new_root(popup.component.clone()), label, role).is_some())
}

fn open_extension_input(ui: &mut TestUi) {
    let add = ui::popup_item(
        &ui.window,
        "Add custom archive extension",
        AccessibleRole::Button,
    );
    scroll_to_end(ui, &add, true);
    click_popup(
        &ui.window,
        "Add custom archive extension",
        AccessibleRole::Button,
    );
    settle(ui);
}

fn submit_extension(ui: &mut TestUi, draft: &str) {
    if !has_archive_control(ui, "Custom archive extension", AccessibleRole::TextInput) {
        open_extension_input(ui);
    }
    ui.window
        .global::<ArchiveOptionsView>()
        .set_draft(draft.into());
    ui::key(&ui.window, slint::platform::Key::Return);
    settle(ui);
}

#[test]
fn plus_reveals_focused_input_and_enter_stages_a_removable_extension() {
    let mut ui = archive_options();
    assert!(
        !has_archive_control(&ui, "Custom archive extension", AccessibleRole::TextInput),
        "the add form is hidden until the compact plus is clicked"
    );
    open_extension_input(&mut ui);
    ui::key(&ui.window, "personal.文档");
    assert_eq!(
        ui.window.global::<ArchiveOptionsView>().get_draft(),
        "personal.文档",
        "the newly revealed input receives keyboard focus"
    );
    ui::key(&ui.window, slint::platform::Key::Return);
    settle(&mut ui);
    let view = ui.window.global::<ArchiveOptionsView>();
    assert!(
        view.get_custom_extensions()
            .iter()
            .any(|extension| extension == "personal.文档")
    );
    assert!(!has_archive_control(
        &ui,
        "Custom archive extension",
        AccessibleRole::TextInput
    ));
    let remove = ui::popup_item(&ui.window, "Remove .personal.文档", AccessibleRole::Button);
    let chip = ui::popup_item(&ui.window, ".personal.文档", AccessibleRole::ListItem);
    assert!(
        ui::visible_text_values(remove.clone())
            .iter()
            .any(|text| text == "×")
    );
    assert!(
        bounds(&remove).0 < (bounds(&chip).0 + bounds(&chip).2) / 2.0,
        "dismissal leads the custom chip"
    );
    assert!(find_visible(chip, ".personal.文档", AccessibleRole::Checkbox).is_none());
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    ui.window.invoke_show_options();
    settle(&mut ui);
    assert_eq!(
        ui.window
            .global::<ArchiveOptionsView>()
            .get_custom_extensions()
            .row_count(),
        0,
        "Cancel discards the staged extension"
    );
}

#[test]
fn wheel_reveals_last_archive_checkbox_after_resizing_options() {
    let mut ui = archive_options();
    let window = ui.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    for (width, height, font_size) in [
        (1340, 820, 13),
        (640, 420, 13),
        (320, 500, 13),
        (1040, 600, 32),
    ] {
        ui.resize(width, height);
        ui.window.invoke_font_options_changed(0, font_size, 15, 1);
        settle(&mut ui);
        let row = view.get_rows().iter().last().unwrap();
        let label = format!(".{} · {}", row.extension, row.label);
        let checkbox = ui::popup_item(&ui.window, &label, AccessibleRole::Checkbox);
        let footer_before = bounds(&ui::popup_item(&ui.window, "OK", AccessibleRole::Button));
        scroll_to_end(&mut ui, &checkbox, true);
        assert_options_inside_window(&ui);
        click_popup(&ui.window, &label, AccessibleRole::Checkbox);
        settle(&mut ui);
        assert_eq!(view.get_rows().iter().last().unwrap().enabled, !row.enabled);
        assert_eq!(
            bounds(&ui::popup_item(&ui.window, "OK", AccessibleRole::Button)),
            footer_before,
            "scrolling or toggling associations must not move the footer"
        );
        render::snapshot(
            &ui,
            &format!("archives-scrolled-{width}x{height}-{font_size}"),
        );
    }
}

#[test]
fn custom_extensions_can_be_added_and_removed_through_the_scrolled_form() {
    let mut ui = archive_options();
    let window = ui.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    for index in 0..32 {
        view.set_draft(format!("custom{index:02}").into());
        view.invoke_add();
        assert!(view.get_error().is_empty());
    }
    view.invoke_search("no matching built-in extension".into());
    ui.resize(640, 420);
    settle(&mut ui);
    assert_eq!(view.get_rows().row_count(), 0);
    let remove = ui::popup_item(&ui.window, "Remove .custom31", AccessibleRole::Button);
    scroll_to_end(&mut ui, &remove, true);
    render::snapshot(&ui, "archives-custom-scrolled");
    click_popup(&ui.window, "Remove .custom31", AccessibleRole::Button);
    settle(&mut ui);
    assert_eq!(view.get_custom_extensions().row_count(), 31);
    assert!(
        !view
            .get_custom_extensions()
            .iter()
            .any(|value| value == "custom31")
    );
    submit_extension(&mut ui, "after-scroll.文档");
    assert!(view.get_error().is_empty());
    assert!(
        view.get_custom_extensions()
            .iter()
            .any(|value| value == "after-scroll.文档")
    );
    let wrapped_extension = "文档备份".repeat(8);
    submit_extension(&mut ui, &wrapped_extension);
    assert!(view.get_error().is_empty());
    ui.resize(640, 640);
    ui.window.invoke_font_options_changed(0, 32, 15, 1);
    settle(&mut ui);
    let label = format!("Remove .{wrapped_extension}");
    let remove = ui::popup_item(&ui.window, &label, AccessibleRole::Button);
    scroll_to_end(&mut ui, &remove, true);
    let remove_bounds = bounds(&remove);
    assert!(remove_bounds.0 >= 0.0 && remove_bounds.2 <= ui.width as f32);
    render::snapshot(&ui, "archives-custom-wrapped-large-font");
    click_popup(&ui.window, &label, AccessibleRole::Button);
    settle(&mut ui);
    assert!(
        !view
            .get_custom_extensions()
            .iter()
            .any(|value| value == wrapped_extension)
    );
    assert_options_inside_window(&ui);
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert!(!ui::has_popup(&ui.window));
}

#[test]
fn custom_extensions_and_add_control_align_with_the_builtin_list() {
    let mut ui = archive_options();
    let window = ui.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    for extension in ["mebackup", "nightly.backup"] {
        view.set_draft(extension.into());
        view.invoke_add();
        assert!(view.get_error().is_empty());
    }
    for (width, height, font_size) in [(1340, 820, 13), (640, 640, 24), (320, 500, 13)] {
        ui.resize(width, height);
        ui.window.invoke_font_options_changed(0, font_size, 15, 1);
        settle(&mut ui);
        let remove = ui::popup_item(&ui.window, "Remove .nightly.backup", AccessibleRole::Button);
        scroll_to_end(&mut ui, &remove, true);
        render::snapshot(
            &ui,
            &format!("archives-custom-alignment-{width}-{font_size}"),
        );
        let row = view.get_rows().iter().last().unwrap();
        let builtin = ui::popup_item(
            &ui.window,
            &format!(".{} · {}", row.extension, row.label),
            AccessibleRole::Checkbox,
        );
        let list_left = bounds(&builtin).0;
        for name in [".mebackup", ".nightly.backup"] {
            let chip = ui::popup_item(&ui.window, name, AccessibleRole::ListItem);
            assert!(
                (bounds(&chip).0 - list_left).abs() <= 1.0,
                "custom {name} must align with built-ins: chip={:?}, builtin={:?}",
                bounds(&chip),
                bounds(&builtin)
            );
        }
        let add = ui::popup_item(
            &ui.window,
            "Add custom archive extension",
            AccessibleRole::Button,
        );
        assert!(
            (bounds(&add).0 - list_left).abs() <= 1.0,
            "plus must align with built-ins: add={:?}, builtin={:?}",
            bounds(&add),
            bounds(&builtin)
        );
        let first = bounds(&ui::popup_item(
            &ui.window,
            ".mebackup",
            AccessibleRole::ListItem,
        ));
        let second = bounds(&ui::popup_item(
            &ui.window,
            ".nightly.backup",
            AccessibleRole::ListItem,
        ));
        assert!(second.1 >= first.3 && bounds(&add).1 >= second.3);
        assert_options_inside_window(&ui);
    }
}

#[test]
fn archives_topic_has_individual_archive_and_document_checkboxes() {
    let mut ui = TestUi::new();
    let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let saved = Rc::new(RefCell::new(Vec::new()));
    let captured = saved.clone();
    midnight_ui::install(
        &ui.window,
        presenter,
        Rc::new(move |command| {
            if let Command::SavePreferences { preferences, .. } = command {
                captured.borrow_mut().push(preferences);
            }
            true
        }),
    );
    ui.window.invoke_show_options();
    ui.draw();
    ui.options_topic(7);
    ui.draw();
    assert_eq!(ui.window.get_options_topic_index(), 7);
    ui::popup_item(&ui.window, ".zip · ZIP archive", AccessibleRole::Checkbox);
    render::snapshot(&ui, "archives-default");
    let window = ui.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    let zip_count = ARCHIVE_ASSOCIATIONS
        .iter()
        .filter(|known| known.format == ArchiveFormat::Zip)
        .count();
    assert_eq!(view.get_rows().row_count(), zip_count);
    assert_eq!(view.get_formats().row_count(), ARCHIVE_FORMATS.len());
    click_popup(&ui.window, ".zip · ZIP archive", AccessibleRole::Checkbox);
    assert!(
        !view
            .get_rows()
            .iter()
            .find(|row| row.extension == "zip")
            .unwrap()
            .enabled
    );
    assert!(saved.borrow().is_empty(), "association changes are staged");
    view.invoke_search("DOCX".into());
    ui.draw();
    assert_eq!(view.get_rows().row_count(), 1);
    assert!(!view.get_rows().row_data(0).unwrap().enabled);
    render::snapshot(&ui, "archives-document-filter");
    for enabled in [true, false] {
        click_popup(
            &ui.window,
            ".docx · Word document / template",
            AccessibleRole::Checkbox,
        );
        ui.draw();
        assert_eq!(view.get_rows().row_data(0).unwrap().enabled, enabled);
        assert!(saved.borrow().is_empty());
    }
    view.invoke_search("".into());
    assert_eq!(view.get_rows().row_count(), zip_count);
    for (index, format) in ARCHIVE_FORMATS.iter().copied().enumerate() {
        view.invoke_choose_format(index as i32);
        ui.draw();
        assert_eq!(
            view.get_rows().row_count(),
            ARCHIVE_ASSOCIATIONS
                .iter()
                .filter(|known| known.format == format)
                .count()
        );
        let draft = format!(".CUSTOM{index}");
        submit_extension(&mut ui, &draft);
        assert!(view.get_error().is_empty());
        assert!(view.get_draft().is_empty());
        assert_eq!(view.get_custom_extensions().row_count(), 1);
        assert_eq!(
            view.get_custom_extensions().row_data(0).unwrap(),
            format!("custom{index}")
        );
        assert!(saved.borrow().is_empty());
    }
    // Conflicts keep the draft and never replace another decoder's assignment.
    submit_extension(&mut ui, "CUSTOM0");
    assert!(view.get_error().contains("already assigned to ZIP"));
    assert_eq!(view.get_draft(), "CUSTOM0");
    submit_extension(&mut ui, ".DOCX");
    assert!(view.get_error().contains("existing checkbox"));
    click_popup(&ui.window, "Show built-in checkbox", AccessibleRole::Button);
    ui.draw();
    assert_eq!(view.get_format_index(), 0);
    assert_eq!(view.get_rows().row_count(), 1);
    assert_eq!(view.get_rows().row_data(0).unwrap().extension, "docx");
    assert_eq!(view.get_draft(), ".DOCX");
    submit_extension(&mut ui, "*.unsafe");
    assert!(!view.get_error().is_empty());
    assert_eq!(view.get_draft(), "*.unsafe");
    render::snapshot(&ui, "archives-custom-validation");
    click_popup(&ui.window, "Remove .custom0", AccessibleRole::Button);
    ui.draw();
    assert!(view.get_error().is_empty());
    assert_eq!(view.get_custom_extensions().row_count(), 0);
    assert!(saved.borrow().is_empty());
    view.set_draft("unfinished.文档".into());
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    ui.window.invoke_show_options();
    settle(&mut ui);
    assert!(
        view.get_draft().is_empty(),
        "Cancel discards the extension text draft"
    );
    view.invoke_search("".into());
    ui.window.invoke_font_options_changed(0, 32, 15, 1);
    ui.window.set_options_topic_index(0);
    settle(&mut ui);
    ui.options_topic(7);
    settle(&mut ui);
    assert_eq!(ui.window.get_options_topic_index(), 7);
    assert_options_inside_window(&ui);
    render::snapshot(&ui, "archives-large-font");
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert!(!ui::has_popup(&ui.window));
    ui.window.invoke_show_options();
    settle(&mut ui);
    assert_options_inside_window(&ui);
    ui.resize(1040, 600);
    settle(&mut ui);
    assert_options_inside_window(&ui);
    ui.window.set_options_topic_index(0);
    settle(&mut ui);
    ui.options_topic(7);
    settle(&mut ui);
    assert_eq!(ui.window.get_options_topic_index(), 7);
    render::snapshot(&ui, "archives-large-font-small-window");
    ui.window.invoke_preferences_changed(0, 1, 1);
    settle(&mut ui);
    assert_options_inside_window(&ui);
    click_popup(&ui.window, "Cancel", AccessibleRole::Button);
    assert!(!ui::has_popup(&ui.window));
}
