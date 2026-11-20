//! Pinned Slint 1.18.1 weak invoking-item focus access.
use i_slint_core::{
    item_tree::{ItemRc, ItemWeak, ParentItemTraversalMode},
    items::{FocusReason, FocusScope, TextInput},
    window::{WindowInner, WindowKind},
};

#[derive(Clone, Default)]
pub(crate) struct FocusReturn {
    item: ItemWeak,
}
impl FocusReturn {
    pub(crate) fn capture(window: &slint::Window) -> Self {
        Self {
            item: WindowInner::from_pub(window).focus_item.borrow().clone(),
        }
    }

    /// Restore Slint's item focus only. This function never activates a native window.
    pub(crate) fn restore(&self, window: &slint::Window) -> bool {
        let Some(item) = self.item_in_current_scope(window) else {
            return false;
        };
        let inner = WindowInner::from_pub(window);
        inner.set_focus_item(&item, true, FocusReason::Programmatic);
        inner
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|focused| focused == item)
    }

    fn item_in_current_scope(&self, window: &slint::Window) -> Option<ItemRc> {
        if !window.is_visible() || crate::dialog_host::dialog_input_blocked(window) {
            return None;
        }
        let item = self.item.upgrade()?;
        if !item.is_visible()
            || !item
                .window_adapter()
                .is_some_and(|adapter| std::ptr::eq(adapter.window(), window))
        {
            return None;
        }
        let mut ancestor = Some(item.clone());
        let mut scope_root = item.clone();
        while let Some(current) = ancestor {
            if current
                .downcast::<FocusScope>()
                .is_some_and(|scope| !scope.as_pin_ref().enabled())
                || current
                    .downcast::<TextInput>()
                    .is_some_and(|input| !input.as_pin_ref().enabled())
            {
                return None;
            }
            scope_root = current.clone();
            ancestor = current.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let inner = WindowInner::from_pub(window);
        let top_root = inner
            .active_popups()
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .map(|popup| ItemRc::new_root(popup.component.clone()))
            .unwrap_or_else(|| ItemRc::new_root(inner.component()));
        if scope_root != top_root {
            return None;
        }
        Some(item)
    }
}

/// Where the keyboard may stay once no dialog or popup is left: a text field (a
/// file list's quick search, the command line, a path editor) or the search
/// results shown over a panel. The control that opened a dialog, popup or tool
/// window with the pointer (a toolbar or drive-bar button, a picker) is not one:
/// the keyboard then returns to the active file list, whatever opened the dialog.
fn keyboard_home(main: &crate::MainWindow) -> bool {
    main.get_feed_focused_pane() >= 0
        || WindowInner::from_pub(slint::ComponentHandle::window(main))
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|item| item.downcast::<TextInput>().is_some())
}

/// Nothing is open over the main window: no form, dialog or popup.
fn nothing_open(main: &crate::MainWindow) -> bool {
    let window = slint::ComponentHandle::window(main);
    window.is_visible()
        && !main.get_embedded_modal_open()
        && !crate::dialog_host::dialog_input_blocked(window)
        && !WindowInner::from_pub(window)
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
}

/// The main window is active again, as when a tool window closes. A pointer-opened
/// control that kept the keyboard (the button that opened the window) gives it to
/// the active file list once nothing else is open.
pub(crate) fn window_activated(main: &crate::MainWindow) {
    use slint::ComponentHandle;
    let weak = main.as_weak();
    // After the activation settles: a click that activated the window may open a
    // dialog, which then owns the keyboard.
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(main) = weak.upgrade() else { return };
        if nothing_open(&main)
            && !main.get_shell_focused()
            && FocusReturn::capture(main.window())
                .item_in_current_scope(main.window())
                .is_some()
            && !keyboard_home(&main)
        {
            main.invoke_focus_files();
        }
    });
}

/// Restore invoking focus after a popup retires, without moving focus out of
/// another modal or reviving an obsolete presentation occurrence. A restored
/// focus outside the keyboard's homes (`keyboard_home`) moves to the file list.
pub(crate) fn restore_after_popup(
    main: &crate::MainWindow,
    focus: Option<FocusReturn>,
    current: impl FnOnce() -> bool + 'static,
) {
    use slint::ComponentHandle;
    let weak = main.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(main) = weak.upgrade() else { return };
        if !current() || !nothing_open(&main) {
            return;
        }
        if !focus.is_some_and(|focus| focus.restore(main.window())) || !keyboard_home(&main) {
            main.invoke_focus_files();
        }
    });
}

/// Let a dialog-wide key handler detect that the focused control already acted
/// on the same key by moving focus. Slint delivers a key to the focused item
/// and then to its ancestors; a Fluent ComboBox opens its popup on Return (which
/// takes focus) and still rejects the key, so the dialog's default action would
/// otherwise run as well.
pub(crate) fn install_key_focus(main: &crate::MainWindow) {
    use slint::ComponentHandle;
    use std::{cell::RefCell, rc::Rc};
    let marked = Rc::new(RefCell::new(ItemWeak::default()));
    let weak = main.as_weak();
    let mark = marked.clone();
    main.global::<crate::PopupFocus>()
        .on_mark_key_focus(move || {
            if let Some(main) = weak.upgrade() {
                let focus = WindowInner::from_pub(main.window())
                    .focus_item
                    .borrow()
                    .clone();
                *mark.borrow_mut() = focus;
            }
        });
    let weak = main.as_weak();
    main.global::<crate::PopupFocus>()
        .on_key_moved_focus(move || {
            weak.upgrade().is_some_and(|main| {
                *WindowInner::from_pub(main.window()).focus_item.borrow() != *marked.borrow()
            })
        });
}

/// Slint restores a popup's old parent focus even when a newer popup remains
/// above it. Preserve the newer dialog's invoking control across that update.
pub(crate) fn install(main: &crate::MainWindow) {
    install_key_focus(main);
    use slint::ComponentHandle;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    let saved = Rc::new(RefCell::new(None::<FocusReturn>));
    let updating = Rc::new(Cell::new(false));
    let weak = main.as_weak();
    main.global::<crate::PopupFocus>().on_update(move || {
        let Some(main) = weak.upgrade() else { return };
        if updating.replace(true) {
            return;
        }
        let current = FocusReturn::capture(main.window());
        if main.get_embedded_modal_open() {
            if current.item_in_current_scope(main.window()).is_some() {
                *saved.borrow_mut() = Some(current);
            } else {
                let focus = saved.borrow().clone();
                if let Some(focus) = focus {
                    focus.restore(main.window());
                }
            }
        } else {
            saved.borrow_mut().take();
            // A dialog or popup closed: Slint gave the keyboard back to what had it
            // before, which may be the toolbar button that opened it.
            let no_popup = !WindowInner::from_pub(main.window())
                .active_popups()
                .iter()
                .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip));
            if no_popup
                && main.window().is_visible()
                && (current.item_in_current_scope(main.window()).is_none() || !keyboard_home(&main))
            {
                main.invoke_focus_files();
            }
        }
        updating.set(false);
    });
}
