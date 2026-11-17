#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    items::{AccessibleRole, TextCursorAffinity, TextInput},
    window::{WindowInner, WindowKind},
};
use midnight_core::{
    ArchiveFormat, Command, Event, FilesystemPluginInfo, FilesystemPluginKind,
    ThumbnailerPluginInfo, UserPreferences, ViewerPluginInfo,
};
use midnight_ui::{MainWindow, Presenter, ViewerPluginRow, ViewerPluginsView, install, refresh};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    platform::{Key, WindowAdapter},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use ui::{TestUi, click_popup};

#[test]
fn plugin_controls_toggle_only_available_packages_and_request_refresh() {
    let mut ui = TestUi::new();
    let toggles = Rc::new(RefCell::new(Vec::new()));
    let captured = toggles.clone();
    let view = ui.window.global::<ViewerPluginsView>();
    view.on_toggle(move |id, enabled| captured.borrow_mut().push((id.to_string(), enabled)));
    let refreshes = Rc::new(Cell::new(0));
    let captured = refreshes.clone();
    view.on_refresh(move || captured.set(captured.get() + 1));
    view.set_settings_ready(true);
    view.set_rows(ModelRc::new(VecModel::from(vec![
        ViewerPluginRow {
            id: "midnight.viewer.source".into(),
            name: "Source viewer".into(),
            version: "1.0".into(),
            status: "Available · not loaded".into(),
            enabled: true,
            configurable: true,
            ..Default::default()
        },
        ViewerPluginRow {
            name: "Broken package".into(),
            status: "Unavailable: incompatible ABI".into(),
            ..Default::default()
        },
    ])));
    ui.window.invoke_show_options();
    ui.window.set_options_topic_index(5);
    slint::platform::update_timers_and_animations();
    ui.draw();
    click_popup(&ui.window, "Source viewer · 1.0", AccessibleRole::Checkbox);
    assert_eq!(
        *toggles.borrow(),
        [("midnight.viewer.source".to_owned(), false)]
    );
    click_popup(&ui.window, "Broken package", AccessibleRole::Checkbox);
    assert_eq!(
        toggles.borrow().len(),
        1,
        "invalid packages must not be configurable"
    );
    click_popup(&ui.window, "Rescan all plugins", AccessibleRole::Button);
    assert_eq!(refreshes.get(), 1);
}

struct Harness {
    ui: TestUi,
    sent: Rc<RefCell<Vec<Command>>>,
    presenter: Rc<RefCell<Presenter>>,
}

#[test]
fn plugin_viewer_list_fits_its_catalog_with_heading_arrows_and_bottom_refresh() {
    let h = Harness::new();
    let list = ui::popup_item(&h.ui.window, "Viewer priority", AccessibleRole::List);
    let row = ui::popup_item(&h.ui.window, "Source viewer · 1", AccessibleRole::ListItem);
    let arrow = ui::popup_item(&h.ui.window, "Move viewer up", AccessibleRole::Button);
    let refresh = ui::popup_item(&h.ui.window, "Rescan all plugins", AccessibleRole::Button);
    assert!(
        list.geometry().size.height >= row.geometry().size.height * 3.0 - 1.0
            && list.geometry().size.height <= row.geometry().size.height * 3.0 + 4.0,
        "three installed viewers should occupy three rows without empty catalog space"
    );
    let list_top = list.map_to_native_window(list.geometry().origin).y;
    let arrow_top = arrow.map_to_native_window(arrow.geometry().origin).y;
    let refresh_top = refresh.map_to_native_window(refresh.geometry().origin).y;
    assert!(
        arrow_top < list_top,
        "priority arrows belong beside the heading"
    );
    assert!(
        refresh_top > list_top + list.geometry().size.height,
        "Refresh belongs below the catalog"
    );
}

