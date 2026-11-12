//! Pinned Slint 1.18.1 weak invoking-item focus access.
use i_slint_core::{
    item_tree::{ItemWeak, ParentItemTraversalMode},
    items::{FocusReason, FocusScope, TextInput},
    window::WindowInner,
};

#[derive(Default)]
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
        if !window.is_visible() || crate::dialog_host::dialog_input_blocked(window) {
            return false;
        }
        let Some(item) = self.item.upgrade() else {
            return false;
        };
        if !item.is_visible()
            || !item
                .window_adapter()
                .is_some_and(|adapter| std::ptr::eq(adapter.window(), window))
        {
            return false;
        }
        let mut ancestor = Some(item.clone());
        while let Some(current) = ancestor {
            if current
                .downcast::<FocusScope>()
                .is_some_and(|scope| !scope.as_pin_ref().enabled())
                || current
                    .downcast::<TextInput>()
                    .is_some_and(|input| !input.as_pin_ref().enabled())
            {
                return false;
            }
            ancestor = current.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let inner = WindowInner::from_pub(window);
        inner.set_focus_item(&item, true, FocusReason::Programmatic);
        inner
            .focus_item
            .borrow()
            .upgrade()
            .is_some_and(|focused| focused == item)
    }
}
