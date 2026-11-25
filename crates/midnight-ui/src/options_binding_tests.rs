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
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
};
use midnight_core::{
    ArchiveFormat, Bookmark, BookmarkId, Command, Event, FilenameColorRule, FilenameColorTarget,
    FilenameStyle, FilterMode, RgbColor, ShellFlavor, ThumbnailerPluginInfo, UserCommand,
    UserCommandId, UserCommandInvocation, UserCommandLaunch, UserPreferences, ViewerPluginInfo,
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
fn application_icon_picker_previews_restores_and_saves_the_selected_artwork() {
    use i_slint_core::{items::WindowItem, window::WindowInner};
    use midnight_core::AppIcon;

    let mut h = Harness::options(
        UserPreferences {
            app_icon: AppIcon::HarvestNavigator,
            ..Default::default()
        },
        1,
    );
    let root = ItemRc::new_root(WindowInner::from_pub(h.window.window()).component());
    let window = root.downcast::<WindowItem>().unwrap();
    let icon = || window.as_pin_ref().icon();
    let label = "Application icon";
    let chooser = AccessibleRole::Combobox;
    let saved = icon();
    assert_eq!(h.value(label, chooser), "Harvest Navigator");
    h.choose(label, 1);
    assert_eq!(h.value(label, chooser), "Origami Trail");
    let origami = icon();
    assert_ne!(origami, saved);
    let preview = h.item("Origami Trail icon preview", AccessibleRole::Image);
    assert_eq!(
        preview
            .downcast::<i_slint_core::items::ImageItem>()
            .unwrap()
            .as_pin_ref()
            .source(),
        origami
    );
    h.activate("Reset to defaults", AccessibleRole::Button);
    assert_eq!(h.value(label, chooser), "Midnight Moon");
    assert_ne!(icon(), origami);
    h.activate("Cancel", AccessibleRole::Button);
    assert_eq!(icon(), saved);
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::SavePreferences { .. }))
    );

    h.window.invoke_show_options();
    h.window.set_options_topic_index(1);
    h.settle();
    assert_eq!(h.value(label, chooser), "Harvest Navigator");
    h.choose(label, 2);
    assert_eq!(h.value(label, chooser), "Interleave");
    assert_ne!(icon(), saved);
    h.activate("OK", AccessibleRole::Button);
    let commands = h.sent.borrow();
    let mut writes = commands.iter().filter_map(|c| match c {
        Command::SavePreferences { preferences, .. } => Some(preferences.app_icon),
        _ => None,
    });
    assert_eq!(writes.next(), Some(AppIcon::Interleave));
    assert_eq!(writes.next(), None);
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
fn installed_viewer_list_reserves_every_row_and_scrolls_with_the_page() {
    let mut h = viewer_options();
    let mut catalog = viewer_catalog();
    for index in 0..24 {
        let mut plugin = catalog[0].clone();
        plugin.id = format!("midnight.viewer.extra-{index}");
        plugin.name = format!("Installed viewer {index}");
        catalog.push(plugin);
    }
    let installed = catalog.len();
    h.presenter
        .borrow_mut()
        .apply(Event::ViewerPluginsListed(Ok(catalog)));
    h.refresh();
    let list = h.item("Viewer priority", AccessibleRole::List);
    let first = h.item("Source viewer · 1", AccessibleRole::ListItem);
    let row_height = first.geometry().size.height;
    assert!(row_height > 0.0);
    assert!(
        list.geometry().size.height >= installed as f32 * row_height,
        "the catalog gives all installed rows space within the page"
    );
    let viewport = std::iter::successors(Some(list), |item| {
        item.parent_item(ParentItemTraversalMode::StopAtPopups)
    })
    .find(|item| item.downcast::<Flickable>().is_some())
    .expect("the plugin page scroll viewport");
    let screen_top = |item: &ItemRc| item.map_to_native_window(item.geometry().origin).y;
    let first_top = screen_top(&first);
    h.activate("Source viewer · 1", AccessibleRole::ListItem);
    h.key(Key::End);
    assert_eq!(
        h.window.global::<ViewerPluginsView>().get_selected(),
        installed as i32 - 1
    );
    assert!(
        screen_top(&first) < first_top,
        "keyboard selection scrolls the page to the last viewer"
    );

    h.key(Key::Home);
    assert_eq!(h.window.global::<ViewerPluginsView>().get_selected(), 0);
    h.type_text("Viewer extensions", "rs; ../outside");
    h.activate("Thumbnailers", AccessibleRole::Button);
    h.activate("OK", AccessibleRole::Button);
    assert!(h.window.get_options_open());
    assert_eq!(h.text("Viewer extensions"), "rs; ../outside");
    let message = h.window.global::<ViewerPluginsView>().get_extension_error();
    assert!(!message.is_empty());
    let editor = h.item("Viewer extensions", AccessibleRole::TextInput);
    let error = h.item(message.as_str(), AccessibleRole::Text);
    let viewport_top = screen_top(&viewport);
    let viewport_bottom = viewport_top + viewport.geometry().size.height;
    assert!(
        screen_top(&editor) >= viewport_top - 1.0
            && screen_top(&error) + error.geometry().size.height <= viewport_bottom + 1.0,
        "rejected OK brings the editor and its validation into the page viewport"
    );
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
    assert!(
        view.get_extension_error().contains("32 KiB"),
        "{}",
        view.get_extension_error()
    );
    assert!(
        h.text("Viewer extensions") == refused,
        "the refused list stays visible for shortening"
    );

    h.activate("Thumbnailers", AccessibleRole::Button);
    assert!(is_checked(&h.item("Thumbnailers", AccessibleRole::Button)));
    h.activate("OK", AccessibleRole::Button);
    assert!(h.window.get_options_open(), "OK refuses the over-long list");
    assert_eq!(h.window.get_options_topic_index(), 5);
    assert!(is_checked(&h.item("Viewers", AccessibleRole::Button)));
    assert!(h.text("Viewer extensions") == refused);

    h.type_text("Viewer extensions", "rs; py; txt");
    assert_eq!(view.get_extension_error(), "");
    assert_eq!(h.text("Viewer extensions"), "rs; py; txt");
    h.type_text("Viewer extensions", &refused);
    h.activate("Reset viewer extensions", AccessibleRole::Button);
    assert_eq!(view.get_extension_error(), "");
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

const IMAGE_THUMBNAILER_ID: &str = "midnight-image";
const VIDEO_THUMBNAILER_ID: &str = "midnight-video";

fn thumbnailer_options() -> Harness {
    let mut h = viewer_options();
    let catalog = [
        (
            IMAGE_THUMBNAILER_ID,
            "Image thumbnailer",
            ["png", "jpg"],
            false,
        ),
        (
            VIDEO_THUMBNAILER_ID,
            "Video thumbnailer",
            ["mp4", "mov"],
            true,
        ),
    ]
    .into_iter()
    .map(|(id, name, extensions, video)| ThumbnailerPluginInfo {
        id: id.into(),
        name: name.into(),
        version: "1".into(),
        extensions: extensions.into_iter().map(String::from).collect(),
        video,
        location: format!("thumbnailers/{id}").into(),
        status: "Installed".into(),
    })
    .collect();
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(catalog)));
    h.refresh();
    h.activate("Thumbnailers", AccessibleRole::Button);
    h
}

