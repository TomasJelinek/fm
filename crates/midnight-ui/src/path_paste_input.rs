//! Pinned Slint 1.18 committed-text insertion; never redispatch through global shortcuts.
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    input::{InternalKeyEvent, KeyEvent, KeyEventType},
    item_tree::{ItemRc, ItemWeak},
    items::TextInput,
    window::WindowInner,
};
use slint::{ComponentHandle, SharedString};

#[derive(Clone)]
pub(super) struct Snapshot {
    item: ItemWeak,
    text: SharedString,
    anchor: i32,
    cursor: i32,
    preedit: SharedString,
}
impl Snapshot {
    pub(super) fn capture(window: &crate::MainWindow) -> Option<Self> {
        let item = WindowInner::from_pub(window.window())
            .focus_item
            .borrow()
            .upgrade()?;
        if item
            .accessible_string_property(AccessibleStringProperty::Label)
            .as_deref()
            != Some("Folder path")
        {
            return None;
        }
        let input = item.downcast::<TextInput>()?;
        let input = input.as_pin_ref();
        Some(Self {
            item: item.downgrade(),
            text: input.text(),
            anchor: input.anchor_position_byte_offset(),
            cursor: input.cursor_position_byte_offset(),
            preedit: input.preedit_text(),
        })
    }
    fn target(&self, window: &crate::MainWindow) -> Option<ItemRc> {
        let item = self.item.upgrade()?;
        if WindowInner::from_pub(window.window())
            .focus_item
            .borrow()
            .upgrade()
            .as_ref()
            != Some(&item)
            || !item.is_visible()
        {
            return None;
        }
        let input = item.downcast::<TextInput>()?;
        let input = input.as_pin_ref();
        (input.has_focus()
            && input.enabled()
            && !input.read_only()
            && input.text() == self.text
            && input.anchor_position_byte_offset() == self.anchor
            && input.cursor_position_byte_offset() == self.cursor
            && input.preedit_text() == self.preedit)
            .then_some(item)
    }
    pub(super) fn current(&self, window: &crate::MainWindow) -> bool {
        self.target(window).is_some()
    }
    /// Call after all Presenter/controller borrows and the original key callback return.
    pub(super) fn insert(&self, window: &crate::MainWindow, text: &str) -> bool {
        let Some(item) = self.target(window) else {
            return false;
        };
        let Some(adapter) = item.window_adapter() else {
            return false;
        };
        // Match native paste: discard the old composition before committing the
        // literal payload; TextInput updates IME/cursor state through its normal edit.
        let Some(input) = item.downcast::<TextInput>() else {
            return false;
        };
        input.as_pin_ref().preedit_text.set(Default::default());
        let mut key = KeyEvent::default();
        key.text = text.into();
        item.borrow().as_ref().key_event(
            &InternalKeyEvent {
                key_event: key,
                event_type: KeyEventType::CommitComposition,
                ..Default::default()
            },
            &adapter,
            &item,
        );
        true
    }
}
