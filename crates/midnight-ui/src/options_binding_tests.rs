//! Editors that outlive a selection change keep showing (and writing) the
//! selected item after native input. Fluent controls assign their own
//! `text`/`checked`/`current-index`/`value`, which permanently detaches a
//! one-way binding; these tests drive that native input first.
use crate::{
    ArchiveOptionsView, ColorOptionsView, HotPathsPresentation, UserCommandSettingsView,
    ViewerPluginsView, test_harness::Harness,
};
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, Flickable},
};
use midnight_core::{
    ArchiveFormat, Bookmark, BookmarkId, Command, Event, FilenameColorRule, FilenameColorTarget,
    FilenameStyle, FilterMode, RgbColor, ShellFlavor, UserCommand, UserCommandId,
    UserCommandInvocation, UserCommandLaunch, UserPreferences, ViewerPluginInfo,
};
use slint::{ComponentHandle, platform::Key};
use std::{cell::Cell, rc::Rc, sync::Arc};

/// The first control with `role` whose label starts with `prefix`.
fn labelled(h: &Harness, role: AccessibleRole, prefix: &str) -> ItemRc {
    h.find(|item| {
        item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label.starts_with(prefix))
    })
    .unwrap_or_else(|| panic!("no {role:?} labelled {prefix}…"))
}

fn is_checked(item: &ItemRc) -> bool {
    item.accessible_string_property(AccessibleStringProperty::Checked)
        .is_some_and(|value| value.as_str() == "true")
}

fn rule(expression: &str) -> FilenameColorRule {
    FilenameColorRule {
        expression: expression.into(),
        ..Default::default()
    }
}

fn color_rules() -> UserPreferences {
    UserPreferences {
        filename_colors: vec![
            rule("*.rs"),
            FilenameColorRule {
                enabled: true,
                mode: FilterMode::Masks,
                case_sensitive: false,
                target: FilenameColorTarget::Folders,
                style: FilenameStyle {
                    foreground: Some(RgbColor(0x11, 0x22, 0x33)),
                    background: None,
                },
                ..rule("*.md")
            },
        ]
        .into(),
        ..Default::default()
    }
}

#[test]
fn color_rule_editor_shows_and_edits_only_the_selected_rule() {
    let mut h = Harness::options(color_rules(), 8);
    assert_eq!(h.text("Color rule expression"), "*.rs");
    // Edit every control of rule 1 natively.
    h.type_text("Color rule expression", "*.rsx");
    h.activate("Enable rule", AccessibleRole::Checkbox);
    h.activate("Case sensitive", AccessibleRole::Checkbox);
    h.choose("Color rule syntax", 1);
    h.choose("Color rule targets", 2);
    h.type_text("Foreground hex color", "#445566");
    assert!(!h.checked("Enable rule") && h.checked("Case sensitive"));
    assert_eq!(
        h.value("Color rule syntax", AccessibleRole::Combobox),
        "Regex"
    );

    h.activate("2. *.md", AccessibleRole::ListItem);
    assert_eq!(h.window.global::<ColorOptionsView>().get_selected(), 1);
    assert_eq!(h.text("Color rule expression"), "*.md");
    assert!(h.checked("Enable rule"), "rule 2 is enabled");
    assert!(!h.checked("Case sensitive"), "rule 2 is case-insensitive");
    assert_eq!(
        h.value("Color rule syntax", AccessibleRole::Combobox),
        "Masks"
    );
    assert_eq!(
        h.value("Color rule targets", AccessibleRole::Combobox),
        "Folders"
    );
    assert_eq!(h.text("Foreground hex color"), "#112233");

    // The next keystroke edits rule 2 only; rule 1 keeps its own edits.
    h.type_text("Color rule expression", "*.mdx");
    h.activate("1. [off] *.rsx", AccessibleRole::ListItem);
    assert_eq!(h.text("Color rule expression"), "*.rsx");
    assert_eq!(h.text("Foreground hex color"), "#445566");
    assert!(!h.checked("Enable rule") && h.checked("Case sensitive"));
    assert_eq!(
        h.value("Color rule syntax", AccessibleRole::Combobox),
        "Regex"
    );
    assert_eq!(
        h.value("Color rule targets", AccessibleRole::Combobox),
        "Files and folders"
    );
    h.activate("2. *.mdx", AccessibleRole::ListItem);
    assert_eq!(h.text("Color rule expression"), "*.mdx");
    assert_eq!(h.text("Foreground hex color"), "#112233");

    // Reset after typing clears the visible hex value as well as the draft.
    h.type_text("Foreground hex color", "#abcdef");
    h.activate("Reset", AccessibleRole::Button);
    assert_eq!(
        h.window.global::<ColorOptionsView>().get_draft().foreground,
        ""
    );
    assert_eq!(h.text("Foreground hex color"), "");
}