#[test]
fn thumbnailer_associations_edit_and_reset_to_trusted_manifest_defaults() {
    let mut h = thumbnailer_options();
    assert_eq!(h.text("Thumbnailer extensions"), "png; jpg");
    h.type_text("Thumbnailer extensions", "png");
    h.activate("Video thumbnailer · 1", AccessibleRole::ListItem);
    assert_eq!(h.text("Thumbnailer extensions"), "mp4; mov");
    h.activate("Image thumbnailer · 1", AccessibleRole::ListItem);
    assert_eq!(h.text("Thumbnailer extensions"), "png");
    h.activate("Reset thumbnailer extensions", AccessibleRole::Button);
    assert_eq!(h.text("Thumbnailer extensions"), "png; jpg");

    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Err(
            "Thumbnailer discovery failed".into(),
        )));
    h.refresh();
    assert!(
        h.find(|item| {
            item.accessible_role() == AccessibleRole::TextInput
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|label| label.as_str() == "Thumbnailer extensions")
        })
        .is_none(),
        "the editor needs a trusted catalog selection"
    );
}

#[test]
fn cancel_restores_staged_thumbnailer_associations() {
    let mut h = thumbnailer_options();
    h.type_text("Thumbnailer extensions", "jpg");
    h.activate("Video thumbnailer · 1", AccessibleRole::ListItem);
    h.activate("Image thumbnailer · 1", AccessibleRole::ListItem);
    assert_eq!(h.text("Thumbnailer extensions"), "jpg");
    h.activate("Cancel", AccessibleRole::Button);
    assert!(!h.window.get_options_open());
    h.window.invoke_show_options();
    h.window.set_options_topic_index(5);
    h.refresh();
    h.activate("Thumbnailers", AccessibleRole::Button);
    assert_eq!(h.text("Thumbnailer extensions"), "png; jpg");
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
}