#[test]
fn plugins_exposes_a_separate_read_only_filesystems_overview() {
    let mut h = Harness::new();
    let base = std::env::current_dir().unwrap();
    let filesystems = [
        (
            "native.fixture",
            "Native filesystem",
            FilesystemPluginKind::Native,
        ),
        (
            "archive.fixture",
            "Archive filesystem",
            FilesystemPluginKind::Archive,
        ),
        (
            "network.fixture",
            "Network filesystem",
            FilesystemPluginKind::Network,
        ),
    ]
    .into_iter()
    .map(|(id, name, kind)| FilesystemPluginInfo {
        id: id.into(),
        name: name.into(),
        version: "1".into(),
        kind: Some(kind),
        location: Some(base.join("filesystems").join(id)),
        status: "Installed".into(),
        formats: if kind == FilesystemPluginKind::Archive {
            vec![ArchiveFormat::Zip]
        } else {
            Vec::new()
        },
    })
    .collect();
    h.presenter
        .borrow_mut()
        .apply(Event::FilesystemPluginsListed(Ok(filesystems)));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    render::snapshot(&h.ui, "plugin-viewers");
    h.sent.borrow_mut().clear();
    h.click("Filesystems", AccessibleRole::Button);
    let list = ui::popup_item(&h.ui.window, "Filesystem plugins", AccessibleRole::List);
    assert!(
        !has_role(list, AccessibleRole::Checkbox),
        "filesystem rows are read-only"
    );
    h.click("Network filesystem · 1", AccessibleRole::ListItem);
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_filesystem_selected(),
        1
    );
    assert!(
        h.sent.borrow().is_empty(),
        "reading a catalog cannot load providers or save preferences"
    );
    render::snapshot(&h.ui, "plugin-filesystems");
    h.click("Viewers", AccessibleRole::Button);
    ui::popup_item(&h.ui.window, "Viewer priority", AccessibleRole::List);
}

fn has_role(item: i_slint_core::item_tree::ItemRc, role: AccessibleRole) -> bool {
    if item.accessible_role() == role {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if has_role(item, role) {
            return true;
        }
    }
    false
}

impl Harness {
    fn new() -> Self {
        let ui = TestUi::new();
        let presenter = Rc::new(RefCell::new(Presenter::new("left".into(), "right".into())));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let captured = sent.clone();
        install(
            &ui.window,
            presenter.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        let catalog = [
            ("source", "Source viewer", vec!["rs", "py"]),
            ("image", "Image viewer", vec!["png", "jpg"]),
            ("pdf", "PDF viewer", vec!["pdf"]),
        ]
        .into_iter()
        .map(|(id, name, extensions)| ViewerPluginInfo {
            id: format!("midnight.viewer.{id}"),
            name: name.into(),
            description: if id == "source" {
                "Syntax highlighting for Rust, C, C++, JavaScript, TypeScript, Python, POSIX shell/Bash/Zsh, Windows Batch/CMD, JSON, TOML, Java, C#, Go, Swift, Kotlin, Ruby, SQL and YAML (18 lexical profiles).".into()
            } else { String::new() },
            version: "1".into(),
            status: "Available".into(),
            location: "viewers".into(),
            fallback: false,
            extensions: extensions.into_iter().map(String::from).collect(),
        })
        .collect::<Vec<_>>();
        presenter
            .borrow_mut()
            .apply(Event::ViewerPluginsListed(Ok(catalog.clone())));
        refresh(&ui.window, &presenter.borrow());
        ui.window.invoke_show_options();
        ui.window.set_options_topic_index(5);
        // Complete the refresh requested when opening Options; the test harness
        // records commands rather than running a real discovery service.
        presenter
            .borrow_mut()
            .apply(Event::ViewerPluginsListed(Ok(catalog)));
        presenter
            .borrow_mut()
            .apply(Event::FilesystemPluginsListed(Ok(Vec::new())));
        presenter
            .borrow_mut()
            .apply(Event::ThumbnailerPluginsListed(Ok(Vec::new())));
        refresh(&ui.window, &presenter.borrow());
        let mut harness = Self {
            ui,
            sent,
            presenter,
        };
        harness.settle();
        harness
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.ui.draw();
        }
    }

    fn click(&mut self, label: &str, role: AccessibleRole) {
        click_popup(&self.ui.window, label, role);
        self.settle();
    }

    fn edit(&mut self, text: &str) {
        ui::popup_item(
            &self.ui.window,
            "Viewer extensions",
            AccessibleRole::TextInput,
        )
        .accessible_action(&AccessibilityAction::SetValue(text.into()));
        self.settle();
    }

