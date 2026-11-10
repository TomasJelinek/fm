#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
use midnight_core::{
    Command, Event, FilesystemPluginInfo, FilesystemPluginKind, UserPreferences, ViewerPluginInfo,
};
use midnight_ui::{Presenter, ViewerPluginRow, ViewerPluginsView, install, refresh};
use slint::{ComponentHandle, Model, ModelRc, VecModel, platform::Key};
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
    click_popup(&ui.window, "Refresh plugins", AccessibleRole::Button);
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
    let refresh = ui::popup_item(&h.ui.window, "Refresh plugins", AccessibleRole::Button);
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
        2
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
                "Syntax highlighting for Rust, C, C++, JavaScript, TypeScript, Python, POSIX shell/Bash/Zsh, JSON, TOML, Java, C#, Go, Swift, Kotlin, Ruby, SQL and YAML (17 lexical profiles).".into()
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
