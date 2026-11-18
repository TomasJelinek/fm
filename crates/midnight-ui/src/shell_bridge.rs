//! Convert UI gestures into captured native-menu and file-clipboard intents.
use crate::{
    BeginOwned, MainWindow, Presenter, pane, presenter::MenuAnchor, refresh, with_owned_commands,
};
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
    begin_owned: BeginOwned,
) {
    let weak = window.as_weak();
    let state = presenter.clone();
    let begin = begin_owned.clone();
    window.on_row_context_menu(move |id, row, x, y, control, shift| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let Ok(row) = usize::try_from(row) else {
            return;
        };
        let anchor = anchor(&window, x, y, control, shift);
        if with_owned_commands(&state, &begin, |p, send| {
            p.request_entry_menu(pane(id), row, anchor, send);
        })
        .is_none()
        {
            report_busy(&state);
        }
        refresh_if_idle(&window, &state);
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let sender = send.clone();
    window.on_drive_context_menu(move |id, token, x, y, control, shift| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let anchor = anchor(&window, x, y, control, shift);
        if !request_drive_menu(&window, &state, sender.as_ref(), id, &token, anchor) {
            // Delivered while Presenter is borrowed, as during a refresh: open the
            // menu on the next tick. A borrow that outlasts it (a nested native
            // loop) drops this right-click, as busy row menus are dropped.
            let (weak, state, sender) = (weak.clone(), state.clone(), sender.clone());
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(window) = weak.upgrade() {
                    request_drive_menu(&window, &state, sender.as_ref(), id, &token, anchor);
                }
            });
        }
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let begin = begin_owned.clone();
    window.on_file_clipboard(move |id, operation| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if with_owned_commands(&state, &begin, |p, send| match operation {
            0 => p.copy_files(pane(id), send),
            1 => p.paste_files(pane(id), send),
            2 => p.request_copy_dialog(pane(id), send),
            3 => p.request_move_dialog(pane(id), send),
            4 => p.cut_files(pane(id), send),
            _ => (),
        })
        .is_none()
        {
            report_busy(&state);
        }
        refresh_if_idle(&window, &state);
    });
    let weak = window.as_weak();
    let state = presenter.clone();
    let begin = begin_owned.clone();
    window.on_file_menu_action(move |token, action| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if with_owned_commands(&state, &begin, |p, send| {
            p.file_menu_action(token.as_str(), action, send);
        })
        .is_none()
        {
            // The popup closed when the item was chosen. Release the captured menu
            // as a dismissal does, or it would block every later context menu; a
            // captured tree-menu owner stays in its slot until an owned action
            // retires it.
            if let Ok(mut p) = state.try_borrow_mut() {
                p.dismiss_file_menu(token.as_str());
            }
            report_busy(&state);
        }
        refresh_if_idle(&window, &state);
    });
    let weak = window.as_weak();
    window.on_file_menu_dismissed(move |token| {
        let weak = weak.clone();
        let presenter = presenter.clone();
        let begin = begin_owned.clone();
        // A refresh can show another popup and synchronously remove this menu's
        // focus while it still borrows the presenter. Retire the menu next tick.
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if with_owned_commands(&presenter, &begin, |p, _| {
                p.dismiss_file_menu(token.as_str());
            })
            .is_none()
            {
                // The captured tree-menu owner stays in its existing slot.
                if let Ok(mut p) = presenter.try_borrow_mut() {
                    p.dismiss_file_menu(token.as_str());
                }
            }
            refresh_if_idle(&window, &presenter);
        });
    });
}

/// Request a drive's root menu unless Presenter is already borrowed.
fn request_drive_menu(
    window: &MainWindow,
    presenter: &Rc<RefCell<Presenter>>,
    send: &dyn Fn(midnight_core::Command) -> bool,
    id: i32,
    token: &str,
    anchor: MenuAnchor,
) -> bool {
    let Ok(mut p) = presenter.try_borrow_mut() else {
        return false;
    };
    p.request_drive_menu(pane(id), token, anchor, send);
    drop(p);
    refresh_if_idle(window, presenter);
    true
}

fn report_busy(presenter: &Rc<RefCell<Presenter>>) {
    if let Ok(mut p) = presenter.try_borrow_mut() {
        p.desktop.report(
            "File action busy",
            "Wait for the current action to finish and try again.".into(),
        );
    }
}

fn refresh_if_idle(window: &MainWindow, presenter: &Rc<RefCell<Presenter>>) {
    if let Ok(p) = presenter.try_borrow() {
        refresh(window, &p);
    }
}

pub(crate) fn update(window: &MainWindow, presenter: &Presenter) {
    let (x, y, mutable) = presenter.file_menu_anchor();
    window.set_file_menu_x(x);
    window.set_file_menu_y(y);
    window.set_file_menu_mutable(mutable);
    window.set_file_menu_token(presenter.file_menu_token().into());
}