    fn saves(&self) -> Vec<UserPreferences> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::SavePreferences { preferences, .. } => Some(*preferences.clone()),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn viewer_priority_keyboard_extensions_and_reset_are_cancelled_as_one_transaction() {
    let mut h = Harness::new();
    render::snapshot(&h.ui, "viewer-associations");
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "rs; py"
    );
    h.edit(".RS; *.rs; rust");
    h.click("Image viewer · 1", AccessibleRole::ListItem);
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "png; jpg"
    );
    ui::key(&h.ui.window, Key::DownArrow);
    h.settle();
    assert_eq!(h.ui.window.global::<ViewerPluginsView>().get_selected(), 2);
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "pdf"
    );
    h.click("Move viewer up", AccessibleRole::Button);
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_rows()
            .row_data(1)
            .unwrap()
            .name,
        "PDF viewer"
    );
    h.edit("pdf; book");
    h.click("Reset viewer extensions", AccessibleRole::Button);
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "pdf"
    );
    assert!(h.saves().is_empty(), "all edits are Options previews");
    h.click("Cancel", AccessibleRole::Button);
    assert!(h.saves().is_empty());
    h.ui.window.invoke_show_options();
    h.ui.window.set_options_topic_index(5);
    h.settle();
    h.click("Source viewer · 1", AccessibleRole::ListItem);
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "rs; py"
    );
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_rows()
            .row_data(1)
            .unwrap()
            .name,
        "Image viewer"
    );
}

#[test]
fn long_viewer_extensions_scroll_horizontally_and_keep_the_caret_fully_visible() {
    let mut h = Harness::new();
    let extensions = "rs; c; h; cpp; cc; cxx; hpp; hh; hxx; js; mjs; cjs; jsx; ts; tsx; mts; cts; py; pyw; sh; bash; zsh; json; toml; java; cs; go; swift; kt; kts; rb; sql; yaml; yml; 日本語";
    for (density, font, width, height) in
        [(0, 14, 1100, 800), (0, 22, 760, 650), (1, 28, 1050, 900)]
    {
        h.edit(extensions);
        h.ui.window.invoke_apply_preferences(1, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(width, height);
        h.click("Viewer extensions", AccessibleRole::TextInput);
        let input = WindowInner::from_pub(h.ui.window.window())
            .focus_item
            .borrow()
            .upgrade()
            .expect("extension input must receive focus");
        let field = ui::popup_item(&h.ui.window, "Viewer extensions", AccessibleRole::TextInput);
        let mut origins = Vec::new();
        for (key, offset) in [(Key::Home, 0), (Key::End, extensions.len()), (Key::Home, 0)] {
            ui::key(&h.ui.window, key);
            h.settle();
            let text = input
                .downcast::<TextInput>()
                .expect("focused extension text");
            let text = text.as_pin_ref();
            assert_eq!(text.text().as_str(), extensions);
            assert_eq!(text.cursor_position_byte_offset() as usize, offset);
            let caret =
                h.ui.native
                    .renderer()
                    .text_input_cursor_rect_for_byte_offset(
                        text,
                        &input,
                        offset,
                        TextCursorAffinity::NextCharacter,
                    );
            let origin =
                input.map_to_native_window(input.geometry().origin + caret.origin.to_vector());
            let bounds = field.geometry();
            let field_origin = field.map_to_native_window(bounds.origin);
            assert!(
                origin.x >= field_origin.x
                    && origin.x + caret.width() <= field_origin.x + bounds.width(),
                "horizontal caret is clipped: density={density}, font={font}"
            );
            assert!(
                origin.y >= field_origin.y
                    && origin.y + caret.height() <= field_origin.y + bounds.height(),
                "vertical caret is clipped: density={density}, font={font}"
            );
            origins.push(input.map_to_native_window(input.geometry().origin).x);
        }
        assert!(
            origins[1] < origins[0] - 20.0,
            "End must reveal the extension tail by scrolling horizontally"
        );
        assert!(
            (origins[2] - origins[0]).abs() < 1.0,
            "Home must restore the leading extensions"
        );
        render::snapshot(
            &h.ui,
            &format!("viewer-extension-scroll-{density}-{font}-{width}"),
        );
    }
}

#[test]
fn invalid_extension_draft_blocks_ok_and_corrected_multiline_text_saves_once() {
    let mut h = Harness::new();
    h.edit("rs; ../bad");
    h.click("OK", AccessibleRole::Button);
    assert!(h.saves().is_empty());
    assert!(ui::has_popup(&h.ui.window));
    assert_eq!(h.ui.window.get_options_topic_index(), 5);
    assert!(
        !h.ui
            .window
            .global::<ViewerPluginsView>()
            .get_error()
            .is_empty()
    );
    render::snapshot(&h.ui, "viewer-associations-invalid");
    h.edit(".RS; *.rs\n rust, 日本語");
    h.click("OK", AccessibleRole::Button);
    let saves = h.saves();
    assert_eq!(saves.len(), 1);
    assert_eq!(
        saves[0]
            .viewer_associations
            .overrides
            .get("midnight.viewer.source"),
        Some(&vec!["rs".into(), "rust".into(), "日本語".into()])
    );
}

#[test]
fn plugins_exposes_thumbnailers_tab_without_viewer_controls() {
    let mut h = Harness::new();
    h.click("Thumbnailers", AccessibleRole::Button);
    let list = ui::popup_item(&h.ui.window, "Thumbnailer plugins", AccessibleRole::List);
    assert!(
        !has_role(list, AccessibleRole::Checkbox),
        "thumbnailer discovery must not offer nonfunctional viewer switches"
    );
    assert!(h.saves().is_empty());
}

fn thumbnailers() -> Vec<ThumbnailerPluginInfo> {
    [("image", false, "png"), ("video", true, "webm")]
        .into_iter()
        .map(|(kind, video, extension)| ThumbnailerPluginInfo {
            id: format!("midnight-{kind}"),
            name: if video {
                "Video thumbnailer"
            } else {
                "Image thumbnailer"
            }
            .into(),
            version: "1".into(),
            extensions: vec![extension.into()],
            video,
            location: std::path::PathBuf::from("thumbnailers").join(kind),
            status: "Installed".into(),
        })
        .collect()
}

#[test]
fn thumbnailer_catalog_shows_details_and_retains_selection_by_identity() {
    let mut h = Harness::new();
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(thumbnailers())));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    h.click("Thumbnailers", AccessibleRole::Button);
    h.sent.borrow_mut().clear();
    h.click("Video thumbnailer · 1", AccessibleRole::ListItem);
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_thumbnailer_selected(), 1);
    let video = view.get_thumbnailer_rows().row_data(1).unwrap();
    assert_eq!(video.id, "midnight-video");
    assert_eq!(video.status, "Installed");
    assert_eq!(video.description, "Extensions: webm");
    assert!(video.kind.contains("animated"));
    assert!(video.location.ends_with("video"));
    assert!(!video.configurable);
    assert!(
        h.sent.borrow().is_empty(),
        "selection must not start a worker or save settings"
    );
    render::snapshot(&h.ui, "plugin-thumbnailers");

    let mut reordered = thumbnailers();
    reordered.reverse();
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(reordered)));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_thumbnailer_selected(),
        0
    );
}

