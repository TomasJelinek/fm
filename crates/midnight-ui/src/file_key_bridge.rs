//! Dispatch semantic file actions without holding state across focus changes.
use crate::{
    MainWindow,
    file_keys::{FileKey, WindowAction},
};

pub(crate) fn dispatch(window: &MainWindow, pane: i32, action: FileKey) -> i32 {
    match action {
        FileKey::Native => return 0,
        FileKey::Consumed => return 1,
        FileKey::FocusPath => return 2,
        FileKey::ClearSearch => return 3,
        FileKey::ErasePrevious => return 4,
        FileKey::ClearAndInsert => return 5,
        FileKey::TryDrive(letter) => {
            let search_active = if pane == 0 {
                window.get_left_search_active()
            } else {
                window.get_right_search_active()
            };
            return i32::from(
                window.invoke_drive_letter(pane, letter.to_string().into()) || !search_active,
            );
        }
        FileKey::Bulk(operation) => window.invoke_bulk_selection(pane, operation),
        FileKey::Sort(column) => window.invoke_sort(pane, column),
        FileKey::Action(action) => window.invoke_action(pane, action),
        FileKey::Panel(key) => window.invoke_key(pane, key),
        FileKey::FolderSizes => window.invoke_folder_sizes(pane),
        FileKey::Properties => window.invoke_properties(pane),
        FileKey::DriveInfo => window.invoke_drive_info(pane),
        FileKey::Metadata(case) => window.invoke_open_metadata(pane, case),
        FileKey::Compare => window.invoke_open_comparison(),
        FileKey::Filter => window.invoke_open_panel_filter(pane),
        FileKey::Zoom => window.invoke_toggle_panel_zoom(pane),
        FileKey::Mutation(action) => window.invoke_mutation(pane, action),
        FileKey::Clipboard(operation) => window.invoke_file_clipboard(pane, operation),
        FileKey::Window(action) => dispatch_window(window, pane, action),
    }
    1
}

pub(crate) fn dispatch_window(window: &MainWindow, pane: i32, action: WindowAction) {
    use WindowAction as A;
    let displayed_pane = |pane| {
        if window.get_panels_swapped() {
            1 - pane
        } else {
            pane
        }
    };
    match action {
        A::OtherPanel => window.invoke_commit_volume_shortcut(pane),
        A::SwapPanels => window.invoke_swap_panels(),
        A::NewTab => window.invoke_tab_new(pane),
        A::CloseTab => window.invoke_tab_close_active(pane),
        A::PreviousTab => window.invoke_tab_cycle(pane, -1),
        A::Tab(number) => window.invoke_tab_number(pane, number),
        A::BookmarkAdd => window.invoke_bookmark_add(
            pane,
            if pane == 0 {
                window.get_left_bookmark_name()
            } else {
                window.get_right_bookmark_name()
            },
            0,
        ),
        A::BookmarkAssign(slot) => window.invoke_bookmark_slot(pane, slot),
        A::BookmarkOpen(slot, other) => {
            window.invoke_bookmark_jump(if other { 1 - pane } else { pane }, slot)
        }
        A::ToggleCommand => {
            if window.get_command_focused() {
                window.invoke_focus_files();
            } else {
                window.invoke_focus_command();
            }
        }
        A::InsertName => window.invoke_insert_name(),
        A::InsertPath(position) => window.invoke_insert_path(if position < 0 {
            window.get_active_pane()
        } else {
            displayed_pane(position)
        }),
        A::ExecuteCommand => window.invoke_execute_command(),
        A::CommandHistory(older) => window.invoke_command_history_step(older),
        A::FocusFiles => window.invoke_focus_files(),
        A::Volumes(position) => window.invoke_open_volume_menu(displayed_pane(position)),
        A::Help => window.invoke_show_app_menu(0),
        A::UserMenu => window.invoke_show_app_menu(1),
        A::MainMenu => window.invoke_show_app_menu(2),
        A::Exit => window.invoke_exit(),
        A::Network(disconnect) => {
            if window.get_network_available() {
                window.invoke_network_connections(disconnect);
            }
        }
    }
}