#[test]
fn rejected_thumbnailer_extensions_return_from_viewers_with_inline_feedback() {
    let mut h = thumbnailer_options();
    let invalid = "png; ../outside";
    h.type_text("Thumbnailer extensions", invalid);
    h.activate("Viewers", AccessibleRole::Button);
    assert!(is_checked(&h.item("Viewers", AccessibleRole::Button)));
    h.activate("OK", AccessibleRole::Button);
    assert!(h.window.get_options_open());
    assert_eq!(h.window.get_options_topic_index(), 5);
    assert!(is_checked(&h.item("Thumbnailers", AccessibleRole::Button)));
    assert_eq!(h.text("Thumbnailer extensions"), invalid);
    let message = h
        .window
        .global::<ViewerPluginsView>()
        .get_thumbnailer_extension_error();
    assert!(!message.is_empty());
    h.item(message.as_str(), AccessibleRole::Text);
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }))
    );
}

#[test]
fn an_explicit_empty_thumbnailer_list_saves_with_viewer_edits_in_one_commit() {
    let mut h = thumbnailer_options();
    h.type_text("Thumbnailer extensions", "");
    h.activate("Viewers", AccessibleRole::Button);
    h.type_text("Viewer extensions", "rs; py; txt");
    h.activate("OK", AccessibleRole::Button);
    let revision = {
        let commands = h.sent.borrow();
        let mut saves = commands.iter().filter_map(|command| match command {
            Command::SavePreferences {
                revision,
                preferences,
            } => Some((*revision, preferences)),
            _ => None,
        });
        let (revision, preferences) = saves.next().expect("one Options commit");
        assert!(saves.next().is_none(), "one Options commit");
        let image_defaults = ["png".into(), "jpg".into()];
        let video_defaults = ["mp4".into(), "mov".into()];
        assert!(
            preferences
                .thumbnailer_associations
                .effective_extensions(IMAGE_THUMBNAILER_ID, &image_defaults)
                .is_empty(),
            "an explicit empty list disables this provider's extension matches"
        );
        assert_eq!(
            preferences
                .thumbnailer_associations
                .effective_extensions(VIDEO_THUMBNAILER_ID, &video_defaults),
            &video_defaults
        );
        assert_eq!(
            preferences
                .viewer_associations
                .overrides
                .get("midnight.viewer.source"),
            Some(&vec!["rs".into(), "py".into(), "txt".into()])
        );
        revision
    };
    h.presenter.borrow_mut().apply(Event::PreferencesSaved {
        revision,
        result: Ok(Default::default()),
    });
    h.refresh();
    assert!(!h.window.get_options_open(), "the commit was acknowledged");
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