#[test]
fn thumbnailer_discovery_failure_is_visible_and_refresh_waits_for_all_catalogs() {
    let mut h = Harness::new();
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(thumbnailers())));
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Err(
            "Thumbnail helper is missing: thumbnailers/video".into(),
        )));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    h.click("Thumbnailers", AccessibleRole::Button);
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_thumbnailer_rows().row_count(), 0);
    assert_eq!(view.get_thumbnailer_selected(), -1);
    assert!(
        view.get_thumbnailer_error()
            .contains("Thumbnail helper is missing")
    );
    h.sent.borrow_mut().clear();
    h.click("Rescan all plugins", AccessibleRole::Button);
    h.ui.window.global::<ViewerPluginsView>().invoke_refresh();
    assert_eq!(
        h.sent
            .borrow()
            .iter()
            .filter(|c| matches!(c, Command::ListViewerPlugins))
            .count(),
        1
    );
    h.presenter
        .borrow_mut()
        .apply(Event::ViewerPluginsListed(Ok(Vec::new())));
    h.presenter
        .borrow_mut()
        .apply(Event::FilesystemPluginsListed(Ok(Vec::new())));
    refresh(&h.ui.window, &h.presenter.borrow());
    assert!(h.ui.window.global::<ViewerPluginsView>().get_pending());
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(thumbnailers())));
    refresh(&h.ui.window, &h.presenter.borrow());
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert!(!view.get_pending());
    assert!(view.get_thumbnailer_error().is_empty());
    assert_eq!(view.get_thumbnailer_rows().row_count(), 2);
}