#[test]
fn colour_picker_controls_follow_the_colour_after_native_input() {
    let mut h = Harness::options(color_rules(), 8);
    h.activate("Choose Foreground", AccessibleRole::Button);
    h.set_value("R value", AccessibleRole::Spinbox, "255");
    h.set_value("Color hue", AccessibleRole::Slider, "200");
    h.set_value("Color saturation", AccessibleRole::Slider, "10");
    h.activate("RGB 0, 0, 0", AccessibleRole::Button);
    assert_eq!(h.value("R value", AccessibleRole::Spinbox), "0");
    assert_eq!(h.value("R color channel", AccessibleRole::Slider), "0");
    h.activate("RGB 239, 68, 68", AccessibleRole::Button);
    assert_eq!(h.value("R value", AccessibleRole::Spinbox), "239");
    let hue: f32 = h
        .value("Color hue", AccessibleRole::Slider)
        .parse()
        .unwrap();
    assert!(hue < 1.0, "red swatch hue, not the typed 200: {hue}");
    let saturation: f32 = h
        .value("Color saturation", AccessibleRole::Slider)
        .parse()
        .unwrap();
    assert!(
        (saturation - 71.5).abs() < 1.0,
        "red swatch saturation, not the typed 10: {saturation}"
    );
}

#[test]
fn an_over_long_color_expression_is_not_applied() {
    let mut h = Harness::options(color_rules(), 8);
    h.type_text("Color rule expression", &"*".repeat(4097));
    let window = h.window.clone_strong();
    let view = window.global::<ColorOptionsView>();
    assert!(
        view.get_error().starts_with("Edit not applied"),
        "{}",
        view.get_error()
    );
    assert_eq!(view.get_draft().expression, "*.rs");
    assert_eq!(
        h.text("Color rule expression"),
        "*.rs",
        "the editor shows the rule's accepted expression again"
    );
}

#[test]
fn return_that_opens_a_combo_box_popup_does_not_press_ok() {
    let mut h = Harness::options(color_rules(), 8);
    let accepted = Rc::new(Cell::new(0));
    let count = accepted.clone();
    h.window
        .on_options_accepted(move || count.set(count.get() + 1));
    h.click("Color rule syntax", AccessibleRole::Combobox);
    h.key(Key::Escape);
    assert!(h.window.get_options_open(), "Escape closed only the popup");
    h.key(Key::Return);
    assert_eq!(
        accepted.get(),
        0,
        "Return opened the list; it must not save"
    );
    assert!(
        h.find(|item| item.accessible_role() == AccessibleRole::ListItem
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|label| label.as_str() == "Regex"))
            .is_some(),
        "the combo box popup is open"
    );
    h.key(Key::Escape);
    // Return on a control that does not handle it still confirms Options.
    h.activate("1. *.rs", AccessibleRole::ListItem);
    h.key(Key::Return);
    assert_eq!(accepted.get(), 1);
}

#[test]
fn reset_to_defaults_updates_controls_the_user_already_changed() {
    let mut h = Harness::options(UserPreferences::default(), 0);
    let label = "Remember command history after restart";
    assert!(h.checked(label));
    h.activate(label, AccessibleRole::Checkbox);
    assert!(!h.checked(label));
    assert!(!h.window.get_remember_command_history(), "staged");
    h.activate("Reset to defaults", AccessibleRole::Button);
    assert!(h.window.get_remember_command_history(), "default restored");
    assert!(h.checked(label), "the edited check box follows Reset");

    h.window.set_options_topic_index(1);
    h.settle();
    let original = h.value("Theme", AccessibleRole::Combobox);
    h.choose("Theme", 1);
    let previewed = h.value("Theme", AccessibleRole::Combobox);
    assert_ne!(previewed, original);
    h.activate("Reset to defaults", AccessibleRole::Button);
    assert_eq!(h.value("Theme", AccessibleRole::Combobox), original);
}

#[test]
fn archive_chooser_and_filter_follow_show_builtin() {
    let mut h = Harness::options(UserPreferences::default(), 7);
    h.choose("Archive format", 1);
    let window = h.window.clone_strong();
    let view = window.global::<ArchiveOptionsView>();
    assert_eq!(view.get_format_index(), 1);
    h.type_text("Search archive associations", "7");
    view.set_draft("zip".into());
    view.invoke_add();
    h.settle();
    assert_eq!(view.get_conflict_extension(), "zip");
    view.invoke_show_builtin();
    h.settle();
    assert_eq!(view.get_format_index(), 0);
    assert_eq!(
        h.value("Archive format", AccessibleRole::Combobox),
        ArchiveFormat::Zip.label()
    );
    assert_eq!(h.text("Search archive associations"), "zip");
}

