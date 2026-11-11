//! Installed native decision host, with real versioned copy commands and no filesystem work.
#[allow(dead_code)]
#[path = "job_windows.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    Model,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub struct Harness {
    pub ui: ui::TestUi,
    pub commands: Rc<RefCell<Vec<Command>>>,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    next_drop: u64,
}
impl Harness {
    pub fn new() -> Self {
        let ui = ui::TestUi::new();
        let path = std::env::temp_dir().join("native conflict fixture 文档");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        });
        install(&ui.window, state.clone(), send.clone());
        Self {
            ui,
            commands,
            state,
            send,
            next_drop: 1000,
        }
    }
    fn apply(&self, event: Event) {
        self.state
            .borrow_mut()
            .apply_with_sender(event, self.send.as_ref());
        self.refresh();
    }
    pub fn refresh(&self) {
        refresh(&self.ui.window, &self.state.borrow());
        self.ui.draw();
    }
    pub fn open(&mut self, version: u64, replace: bool, merge: bool) -> (OperationId, u64) {
        self.next_drop += 1;
        let destination = std::env::temp_dir()
            .join("conflict destination")
            .join("original.txt");
        self.apply(Event::DragDrop(DragDropEvent::DropAccepted {
            drag: None,
            token: DropToken(self.next_drop),
            pane: PaneId::Right,
            item_count: 1,
            intent: TransferIntent::Copy,
            verification: None,
            destination: Arc::new(destination.parent().unwrap().into()),
        }));
        // Drop tokens and operation IDs have independent sequences.
        let id = OperationId(
            self.ui
                .window
                .get_operations()
                .iter()
                .filter_map(|row| row.identity.strip_prefix("copy-job:")?.parse::<u64>().ok())
                .max()
                .expect("accepted drop must publish a copy job"),
        );
        let kind = if merge {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        self.apply(Event::Copy(CopyEvent::Conflict(Arc::new(CopyConflict {
            id,
            version,
            source: MutationItem {
                path: std::env::temp_dir()
                    .join("conflict source")
                    .join("original.txt"),
                kind,
                metadata: EntryMetadata::default(),
            },
            target: MutationItem {
                path: destination.clone(),
                kind,
                metadata: EntryMetadata::default(),
            },
            destination,
            can_replace: replace,
            can_merge: merge,
        }))));
        self.ui.resize(1040, 760);
        assert_eq!(
            self.ui.window.get_copy_token(),
            format!("copy:{}:{version}", id.0)
        );
        assert_eq!(
            self.ui.visible_count(),
            self.ui
                .window
                .get_operations()
                .iter()
                .filter(|row| row.active)
                .count()
                + 1,
            "Each active operation has its own progress window, plus this decision"
        );
        assert!(!self.ui.window.get_dialog_input_blocked());
        assert!(
            !crate::ui::has_popup(&self.ui.window),
            "the decision is not an owner popup"
        );
        self.ui.item("Decide later", AccessibleRole::Button);
        self.commands.borrow_mut().clear();
        (id, version)
    }
    pub fn type_name(&self, value: &str) {
        let child = self.ui.child();
        let item = self
            .ui
            .item("Copy under a different name", AccessibleRole::TextInput);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
        assert!(item.is_visible() && position.x >= 0.0 && position.y >= 0.0);
        assert!(
            position.x < child.window().size().width as f32
                && position.y < child.window().size().height as f32
        );
        for event in [
            WindowEvent::PointerMoved { position },
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
            WindowEvent::KeyPressed {
                text: Key::Control.into(),
            },
        ] {
            child.window().dispatch_event(event);
        }
        self.ui.key("a");
        child.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        self.ui.key(value);
    }
    pub fn decision(&self, id: OperationId, version: u64) -> CopyChoice {
        let commands = self.commands.borrow();
        let decisions: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                Command::DecideCopy(decision) => Some(decision),
                _ => None,
            })
            .collect();
        assert_eq!(decisions.len(), 1, "one exact decision must be admitted");
        assert_eq!((decisions[0].id, decisions[0].version), (id, version));
        decisions[0].choice.clone()
    }
}