#[test]
fn all_six_installed_viewers_fit_without_catalog_scrolling_at_normal_sizes() {
    let mut h = Harness::new();
    let names = [
        "Text and binary viewer",
        "Source viewer",
        "Markdown viewer",
        "Image viewer",
        "PDF viewer",
        "HTML viewer",
    ];
    h.ui.window
        .global::<ViewerPluginsView>()
        .set_rows(ModelRc::new(VecModel::from(
            names
                .iter()
                .map(|name| ViewerPluginRow {
                    name: (*name).into(),
                    version: "1".into(),
                    enabled: true,
                    configurable: true,
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
        )));
    for (density, font) in [(0, 12), (0, 18), (1, 15)] {
        h.ui.window.invoke_apply_preferences(1, density);
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(1340, 900);
        h.settle();
        let list = ui::popup_item(&h.ui.window, "Viewer priority", AccessibleRole::List);
        let refresh = ui::popup_item(&h.ui.window, "Rescan all plugins", AccessibleRole::Button);
        let top = list.map_to_native_window(list.geometry().origin).y;
        let bottom = top + list.geometry().size.height;
        let footer_top = refresh.map_to_native_window(refresh.geometry().origin).y;
        for name in names {
            let row = ui::popup_item(
                &h.ui.window,
                &format!("{name} · 1"),
                AccessibleRole::ListItem,
            );
            let row_top = row.map_to_native_window(row.geometry().origin).y;
            let row_bottom = row_top + row.geometry().size.height;
            assert!(
                row_top >= top && row_bottom <= bottom,
                "{name} must fit in the catalog, density={density}, font={font}"
            );
            assert!(
                row_bottom < footer_top,
                "{name} must remain above the pinned rescan action"
            );
        }
        render::snapshot(&h.ui, &format!("six-viewers-{density}-{font}"));
    }
}

fn package(
    id: &str,
    name: &str,
    kind: FilesystemPluginKind,
    formats: Vec<ArchiveFormat>,
) -> FilesystemPluginInfo {
    FilesystemPluginInfo {
        id: id.into(),
        name: name.into(),
        version: "2.3".into(),
        kind: Some(kind),
        formats,
        location: Some(
            std::env::current_dir()
                .unwrap()
                .join("filesystem packages/資料")
                .join(id),
        ),
        status: "Installed · artifact validated".into(),
    }
}

fn package_diagnostics() -> [FilesystemPluginInfo; 3] {
    [
        (
            "malformed.zip",
            "Malformed archive manifest: expected an object",
        ),
        (
            "missing-worker",
            "Missing worker artifact: archive-worker.exe",
        ),
        (
            "duplicate-id",
            "Duplicate filesystem package ID: archive.first",
        ),
    ]
    .map(|(directory, status)| FilesystemPluginInfo {
        id: String::new(),
        name: "Unavailable package".into(),
        version: String::new(),
        kind: None,
        formats: Vec::new(),
        location: Some(
            std::env::current_dir()
                .unwrap()
                .join("filesystem packages/invalid")
                .join(directory),
        ),
        status: status.into(),
    })
}

fn filesystem_catalog() -> Vec<FilesystemPluginInfo> {
    let [malformed, missing, duplicate] = package_diagnostics();
    vec![
        package(
            "native.fixture",
            "Native filesystem",
            FilesystemPluginKind::Native,
            Vec::new(),
        ),
        package(
            "archive.first",
            "Archive first",
            FilesystemPluginKind::Archive,
            vec![ArchiveFormat::Zip, ArchiveFormat::Tar],
        ),
        malformed,
        package(
            "network.fixture",
            "Network filesystem",
            FilesystemPluginKind::Network,
            Vec::new(),
        ),
        package(
            "archive.second",
            "Archive second",
            FilesystemPluginKind::Archive,
            vec![ArchiveFormat::SevenZip],
        ),
        missing,
        package(
            "archive.empty",
            "Archive without declarations",
            FilesystemPluginKind::Archive,
            Vec::new(),
        ),
        duplicate,
    ]
}

fn popup_root(window: &MainWindow) -> ItemRc {
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("Options popup");
    ItemRc::new_root(popup.component.clone())
}

fn named_item(item: ItemRc, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = named_item(item, label, role) {
            return Some(found);
        }
    }
    None
}

fn plugin_tab_strip(item: ItemRc) -> Option<ItemRc> {
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = plugin_tab_strip(item) {
            return Some(found);
        }
    }
    ["Viewers", "Filesystems", "Archives", "Thumbnailers"]
        .iter()
        .all(|label| named_item(item.clone(), label, AccessibleRole::Button).is_some())
        .then_some(item)
}