fn viewer_catalog() -> Vec<ViewerPluginInfo> {
    [
        ("source", "Source viewer", vec!["rs", "py"]),
        ("image", "Image viewer", vec!["png", "jpg"]),
        ("pdf", "PDF viewer", vec!["pdf"]),
    ]
    .into_iter()
    .map(|(id, name, extensions)| ViewerPluginInfo {
        id: format!("midnight.viewer.{id}"),
        name: name.into(),
        description: String::new(),
        version: "1".into(),
        status: "Available".into(),
        location: "viewers".into(),
        fallback: false,
        extensions: extensions.into_iter().map(String::from).collect(),
    })
    .collect()
}

/// Options open on Plugins with a loaded viewer catalog; Source viewer selected.
fn viewer_options() -> Harness {
    let mut h = Harness::new(UserPreferences::default());
    for event in [
        Event::ViewerPluginsListed(Ok(viewer_catalog())),
        Event::FilesystemPluginsListed(Ok(Vec::new())),
        Event::ThumbnailerPluginsListed(Ok(Vec::new())),
    ] {
        h.presenter.borrow_mut().apply(event);
    }
    h.refresh();
    h.window.invoke_show_options();
    h.window.set_options_topic_index(5);
    // Complete the catalog refresh requested by opening Options.
    h.presenter
        .borrow_mut()
        .apply(Event::ViewerPluginsListed(Ok(viewer_catalog())));
    h.refresh();
    h
}

#[test]
fn a_refused_viewer_extension_list_stays_visible_with_its_error() {
    let mut h = viewer_options();
    assert_eq!(h.text("Viewer extensions"), "rs; py");
    // The box binds two-way: a refresh must not swap the refused text for the
    // last accepted list while the error still describes the refused one.
    let refused = "x; ".repeat(11_000);
    assert!(refused.len() > 32 * 1024);
    h.type_text("Viewer extensions", &refused);
    let window = h.window.clone_strong();
    let view = window.global::<ViewerPluginsView>();
    assert!(view.get_error().contains("32 KiB"), "{}", view.get_error());
    assert!(
        h.text("Viewer extensions") == refused,
        "the refused list stays visible for shortening"
    );
    h.activate("OK", AccessibleRole::Button);
    assert!(h.window.get_options_open(), "OK refuses the over-long list");
    assert_eq!(h.window.get_options_topic_index(), 5);
    assert!(h.text("Viewer extensions") == refused);

    h.type_text("Viewer extensions", "rs; py; txt");
    assert_eq!(view.get_error(), "");
    assert_eq!(h.text("Viewer extensions"), "rs; py; txt");
    h.type_text("Viewer extensions", &refused);
    h.activate("Reset viewer extensions", AccessibleRole::Button);
    assert_eq!(view.get_error(), "");
    assert_eq!(
        h.text("Viewer extensions"),
        "rs; py",
        "Reset shows the defaults"
    );
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. })),
        "nothing was saved"
    );
}

#[test]
fn viewer_extensions_follow_the_selected_viewer_after_typing() {
    let mut h = viewer_options();
    assert_eq!(h.text("Viewer extensions"), "rs; py");
    h.type_text("Viewer extensions", ".RS; *.rs; rust");
    h.activate("Image viewer · 1", AccessibleRole::ListItem);
    assert_eq!(h.text("Viewer extensions"), "png; jpg");
    h.type_text("Viewer extensions", "png; jpg; gif");
    h.activate("PDF viewer · 1", AccessibleRole::ListItem);
    assert_eq!(h.text("Viewer extensions"), "pdf");
    h.activate("Image viewer · 1", AccessibleRole::ListItem);
    assert_eq!(
        h.text("Viewer extensions"),
        "png; jpg; gif",
        "only the Image viewer's own edit was staged for it"
    );
    h.activate("Reset viewer extensions", AccessibleRole::Button);
    assert_eq!(h.text("Viewer extensions"), "png; jpg");
    assert_eq!(
        h.window.global::<ViewerPluginsView>().get_extensions(),
        "png; jpg"
    );
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. })),
        "all edits remain Options previews"
    );
}

