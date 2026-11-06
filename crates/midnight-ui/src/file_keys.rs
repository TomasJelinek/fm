//! Shared file-panel key policy; native text editing remains with Slint's TextInput.
use i_slint_core::input::KeyEvent;
use slint::platform::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileKey {
    Native,
    Consumed,
    FocusPath,
    ClearSearch,
    ErasePrevious,
    ClearAndInsert,
    Bulk(i32),
    Sort(i32),
    Action(i32),
    Panel(i32),
    FolderSizes,
    Properties,
    DriveInfo,
    TryDrive,
    Mutation(i32),
}

pub(crate) fn route(event: &KeyEvent, active: bool, composing: bool) -> FileKey {
    let modifiers = event.modifiers;
    let text = event.text.as_str();
    let mut characters = text.chars();
    let first = characters.next();
    let single = first.is_some() && characters.next().is_none();
    let key = |key: Key| single && first == Some(char::from(key));
    if composing && !modifiers.control && !modifiers.alt && !modifiers.meta {
        return FileKey::Native;
    }
    if modifiers.meta {
        return FileKey::Consumed;
    }
    if !modifiers.control && !modifiers.alt {
        if modifiers.shift && key(Key::F4) {
            return FileKey::Mutation(4);
        }
        if key(Key::Delete) || key(Key::F8) {
            return FileKey::Mutation(if modifiers.shift { 2 } else { 1 });
        }
        if !modifiers.shift && key(Key::F2) {
            return FileKey::Mutation(0);
        }
        if !modifiers.shift && key(Key::F7) {
            return FileKey::Mutation(3);
        }
    }
    if modifiers.control
        && !modifiers.alt
        && modifiers.shift
        && (text.eq_ignore_ascii_case("s") || key(Key::F10))
    {
        return FileKey::FolderSizes;
    }
    if modifiers.control && !modifiers.alt && !modifiers.shift {
        for (letter, operation) in [("a", 0), ("d", 1), ("i", 2)] {
            if text.eq_ignore_ascii_case(letter) {
                return FileKey::Bulk(operation);
            }
        }
        for (shortcut, column) in [(Key::F3, 0), (Key::F4, 3), (Key::F5, 2), (Key::F6, 1)] {
            if key(shortcut) {
                return FileKey::Sort(column);
            }
        }
        if key(Key::F9) || text.eq_ignore_ascii_case("r") {
            return FileKey::Action(3);
        }
        if text.eq_ignore_ascii_case("g") {
            return FileKey::FocusPath;
        }
        if key(Key::Space) {
            return FileKey::Native;
        }
        if key(Key::PageUp) {
            return FileKey::Action(2);
        }
        if key(Key::PageDown) {
            return FileKey::Panel(10);
        }
        if key(Key::Backspace) || text == "\\" {
            return FileKey::Action(4);
        }
        if key(Key::F1) || text.eq_ignore_ascii_case("l") {
            return FileKey::DriveInfo;
        }
    }
    if !modifiers.control && !modifiers.alt && modifiers.shift && key(Key::Backspace) {
        return FileKey::Action(4);
    }
    if !modifiers.control && modifiers.alt && !modifiers.shift {
        if key(Key::LeftArrow) {
            return FileKey::Action(0);
        }
        if key(Key::RightArrow) {
            return FileKey::Action(1);
        }
    }
    if modifiers.alt && !modifiers.shift && key(Key::Return) {
        return FileKey::Properties;
    }
    if key(Key::Return) && (modifiers.control || modifiers.alt || modifiers.shift) {
        return FileKey::Consumed;
    }
    if key(Key::Tab) && !modifiers.control && !modifiers.alt {
        return FileKey::Panel(0);
    }
    if modifiers.shift && !modifiers.control && !modifiers.alt && single {
        return FileKey::TryDrive;
    }
    if modifiers.control || modifiers.alt {
        return FileKey::Consumed;
    }
    if modifiers.shift {
        return if active {
            FileKey::Native
        } else {
            FileKey::Consumed
        };
    }
    if key(Key::Escape) {
        return FileKey::ClearSearch;
    }
    if key(Key::Return) {
        return FileKey::Panel(1);
    }
    if key(Key::Backspace) {
        return if active {
            FileKey::ErasePrevious
        } else {
            FileKey::Action(2)
        };
    }
    for (shortcut, operation) in [
        (Key::UpArrow, 2),
        (Key::DownArrow, 3),
        (Key::PageUp, 4),
        (Key::PageDown, 5),
        (Key::Home, 6),
        (Key::End, 7),
    ] {
        if key(shortcut) {
            return FileKey::Panel(operation);
        }
    }
    if key(Key::Space) && !active {
        return FileKey::Panel(8);
    }
    if !active && key(Key::LeftArrow) {
        return FileKey::Panel(11);
    }
    if !active && key(Key::RightArrow) {
        return FileKey::Panel(12);
    }
    if key(Key::Insert) {
        return if active {
            FileKey::ClearAndInsert
        } else {
            FileKey::Panel(9)
        };
    }
    FileKey::Native
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::{SharedString, platform::Key};

    fn event(text: impl Into<SharedString>, control: bool, shift: bool, alt: bool) -> KeyEvent {
        let mut event = KeyEvent::default();
        event.text = text.into();
        event.modifiers.control = control;
        event.modifiers.shift = shift;
        event.modifiers.alt = alt;
        event
    }
    #[test]
    fn mutations_use_file_focus_only_and_preserve_modifier_semantics() {
        for (key, shift, action) in [
            (Key::F2, false, 0),
            (Key::Delete, false, 1),
            (Key::F8, true, 2),
            (Key::Delete, true, 2),
            (Key::F7, false, 3),
            (Key::F4, true, 4),
        ] {
            assert_eq!(
                route(&event(key, false, shift, false), false, false),
                FileKey::Mutation(action)
            );
            assert_ne!(
                route(&event(key, true, shift, false), false, false),
                FileKey::Mutation(action)
            );
            assert_eq!(
                route(&event(key, false, shift, false), false, true),
                FileKey::Native
            );
        }
    }

    #[test]
    fn modified_file_shortcuts_keep_the_existing_policy_and_precedence() {
        for (text, control, shift, alt, expected) in [
            (
                SharedString::from("a"),
                true,
                false,
                false,
                FileKey::Bulk(0),
            ),
            ("D".into(), true, false, false, FileKey::Bulk(1)),
            ("i".into(), true, false, false, FileKey::Bulk(2)),
            (Key::F3.into(), true, false, false, FileKey::Sort(0)),
            (Key::F4.into(), true, false, false, FileKey::Sort(3)),
            (Key::F5.into(), true, false, false, FileKey::Sort(2)),
            (Key::F6.into(), true, false, false, FileKey::Sort(1)),
            ("R".into(), true, false, false, FileKey::Action(3)),
            ("g".into(), true, false, false, FileKey::FocusPath),
            (Key::Space.into(), true, false, false, FileKey::Native),
            (Key::PageDown.into(), true, false, false, FileKey::Panel(10)),
            (
                Key::Backspace.into(),
                true,
                false,
                false,
                FileKey::Action(4),
            ),
            ("l".into(), true, false, false, FileKey::DriveInfo),
            ("s".into(), true, true, false, FileKey::FolderSizes),
            (Key::F10.into(), true, true, false, FileKey::FolderSizes),
            (
                Key::LeftArrow.into(),
                false,
                false,
                true,
                FileKey::Action(0),
            ),
            (Key::Return.into(), true, false, true, FileKey::Properties),
            ("a".into(), true, true, false, FileKey::Consumed),
        ] {
            assert_eq!(
                route(&event(text, control, shift, alt), false, false),
                expected
            );
        }
    }

    #[test]
    fn composition_native_edits_and_shift_drive_keep_the_existing_policy() {
        for (text, active, expected) in [
            (Key::Backspace, false, FileKey::Action(2)),
            (Key::Backspace, true, FileKey::ErasePrevious),
            (Key::Space, false, FileKey::Panel(8)),
            (Key::Space, true, FileKey::Native),
            (Key::Insert, true, FileKey::ClearAndInsert),
            (Key::Escape, true, FileKey::ClearSearch),
            (Key::Home, true, FileKey::Panel(6)),
            (Key::LeftArrow, false, FileKey::Panel(11)),
            (Key::RightArrow, false, FileKey::Panel(12)),
            (Key::LeftArrow, true, FileKey::Native),
            (Key::RightArrow, true, FileKey::Native),
        ] {
            assert_eq!(
                route(&event(text, false, false, false), active, false),
                expected
            );
        }
        assert_eq!(
            route(&event("文", false, false, false), false, false),
            FileKey::Native
        );
        assert_eq!(
            route(&event("D", false, true, false), true, false),
            FileKey::TryDrive
        );
        assert_eq!(
            route(&event("D", false, true, false), true, true),
            FileKey::Native
        );
        assert_eq!(
            route(&event(Key::Backspace, false, false, false), true, true),
            FileKey::Native
        );
        assert_eq!(
            route(&event("a", true, false, false), true, true),
            FileKey::Bulk(0)
        );
        assert_eq!(
            route(&event(Key::Tab, false, true, false), false, false),
            FileKey::Panel(0)
        );
    }
}