impl Harness {
    fn filesystem_catalog(&mut self, catalog: Vec<FilesystemPluginInfo>) {
        self.presenter
            .borrow_mut()
            .apply(Event::FilesystemPluginsListed(Ok(catalog)));
        refresh(&self.ui.window, &self.presenter.borrow());
        self.settle();
    }

    fn plugin_tab(&mut self, label: &str) {
        let strip = plugin_tab_strip(popup_root(&self.ui.window)).expect("plugin tab strip");
        let tab = named_item(strip, label, AccessibleRole::Button).expect("plugin category tab");
        let bounds = tab.geometry();
        let origin = tab.map_to_native_window(bounds.origin);
        ui::click(
            &self.ui.window,
            origin.x + bounds.width() / 2.0,
            origin.y + bounds.height() / 2.0,
        );
        self.settle();
        assert_eq!(self.ui.window.get_options_topic_index(), 5);
    }

    fn popup_text(&self) -> String {
        ui::visible_text_values(popup_root(&self.ui.window))
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rescans(&self) -> usize {
        self.sent
            .borrow()
            .iter()
            .filter(|command| matches!(command, Command::ListViewerPlugins))
            .count()
    }
}

#[test]
fn archive_catalog_is_read_only_and_uses_each_packages_declared_formats() {
    let mut h = Harness::new();
    let catalog = filesystem_catalog();
    let first = catalog[1].clone();
    h.filesystem_catalog(catalog);
    h.sent.borrow_mut().clear();
    h.plugin_tab("Archives");
    let list = ui::popup_item(&h.ui.window, "Archive plugins", AccessibleRole::List);
    assert!(!has_role(list, AccessibleRole::Checkbox));
    assert!(ui::try_popup_item(&h.ui.window, "Move viewer up", AccessibleRole::Button).is_none());
    assert!(ui::try_popup_item(&h.ui.window, "Move viewer down", AccessibleRole::Button).is_none());
    assert!(
        ui::try_popup_item(&h.ui.window, "Viewer extensions", AccessibleRole::TextInput).is_none()
    );
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(
        view.get_filesystem_rows()
            .iter()
            .map(|row| row.id.to_string())
            .collect::<Vec<_>>(),
        ["native.fixture", "network.fixture"]
    );
    let archives = view.get_archive_rows();
    assert_eq!(
        archives
            .iter()
            .map(|row| row.id.to_string())
            .collect::<Vec<_>>(),
        ["archive.first", "archive.second", "archive.empty"]
    );
    let row = archives.row_data(0).unwrap();
    assert_eq!(row.name, first.name.as_str());
    assert_eq!(row.version, first.version.as_str());
    assert_eq!(row.status, first.status.as_str());
    assert_eq!(
        row.location,
        first.location.unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(row.description, "Declared formats: ZIP; TAR");
    assert!(!row.configurable);
    assert!(
        h.popup_text().contains("archive.first"),
        "package ID is visible"
    );
    h.click("Archive second · 2.3", AccessibleRole::ListItem);
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_archive_selected(), 1);
    assert_eq!(
        view.get_archive_rows().row_data(1).unwrap().description,
        "Declared formats: 7-Zip"
    );
    assert!(h.popup_text().contains("archive.second"));
    h.click(
        "Archive without declarations · 2.3",
        AccessibleRole::ListItem,
    );
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_archive_rows()
            .row_data(2)
            .unwrap()
            .description,
        "Declared formats: None declared"
    );
    assert!(
        h.sent.borrow().is_empty(),
        "reading package rows must not discover, start workers or save preferences"
    );
}

