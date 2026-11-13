//! A capability-backed system shortcut opens the desktop shell, preserving panel authority.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    graphics::ImageInner,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use midnight_core::*;
use midnight_ui::{LocationRole, Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{PointerEventButton, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

const TOOLTIP: &str = "Open system Recycle Bin/Trash";
const LABEL: &str = "System Trash 文档";

struct Harness {
    ui: ui::TestUi,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    root: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let state = Rc::new(RefCell::new(Presenter::new(
            root.join("left"),
            root.join("right"),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        install(
            &ui.window,
            state.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        for pane in [PaneId::Left, PaneId::Right] {
            let navigation = commands
                .borrow()
                .iter()
                .find_map(|command| match command {
                    Command::Navigate {
                        pane: target,
                        navigation,
                        ..
                    } if *target == pane => Some(*navigation),
                    _ => None,
                })
                .unwrap();
            let entries: Arc<[Entry]> = vec![Entry::new(
                EntryId(1),
                root.join(if pane == PaneId::Left {
                    "left"
                } else {
                    "right"
                })
                .join("kept.txt"),
                EntryKind::File,
            )]
            .into();
            state.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            });
        }
        let mut h = Self {
            ui,
            state,
            commands,
            root,
        };
        h.context(Some(2));
        h.commands.borrow_mut().clear();
        h
    }

    fn context(&mut self, position: Option<usize>) {
        let native = |id: &str, kind| NavigationLocation {
            id: id.into(),
            label: id.into(),
            path: self.root.join(id),
            kind,
            drive_letter: None,
        };
        self.state.borrow_mut().apply(Event::DesktopContext {
            shell: ShellFlavor::Posix,
            network_position: 1,
            system_locations: position.map_or_else(Vec::new, |special_position| {
                vec![SystemLocationChoice {
                    location: SystemLocation::RecycleBin,
                    label: LABEL.into(),
                    special_position,
                }]
            }),
            locations: vec![
                native("disk", LocationKind::Volume),
                native("home", LocationKind::Home),
                native("documents", LocationKind::Documents),
                native("personal", LocationKind::OneDrive),
                native("business", LocationKind::OneDrive),
                native("last", LocationKind::Temporary),
            ],
        });
        refresh(&self.ui.window, &self.state.borrow());
        self.ui.draw();
    }

    fn assert_shell_request(&mut self) {
        let request = match self.commands.borrow().as_slice() {
            [
                Command::OpenSystemLocation {
                    request,
                    location: SystemLocation::RecycleBin,
                },
            ] => *request,
            commands => {
                panic!("system shortcut must issue exactly one typed shell action: {commands:?}")
            }
        };
        self.state.borrow_mut().apply(Event::DesktopActionFinished {
            request,
            result: Ok(DesktopActionResult::SystemLocationOpened {
                location: SystemLocation::RecycleBin,
            }),
        });
        refresh(&self.ui.window, &self.state.borrow());
        self.ui.draw();
        self.commands.borrow_mut().clear();
    }
}

fn artwork(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = artwork(item) {
            return Some(found);
        }
    }
    None
}

#[test]
fn recycle_bin_menu_and_toolbar_send_only_the_shell_command_and_preserve_both_panels() {
    let mut h = Harness::new();
    // Image(None) is not reflexively equal in pinned Slint. Use one private
    // nonempty marker only in detached comparison rows, preserving all fields
    // and leaving every nonempty icon and the actual UI models untouched.
    let empty_icon_marker = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(1, 1));
    let comparison_rows = |rows: slint::ModelRc<midnight_ui::FileRow>| {
        rows.iter()
            .map(|mut row| {
                if matches!(<&ImageInner>::from(&row.icon), ImageInner::None) {
                    row.icon = empty_icon_marker.clone();
                }
                row
            })
            .collect::<Vec<_>>()
    };
    let before_paths = [PaneId::Left, PaneId::Right].map(|pane| {
        let visible = h.state.borrow().visible_directory(pane).unwrap();
        (visible.path, visible.navigation)
    });
    let before_rows = (
        comparison_rows(h.ui.window.get_left_rows()),
        comparison_rows(h.ui.window.get_right_rows()),
    );
    let before_cursor = (
        h.ui.window.get_left_cursor(),
        h.ui.window.get_right_cursor(),
    );
    for rows in [
        h.ui.window.get_left_locations(),
        h.ui.window.get_right_locations(),
    ] {
        let rows: Vec<_> = rows.iter().collect();
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == LocationRole::RecycleBin)
                .count(),
            1
        );
        assert_eq!(rows[4].kind, LocationRole::RecycleBin);
        assert!(rows.last().unwrap().label.starts_with("last"));
    }
    for pane in [0, 1] {
        h.ui.window.invoke_open_volume_menu(pane);
        ui::click_popup(&h.ui.window, LABEL, AccessibleRole::Button);
        h.assert_shell_request();
    }
    ui::click_root(&h.ui.window, TOOLTIP, AccessibleRole::Button);
    h.assert_shell_request();
    let position = ui::root_position(&h.ui.window, TOOLTIP, AccessibleRole::Button);
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Right,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Right,
        },
    ] {
        h.ui.window.window().dispatch_event(event);
    }
    assert!(
        h.commands.borrow().is_empty(),
        "the system row has no filesystem context menu"
    );
    for (index, pane) in [PaneId::Left, PaneId::Right].into_iter().enumerate() {
        let visible = h.state.borrow().visible_directory(pane).unwrap();
        assert_eq!((visible.path, visible.navigation), before_paths[index]);
    }
    assert_eq!(comparison_rows(h.ui.window.get_left_rows()), before_rows.0);
    assert_eq!(comparison_rows(h.ui.window.get_right_rows()), before_rows.1);
    assert_eq!(
        (
            h.ui.window.get_left_cursor(),
            h.ui.window.get_right_cursor()
        ),
        before_cursor
    );
}

#[test]
fn recycle_bin_capability_controls_visibility_stale_tokens_and_native_theme_artwork() {
    let mut h = Harness::new();
    for native in [false, true] {
        if native {
            h.state.borrow_mut().apply(Event::IconLoaded {
                key: IconKey::action(ActionIcon::RecycleBin),
                pixels: Arc::new(IconPixels::new(3, 3, vec![255; 36]).unwrap()),
            });
            refresh(&h.ui.window, &h.state.borrow());
        }
        for theme in [0, 2] {
            h.ui.window.invoke_apply_preferences(theme, 0);
            h.ui.draw();
            let image = artwork(ui::root_item(&h.ui.window, TOOLTIP, AccessibleRole::Button))
                .expect("system shortcut needs visible semantic artwork");
            let image = image.downcast::<ImageItem>().unwrap();
            assert_eq!(
                image.as_pin_ref().source().size().width,
                if native { 3 } else { 24 }
            );
        }
    }
    let token =
        h.ui.window
            .get_navigation_choices()
            .iter()
            .find(|row| row.kind == LocationRole::RecycleBin)
            .unwrap()
            .identity;
    h.context(None);
    assert!(
        !h.ui
            .window
            .get_navigation_choices()
            .iter()
            .any(|row| row.kind == LocationRole::RecycleBin)
    );
    assert!(
        !h.ui
            .window
            .get_left_locations()
            .iter()
            .any(|row| row.kind == LocationRole::RecycleBin)
    );
    h.ui.window.invoke_navigation_choice(0, token.clone());
    h.context(Some(0));
    h.ui.window.invoke_navigation_choice(1, token);
    assert!(
        h.commands.borrow().is_empty(),
        "a capability return must not revive stale consent"
    );
}
