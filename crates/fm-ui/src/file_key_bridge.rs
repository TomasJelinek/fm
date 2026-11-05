//! UI callback dispatch never holds presenter state across native editing or focus changes.
use crate::{
    MainWindow,
    file_keys::{self, FileKey},
};
use slint::ComponentHandle;

pub(crate) fn install(window: &MainWindow) {
    let weak = window.as_weak();
    window.on_files_key(move |pane, event, active, composing| {
        let Some(window) = weak.upgrade() else {
            return 1;
        };
        match file_keys::route(&event, active, composing) {
            FileKey::Native => return 0,
            FileKey::Consumed => return 1,
            FileKey::FocusPath => return 2,
            FileKey::ClearSearch => return 3,
            FileKey::ErasePrevious => return 4,
            FileKey::ClearAndInsert => return 5,
            FileKey::TryDrive => {
                return i32::from(window.invoke_drive_letter(pane, event.text) || !active);
            }
            FileKey::Bulk(operation) => window.invoke_bulk_selection(pane, operation),
            FileKey::Sort(column) => window.invoke_sort(pane, column),
            FileKey::Action(action) => window.invoke_action(pane, action),
            FileKey::Panel(key) => window.invoke_key(pane, key),
            FileKey::FolderSizes => window.invoke_folder_sizes(pane),
            FileKey::Properties => window.invoke_properties(pane),
            FileKey::DriveInfo => window.invoke_drive_info(pane),
        }
        1
    });
}
