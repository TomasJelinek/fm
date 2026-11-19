use crate::{DirectoryLinkTooltip, MainWindow, Presenter};
use midnight_core::{Command, PaneId};
use slint::{ComponentHandle, Timer};
use std::{cell::RefCell, rc::Rc, time::Duration};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let visible = presenter.directory_link_tooltip().filter(|(pane, row, _)| {
        let hovered = match pane {
            PaneId::Left => window.get_left_hovered_row(),
            PaneId::Right => window.get_right_hovered_row(),
        };
        hovered.as_str() == *row
    });
    window.set_directory_link_tooltip(visible.map_or_else(
        DirectoryLinkTooltip::default,
        |(pane, row, text)| DirectoryLinkTooltip {
            pane: pane.index() as i32,
            row: row.into(),
            text: crate::shown_text(text),
        },
    ));
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    window.on_read_directory_link_target(move |side, identity| {
        let weak = weak.clone();
        let state = state.clone();
        let send = send.clone();
        // A tooltip can be constructed while Slint is synchronizing its items.
        // Submit after that callback unwinds, and check the live pointer again.
        Timer::single_shot(Duration::ZERO, move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let (pane, hovered) = match side {
                0 => (PaneId::Left, window.get_left_hovered_row()),
                1 => (PaneId::Right, window.get_right_hovered_row()),
                _ => return,
            };
            if identity.is_empty()
                || identity != hovered
                || window.get_dialog_input_blocked()
                || window.get_scroll_interaction_active()
            {
                return;
            }
            let Ok(mut presenter) = state.try_borrow_mut() else {
                return;
            };
            presenter.read_directory_link_target(pane, identity.as_str(), send.as_ref());
            refresh(&window, &presenter);
        });
    });
    let weak = window.as_weak();
    window.on_directory_link_hover_changed(move |side, identity| {
        let pane = match side {
            0 => PaneId::Left,
            1 => PaneId::Right,
            _ => return,
        };
        let Ok(mut presenter) = presenter.try_borrow_mut() else {
            return;
        };
        let cleared = presenter.directory_link_hover_changed(pane, identity.as_str());
        drop(presenter);
        if cleared && let Some(window) = weak.upgrade() {
            window.set_directory_link_tooltip(DirectoryLinkTooltip::default());
        }
    });
}