#[test]
fn unclassified_package_diagnostics_and_catalog_errors_are_visible_on_both_tabs() {
    let mut h = Harness::new();
    h.filesystem_catalog(filesystem_catalog());
    h.sent.borrow_mut().clear();
    for tab in ["Filesystems", "Archives"] {
        h.plugin_tab(tab);
        let diagnostics =
            h.ui.window
                .global::<ViewerPluginsView>()
                .get_filesystem_diagnostics();
        let text = h.popup_text();
        assert!(text.contains("Filesystem and archive package diagnostics"));
        for diagnostic in package_diagnostics() {
            let path = diagnostic.location.unwrap().to_string_lossy().into_owned();
            assert!(diagnostics.contains(&diagnostic.status));
            assert!(diagnostics.contains(&path));
            assert!(text.contains(&diagnostic.status));
            assert!(text.contains(&path));
        }
    }
    let error = "Filesystem and archive catalog unavailable: package metadata denied";
    h.presenter
        .borrow_mut()
        .apply(Event::FilesystemPluginsListed(Err(error.into())));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    for tab in ["Filesystems", "Archives"] {
        h.plugin_tab(tab);
        let view = h.ui.window.global::<ViewerPluginsView>();
        assert_eq!(view.get_filesystem_error(), error);
        assert_eq!(view.get_archive_rows().row_count(), 0);
        assert_eq!(view.get_archive_selected(), -1);
        assert!(h.popup_text().contains(error));
    }
    assert!(h.sent.borrow().is_empty());
}

#[test]
fn archive_and_filesystem_selections_survive_rescan_independently_after_reordering() {
    let mut h = Harness::new();
    h.filesystem_catalog(filesystem_catalog());
    h.plugin_tab("Filesystems");
    h.click("Network filesystem · 2.3", AccessibleRole::ListItem);
    h.plugin_tab("Archives");
    h.click("Archive first · 2.3", AccessibleRole::ListItem);
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_filesystem_selected(), 1);
    assert_eq!(view.get_archive_selected(), 0);
    h.sent.borrow_mut().clear();
    h.click("Rescan all plugins", AccessibleRole::Button);
    h.ui.window.global::<ViewerPluginsView>().invoke_refresh();
    assert_eq!(h.rescans(), 1);
    assert!(h.ui.window.global::<ViewerPluginsView>().get_pending());
    let mut reordered = filesystem_catalog();
    reordered.reverse();
    h.filesystem_catalog(reordered.clone());
    h.presenter
        .borrow_mut()
        .apply(Event::ViewerPluginsListed(Ok(Vec::new())));
    refresh(&h.ui.window, &h.presenter.borrow());
    assert!(h.ui.window.global::<ViewerPluginsView>().get_pending());
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(Vec::new())));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert!(!view.get_pending());
    assert_eq!(view.get_filesystem_selected(), 0);
    assert_eq!(view.get_archive_selected(), 2);
    assert_eq!(
        view.get_archive_rows().row_data(2).unwrap().id,
        "archive.first"
    );
    h.plugin_tab("Filesystems");
    assert_eq!(
        h.ui.window
            .global::<ViewerPluginsView>()
            .get_filesystem_selected(),
        0
    );
    reordered.retain(|plugin| plugin.id != "archive.first");
    h.filesystem_catalog(reordered);
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_filesystem_selected(), 0);
    assert_eq!(view.get_archive_selected(), 0);
    assert_eq!(
        view.get_archive_rows().row_data(0).unwrap().id,
        "archive.empty"
    );
    assert_eq!(
        h.sent.borrow().len(),
        1,
        "only the shared rescan sends a command"
    );
}

#[test]
fn archive_empty_state_waits_for_the_shared_loading_cycle() {
    let mut h = Harness::new();
    h.plugin_tab("Archives");
    let view = h.ui.window.global::<ViewerPluginsView>();
    assert_eq!(view.get_archive_rows().row_count(), 0);
    assert_eq!(view.get_archive_selected(), -1);
    assert!(h.popup_text().contains("No archive plugins"));
    h.sent.borrow_mut().clear();
    h.click("Rescan all plugins", AccessibleRole::Button);
    assert!(h.ui.window.global::<ViewerPluginsView>().get_pending());
    assert!(!h.popup_text().contains("No archive plugins"));
    ui::popup_item(&h.ui.window, "Rescanning…", AccessibleRole::Button);
    h.ui.window.global::<ViewerPluginsView>().invoke_refresh();
    assert_eq!(h.rescans(), 1);
    for event in [
        Event::ViewerPluginsListed(Ok(Vec::new())),
        Event::FilesystemPluginsListed(Ok(Vec::new())),
    ] {
        h.presenter.borrow_mut().apply(event);
    }
    refresh(&h.ui.window, &h.presenter.borrow());
    assert!(h.ui.window.global::<ViewerPluginsView>().get_pending());
    h.presenter
        .borrow_mut()
        .apply(Event::ThumbnailerPluginsListed(Ok(Vec::new())));
    refresh(&h.ui.window, &h.presenter.borrow());
    h.settle();
    assert!(!h.ui.window.global::<ViewerPluginsView>().get_pending());
    assert!(h.popup_text().contains("No archive plugins"));
}