#[test]
fn hot_path_number_chooser_follows_the_selected_path() {
    let root = std::env::temp_dir().join("hot-path-chooser");
    let bookmark = |id, name: &str, slot| Bookmark {
        id: BookmarkId(id),
        name: name.into(),
        path: root.join(name),
        archive: None,
        network: None,
        slot,
    };
    let mut preferences = UserPreferences {
        bookmarks: vec![bookmark(1, "alpha", Some(3)), bookmark(2, "beta", None)],
        ..Default::default()
    };
    for tabs in &mut preferences.tabs {
        tabs.paths = vec![root.join("current")];
    }
    let mut h = Harness::with_location(preferences, root.join("current"));
    h.window.invoke_open_hot_paths_window(0);
    h.settle();
    let row = |h: &Harness, name: &str| {
        let path = root.join(name).display().to_string();
        h.find(|item| {
            item.accessible_role() == AccessibleRole::Button
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|label| label.as_str().starts_with(&path))
        })
        .unwrap_or_else(|| panic!("no Hot Path row for {name}"))
    };
    let chooser = "Hot Path shortcut";
    let alpha = row(&h, "alpha");
    h.activate_item(alpha);
    let numbered = h.value(chooser, AccessibleRole::Combobox);
    assert!(numbered.starts_with("Hot Path 3"), "{numbered}");
    h.choose(chooser, 2);
    assert!(
        h.value(chooser, AccessibleRole::Combobox)
            .starts_with("Hot Path 5")
    );
    let beta = row(&h, "beta");
    h.activate_item(beta);
    assert_eq!(
        h.value(chooser, AccessibleRole::Combobox),
        "No numbered shortcut",
        "the chooser shows the newly selected path's number"
    );
    let token = h.window.global::<HotPathsPresentation>().get_token();
    assert!(!token.is_empty());
    h.activate("Apply shortcut", AccessibleRole::Button);
    let slots: Vec<_> = h
        .presenter
        .borrow()
        .bookmarks()
        .iter()
        .map(|bookmark| bookmark.slot)
        .collect();
    assert_eq!(slots, [Some(3), None], "Apply used the displayed number");
}

#[test]
fn narrow_topic_chooser_follows_page_changes_after_a_native_pick() {
    let mut h = Harness::options(UserPreferences::default(), 0);
    h.resize(600, 700);
    let chooser = "Options topic";
    assert_eq!(h.value(chooser, AccessibleRole::Combobox), "General");
    h.choose(chooser, 1);
    assert_eq!(h.window.get_options_topic_index(), 1, "picked Appearance");
    // Links and validation errors change the page from outside the chooser.
    h.window.set_options_topic_index(8);
    h.settle();
    assert_eq!(h.value(chooser, AccessibleRole::Combobox), "Colors");
}

#[test]
fn required_value_boxes_follow_discard_after_a_native_toggle() {
    let command = UserCommand {
        id: UserCommandId::new("script").unwrap(),
        name: "Script".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Terminal {
            shell: ShellFlavor::Posix,
            script: "echo ready".into(),
            variables: Vec::new(),
        },
    };
    let preferences = UserPreferences {
        user_commands: Arc::new(vec![command]),
        ..Default::default()
    };
    let mut h = Harness::options(preferences, 9);
    let row = labelled(&h, AccessibleRole::ListItem, "Script");
    h.activate_item(row);
    h.activate("Required terminal values", AccessibleRole::Button);
    let active = |h: &Harness| labelled(h, AccessibleRole::Checkbox, "Active folder");
    assert!(!is_checked(&active(&h)));
    h.activate_item(active(&h));
    let window = h.window.clone_strong();
    let view = window.global::<UserCommandSettingsView>();
    assert!(
        view.get_draft().active_path,
        "the toggle requires the value"
    );
    assert!(is_checked(&active(&h)));
    h.activate("Cancel Custom Command draft", AccessibleRole::Button);
    assert!(
        !view.get_draft().active_path,
        "Discard restored the saved command"
    );
    assert!(
        !is_checked(&active(&h)),
        "the toggled box follows the discarded draft"
    );
}

#[test]
fn compact_picker_keyboard_browsing_reveals_its_row_after_wheel_scrolling() {
    let mut h = Harness::options(UserPreferences::default(), 1);
    let families = (0..60).map(|index| format!("Family {index:02}")).collect();
    h.presenter
        .borrow_mut()
        .apply(Event::FontFamiliesLoaded(Ok(families)));
    h.refresh();
    h.click("Font family", AccessibleRole::Combobox);
    let offset = |h: &Harness| {
        let list = h
            .find(|item| item.downcast::<Flickable>().is_some())
            .expect("the open picker list");
        list.downcast::<Flickable>()
            .unwrap()
            .as_pin_ref()
            .content_y()
            .get()
    };
    assert_eq!(offset(&h), 0.0, "the committed first row is in view");
    let first = h.item("Application default", AccessibleRole::Button);
    let rect = first.geometry();
    let origin = first.map_to_native_window(rect.origin);
    h.wheel(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
        -3000.0,
    );
    assert!(offset(&h) < -100.0, "the wheel scrolled the list away");
    // Down browses to row 1; the list must bring it back into view.
    h.key(Key::DownArrow);
    assert_eq!(offset(&h), 0.0, "keyboard browsing reveals its row");
}
