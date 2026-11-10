//! Shared file-panel key policy; native text editing remains with Slint's TextInput.
use i_slint_core::input::KeyEvent;
use midnight_core::{ShortcutContext, ShortcutMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowAction {
    OtherPanel,
    SwapPanels,
    NewTab,
    CloseTab,
    PreviousTab,
    Tab(i32),
    BookmarkAdd,
    BookmarkAssign(i32),
    BookmarkOpen(i32, bool),
    ToggleCommand,
    InsertName,
    InsertPath(i32),
    ExecuteCommand,
    CommandHistory(bool),
    FocusFiles,
    Volumes(i32),
    Help,
    UserMenu,
    MainMenu,
    Options,
    Jobs,
    About,
    CustomCommands,
    Bookmarks,
    ViewMode(i32),
    Exit,
    Network(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    Metadata(bool),
    View(bool),
    CompareFiles,
    Find,
    Checksums,
    MakeFileList,
    UserCommand(midnight_core::UserCommandId),
    Edit,
    Compare,
    Filter,
    Zoom,
    TryDrive(char),
    Mutation(i32),
    Clipboard(i32),
    Window(WindowAction),
}

#[cfg(test)]
pub(crate) fn route(event: &KeyEvent, active: bool, composing: bool) -> FileKey {
    route_with(
        &ShortcutMap::new(&[]).expect("default shortcuts"),
        event,
        active,
        composing,
    )
}

pub(crate) fn route_with(
    map: &ShortcutMap,
    event: &KeyEvent,
    active: bool,
    composing: bool,
) -> FileKey {
    let modifiers = event.modifiers;
    if composing && !modifiers.control && !modifiers.alt && !modifiers.meta {
        return FileKey::Native;
    }
    let Some(chord) = crate::shortcut_input::chord(event) else {
        return FileKey::Native;
    };
    if chord.control && !chord.alt && !chord.shift && !chord.meta && chord.key == "Space" {
        return FileKey::Native;
    }
    let context = if active {
        ShortcutContext::Search
    } else {
        ShortcutContext::Files
    };
    if let Some(action) = map.resolve(&chord, context) {
        if composing
            && matches!(
                action.id.as_str(),
                "files.copy" | "files.paste" | "files.cut"
            )
        {
            return FileKey::Native;
        }
        return semantic(&action.id, active).unwrap_or(FileKey::Consumed);
    }
    if active || composing {
        return FileKey::Native;
    }
    if modifiers.control || modifiers.alt || modifiers.meta || modifiers.shift {
        FileKey::Consumed
    } else {
        FileKey::Native
    }
}

/// Map semantic actions to existing callbacks; no keyboard chord appears here.
pub(crate) fn semantic(action: &str, active: bool) -> Option<FileKey> {
    use WindowAction as W;
    Some(match action {
        "panel.switch" => FileKey::Panel(0),
        "panel.open" => FileKey::Panel(1),
        "panel.up" => FileKey::Panel(2),
        "panel.down" => FileKey::Panel(3),
        "panel.page-up" => FileKey::Panel(4),
        "panel.page-down" => FileKey::Panel(5),
        "panel.first" => FileKey::Panel(6),
        "panel.last" => FileKey::Panel(7),
        "selection.toggle-size" => FileKey::Panel(8),
        "selection.toggle" => {
            if active {
                FileKey::ClearAndInsert
            } else {
                FileKey::Panel(9)
            }
        }
        "panel.enter-folder" => FileKey::Panel(10),
        "panel.left" => FileKey::Panel(11),
        "panel.right" => FileKey::Panel(12),
        "panel.back" => FileKey::Action(0),
        "panel.forward" => FileKey::Action(1),
        "panel.parent" => FileKey::Action(2),
        "panel.refresh" => FileKey::Action(3),
        "panel.root" => FileKey::Action(4),
        "panel.path" => FileKey::FocusPath,
        "files.find" => FileKey::Find,
        "files.checksums" => FileKey::Checksums,
        "files.make-list" => FileKey::MakeFileList,
        "panel.folder-sizes" => FileKey::FolderSizes,
        "panel.drive-info" => FileKey::DriveInfo,
        "panel.compare" => FileKey::Compare,
        "panel.filter" => FileKey::Filter,
        "panel.zoom" => FileKey::Zoom,
        "files.attributes" => FileKey::Metadata(false),
        "files.view" => FileKey::View(false),
        "files.compare" => FileKey::CompareFiles,
        "files.view-basic" => FileKey::View(true),
        "files.edit" => FileKey::Edit,
        "files.case" => FileKey::Metadata(true),
        "selection.all" => FileKey::Bulk(0),
        "selection.files" => FileKey::Bulk(3),
        "selection.clear" => FileKey::Bulk(1),
        "selection.invert" => FileKey::Bulk(2),
        "selection.invert-all" => FileKey::Bulk(4),
        "sort.name" => FileKey::Sort(0),
        "sort.extension" => FileKey::Sort(3),
        "sort.modified" => FileKey::Sort(2),
        "sort.size" => FileKey::Sort(1),
        "search.clear" => FileKey::ClearSearch,
        "search.erase" => FileKey::ErasePrevious,
        "files.rename" => FileKey::Mutation(0),
        "files.trash" => FileKey::Mutation(1),
        "files.delete" => FileKey::Mutation(2),
        "files.create-directory" => FileKey::Mutation(3),
        "files.create-file" => FileKey::Mutation(4),
        "files.properties" => FileKey::Properties,
        "files.copy" => FileKey::Clipboard(0),
        "files.paste" => FileKey::Clipboard(1),
        "files.copy-to" => FileKey::Clipboard(2),
        "files.move-to" => FileKey::Clipboard(3),
        "files.cut" => FileKey::Clipboard(4),
        "panel.as-other" => FileKey::Window(W::OtherPanel),
        "panel.swap" => FileKey::Window(W::SwapPanels),
        "tab.new" => FileKey::Window(W::NewTab),
        "tab.close" => FileKey::Window(W::CloseTab),
        "tab.previous" => FileKey::Window(W::PreviousTab),
        "bookmark.add" => FileKey::Window(W::BookmarkAdd),
        "bookmark.assign.10" => FileKey::Window(W::BookmarkAssign(10)),
        "command.focus" => FileKey::Window(W::ToggleCommand),
        "command.insert-name" => FileKey::Window(W::InsertName),
        "command.insert-path" => FileKey::Window(W::InsertPath(-1)),
        "command.insert-left-path" => FileKey::Window(W::InsertPath(0)),
        "command.insert-right-path" => FileKey::Window(W::InsertPath(1)),
        "command.execute" => FileKey::Window(W::ExecuteCommand),
        "command.history-older" => FileKey::Window(W::CommandHistory(true)),
        "command.history-newer" => FileKey::Window(W::CommandHistory(false)),
        "command.focus-files" => FileKey::Window(W::FocusFiles),
        "drive.left" => FileKey::Window(W::Volumes(0)),
        "drive.right" => FileKey::Window(W::Volumes(1)),
        "app.help" => FileKey::Window(W::Help),
        "app.user-menu" => FileKey::Window(W::UserMenu),
        "app.main-menu" => FileKey::Window(W::MainMenu),
        "app.options" => FileKey::Window(W::Options),
        "app.jobs" => FileKey::Window(W::Jobs),
        "app.about" => FileKey::Window(W::About),
        "app.custom-commands" => FileKey::Window(W::CustomCommands),
        "bookmark.manage" => FileKey::Window(W::Bookmarks),
        "view.brief" => FileKey::Window(W::ViewMode(1)),
        "view.detailed" => FileKey::Window(W::ViewMode(0)),
        "view.thumbnails" => FileKey::Window(W::ViewMode(2)),
        "app.exit" => FileKey::Window(W::Exit),
        "network.connect" => FileKey::Window(W::Network(false)),
        "network.disconnect" => FileKey::Window(W::Network(true)),
        _ => {
            if let Some(id) = action.strip_prefix("user-command.") {
                FileKey::UserCommand(midnight_core::UserCommandId::new(id).ok()?)
            } else if let Some(number) = numbered(action, "tab.", 9) {
                FileKey::Window(W::Tab(number))
            } else if let Some(number) = numbered(action, "bookmark.open.", 10) {
                FileKey::Window(W::BookmarkOpen(number, false))
            } else if let Some(number) = numbered(action, "bookmark.other.", 10) {
                FileKey::Window(W::BookmarkOpen(number, true))
            } else {
                let letter = action
                    .strip_prefix("drive.")
                    .filter(|letter| letter.len() == 1)
                    .and_then(|letter| letter.chars().next())
                    .filter(char::is_ascii_lowercase)?;
                FileKey::TryDrive(letter.to_ascii_uppercase())
            }
        }
    })
}

fn numbered(action: &str, prefix: &str, maximum: i32) -> Option<i32> {
    action
        .strip_prefix(prefix)?
        .parse::<i32>()
        .ok()
        .filter(|number| (1..=maximum).contains(number))
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
    fn clipboard_keys_use_file_actions_only_outside_native_text_editing() {
        for (text, operation) in [("c", 0), ("v", 1), ("x", 4)] {
            assert_eq!(
                route(&event(text, true, false, false), false, false),
                FileKey::Clipboard(operation)
            );
            assert_eq!(
                route(&event(text, true, false, false), true, false),
                FileKey::Native,
                "an active quick-search editor keeps its native clipboard"
            );
            assert_eq!(
                route(&event(text, true, false, false), false, true),
                FileKey::Native,
                "composition keeps its native clipboard"
            );
            assert_ne!(
                route(&event(text, true, true, false), false, false),
                FileKey::Clipboard(operation)
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
            (Key::F7.into(), false, false, true, FileKey::Find),
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
            ("a".into(), true, true, false, FileKey::Bulk(0)),
            ("i".into(), true, true, false, FileKey::Bulk(4)),
        ] {
            assert_eq!(
                route(&event(text, control, shift, alt), false, false),
                expected
            );
        }
    }

    #[test]
    fn move_clipboard_actions_use_semantic_file_routes() {
        assert_eq!(
            semantic("files.move-to", false),
            Some(FileKey::Clipboard(3))
        );
        assert_eq!(semantic("files.cut", false), Some(FileKey::Clipboard(4)));
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
            FileKey::TryDrive('D')
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

    #[test]
    fn every_catalog_action_has_one_semantic_dispatch() {
        for action in midnight_core::shortcut_catalog() {
            if matches!(
                action.scope,
                midnight_core::ShortcutScope::ViewerAny
                    | midnight_core::ShortcutScope::ViewerContent
            ) {
                continue;
            }
            assert!(
                semantic(&action.id, false).is_some(),
                "missing {}",
                action.id
            );
        }
        assert_eq!(semantic("unknown.action", false), None);
    }
}