#[test]
fn archive_associations_navigation_preserves_the_options_draft_without_sending_commands() {
    let mut h = Harness::new();
    h.filesystem_catalog(filesystem_catalog());
    h.edit("rs; draft-only");
    h.sent.borrow_mut().clear();
    h.plugin_tab("Archives");
    h.click("Archive associations…", AccessibleRole::Button);
    assert_eq!(h.ui.window.get_options_topic_index(), 7);
    ui::popup_item(
        &h.ui.window,
        "Search archive associations",
        AccessibleRole::TextInput,
    );
    h.ui.options_topic(5);
    h.settle();
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "rs; draft-only"
    );
    assert!(
        h.sent.borrow().is_empty(),
        "association navigation is local UI state"
    );
    h.click("Cancel", AccessibleRole::Button);
    assert!(h.saves().is_empty());
    h.ui.window.invoke_show_options();
    h.ui.window.set_options_topic_index(5);
    h.settle();
    assert_eq!(
        h.ui.window.global::<ViewerPluginsView>().get_extensions(),
        "rs; py"
    );
}

#[test]
fn archive_catalog_keyboard_selection_and_compact_category_picker_remain_accessible() {
    let mut h = Harness::new();
    h.filesystem_catalog(filesystem_catalog());
    h.plugin_tab("Archives");
    h.click("Archive first · 2.3", AccessibleRole::ListItem);
    h.sent.borrow_mut().clear();
    for (key, selected) in [(Key::DownArrow, 1), (Key::End, 2), (Key::Home, 0)] {
        ui::key(&h.ui.window, key);
        h.settle();
        assert_eq!(
            h.ui.window
                .global::<ViewerPluginsView>()
                .get_archive_selected(),
            selected
        );
    }
    for (width, height, font) in [(760, 650, 22), (420, 360, 28)] {
        h.ui.window.invoke_apply_fonts("".into(), font, font);
        h.ui.resize(width, height);
        h.settle();
        let picker = ui::popup_item(&h.ui.window, "Plugin type", AccessibleRole::Combobox);
        let bounds = picker.geometry();
        let origin = picker.map_to_native_window(bounds.origin);
        assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
        assert!(origin.x >= 0.0 && origin.y >= 0.0);
        assert!(origin.x + bounds.width() <= width as f32 + 1.0);
        assert!(origin.y + bounds.height() <= height as f32 + 1.0);
        let rescan = ui::popup_item(&h.ui.window, "Rescan all plugins", AccessibleRole::Button);
        let bounds = rescan.geometry();
        let origin = rescan.map_to_native_window(bounds.origin);
        assert!(origin.y >= 0.0 && origin.y + bounds.height() <= height as f32 + 1.0);
        h.click("Plugin type", AccessibleRole::Combobox);
        ui::key(&h.ui.window, Key::UpArrow);
        ui::key(&h.ui.window, Key::UpArrow);
        ui::key(&h.ui.window, Key::Return);
        h.settle();
        ui::popup_item(&h.ui.window, "Viewer priority", AccessibleRole::List);
        h.click("Plugin type", AccessibleRole::Combobox);
        ui::key(&h.ui.window, Key::DownArrow);
        ui::key(&h.ui.window, Key::DownArrow);
        ui::key(&h.ui.window, Key::Return);
        h.settle();
        ui::popup_item(&h.ui.window, "Archive plugins", AccessibleRole::List);
        render::snapshot(&h.ui, &format!("archive-plugin-picker-{width}-{font}"));
    }
    assert!(h.sent.borrow().is_empty());
}
