//! Convert UI gestures into captured native-menu and file-clipboard intents.
use crate::{MainWindow, Presenter, pane, presenter::MenuAnchor, refresh};
use midnight_core::{ShellMenuModifiers, ShellMenuPosition};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{cell::RefCell, rc::Rc};

#[cfg(windows)]
#[path = "shell_bridge_windows.rs"]
mod native;

fn anchor(window: &MainWindow, x: f32, y: f32, control: bool, shift: bool) -> MenuAnchor {
    let x = if x.is_finite() { x } else { 0.0 };
    let y = if y.is_finite() { y } else { 0.0 };
    let mut anchor = MenuAnchor {
        logical: (x, y),
        position: ShellMenuPosition {
            x: x.round() as i32,
            y: y.round() as i32,
        },
        modifiers: ShellMenuModifiers { shift, control },
        owner: 0,
    };
    window.window().with_winit_window(|native_window| {
        let scale = native_window.scale_factor();
        if let Ok(origin) = native_window.inner_position() {
            anchor.position = ShellMenuPosition {
                x: (f64::from(origin.x) + f64::from(x) * scale).round() as i32,
                y: (f64::from(origin.y) + f64::from(y) * scale).round() as i32,
            };
        }
        #[cfg(windows)]
        {
            anchor.owner = native::owner(native_window);
        }
    });
    anchor
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(midnight_core::Command) -> bool>,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_row_context_menu(move |id, row, x, y, control, shift| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let anchor = anchor(&window, x, y, control, shift);
        state
            .borrow_mut()
            .request_entry_menu(pane(id), row, anchor, sender.as_ref());
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_drive_context_menu(move |id, token, x, y, control, shift| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let anchor = anchor(&window, x, y, control, shift);
        state
            .borrow_mut()
            .request_drive_menu(pane(id), token.as_str(), anchor, sender.as_ref());
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_file_clipboard(move |id, operation| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        {
            let mut p = state.borrow_mut();
            match operation {
                0 => p.copy_files(pane(id), sender.as_ref()),
                1 => p.paste_files(pane(id), sender.as_ref()),
                2 => p.request_copy_dialog(pane(id), sender.as_ref()),
                3 => p.request_move_dialog(pane(id), sender.as_ref()),
                4 => p.cut_files(pane(id), sender.as_ref()),
                _ => return,
            }
        }
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send;
    window.on_file_menu_action(move |token, action| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        state
            .borrow_mut()
            .file_menu_action(token.as_str(), action, sender.as_ref());
        refresh(&window, &state.borrow());
    });
    let weak = window.as_weak();
    window.on_file_menu_dismissed(move |token| {
        let weak = weak.clone();
        let presenter = presenter.clone();
        // A refresh can show another popup and synchronously remove this menu's
        // focus while it still borrows the presenter. Retire the menu next tick.
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            presenter.borrow_mut().dismiss_file_menu(token.as_str());
            refresh(&window, &presenter.borrow());
        });
    });
}

pub(crate) fn update(window: &MainWindow, presenter: &Presenter) {
    let (x, y, mutable) = presenter.file_menu_anchor();
    window.set_file_menu_x(x);
    window.set_file_menu_y(y);
    window.set_file_menu_mutable(mutable);
    window.set_file_menu_token(presenter.file_menu_token().into());
}
