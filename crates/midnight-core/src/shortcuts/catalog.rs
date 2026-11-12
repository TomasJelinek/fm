//! Stable action IDs shared by routing, Options and displayed key hints.
use super::{ShortcutDefinition, ShortcutScope};
use std::sync::OnceLock;

pub fn shortcut_catalog() -> &'static [ShortcutDefinition] {
    static CATALOG: OnceLock<Vec<ShortcutDefinition>> = OnceLock::new();
    CATALOG.get_or_init(build)
}

fn build() -> Vec<ShortcutDefinition> {
    use ShortcutScope::*;
    let mut catalog = Vec::new();
    let mut add = |id: &str, label: &str, scope, keys: &[&str]| {
        catalog.push(ShortcutDefinition {
            id: id.into(),
            label: label.into(),
            scope,
            defaults: keys.iter().map(|key| (*key).into()).collect(),
        });
    };
    add(
        "panel.as-other",
        "Open other panel's folder",
        Volume,
        &["Period"],
    );
    add(
        "panel.switch",
        "Switch active panel",
        FilesAny,
        &["Tab", "Shift+Tab"],
    );
    add(
        "panel.open",
        "Open focused file or folder",
        FilesAny,
        &["Enter"],
    );
    add(
        "panel.up",
        "Previous row or search match",
        FilesAny,
        &["Up"],
    );
    add(
        "panel.down",
        "Next row or search match",
        FilesAny,
        &["Down"],
    );
    add(
        "panel.previous-selected",
        "Previous selected file or folder",
        FilesIdle,
        &["Alt+Up"],
    );
    add(
        "panel.next-selected",
        "Next selected file or folder",
        FilesIdle,
        &["Alt+Down"],
    );
    add("panel.page-up", "Move one page up", FilesAny, &["PageUp"]);
    add(
        "panel.page-down",
        "Move one page down",
        FilesAny,
        &["PageDown"],
    );
    add("panel.first", "First row", FilesAny, &["Home"]);
    add("panel.last", "Last row", FilesAny, &["End"]);
    add(
        "panel.left",
        "Previous file column or grid cell",
        FilesIdle,
        &["Left"],
    );
    add(
        "panel.right",
        "Next file column or grid cell",
        FilesIdle,
        &["Right"],
    );
    add(
        "panel.parent",
        "Parent folder",
        FilesAny,
        &["Backspace", "Ctrl+PageUp"],
    );
    add(
        "panel.root",
        "Filesystem root",
        FilesAny,
        &["Ctrl+Backspace", "Shift+Backspace", "Ctrl+Backslash"],
    );
    add(
        "panel.enter-folder",
        "Enter focused folder without opening files",
        FilesAny,
        &["Ctrl+PageDown"],
    );
    add(
        "panel.back",
        "Previous folder in history",
        FilesAny,
        &["Alt+Left"],
    );
    add(
        "panel.forward",
        "Next folder in history",
        FilesAny,
        &["Alt+Right"],
    );
    add(
        "panel.refresh",
        "Refresh active panel",
        FilesAny,
        &["Ctrl+R", "Ctrl+F9"],
    );
    add("panel.path", "Focus path editor", FilesAny, &["Ctrl+G"]);
    add("panel.swap", "Swap complete panels", FilesAny, &["Ctrl+U"]);
    add(
        "panel.compare",
        "Compare panel directories",
        FilesAny,
        &["Ctrl+F10"],
    );
    add("panel.zoom", "Zoom active panel", FilesAny, &["Ctrl+F11"]);
    add("files.view", "View focused file", FilesAny, &["F3"]);
    add(
        "files.find",
        "Find files and text",
        FilesAny,
        &["Ctrl+F", "Alt+F7"],
    );
    add(
        "files.checksums",
        "Calculate checksums",
        FilesAny,
        &["Ctrl+F8"],
    );
    add(
        "files.compare",
        "Compare two files",
        FilesAny,
        &["Ctrl+Shift+C"],
    );
    add(
        "files.view-basic",
        "View as plain text",
        FilesAny,
        &["Alt+F3"],
    );
    add(
        "files.edit",
        "Edit focused file externally",
        FilesAny,
        &["F4"],
    );
    add("viewer.close", "Close viewer", ViewerAny, &["Escape"]);
    add(
        "compare.next-difference",
        "Next comparison difference",
        ComparisonContent,
        &["F7", "Alt+Down", "Alt+Right", "N"],
    );
    add(
        "compare.previous-difference",
        "Previous comparison difference",
        ComparisonContent,
        &["Shift+F7", "Alt+Up", "Alt+Left", "P"],
    );
    add(
        "compare.first-difference",
        "First comparison difference",
        ComparisonContent,
        &["Alt+Home"],
    );
    add(
        "compare.last-difference",
        "Last comparison difference",
        ComparisonContent,
        &["Alt+End"],
    );
    add(
        "viewer.next-file",
        "View next captured file",
        ViewerOnlyContent,
        &["Space", "Alt+Right"],
    );
    add(
        "viewer.previous-file",
        "View previous captured file",
        ViewerOnlyContent,
        &["Backspace", "Alt+Left"],
    );
    add("viewer.find", "Find in viewed file", ViewerAny, &["Ctrl+F"]);
    add("viewer.wrap", "Wrap viewer text", ViewerAny, &["Ctrl+W"]);
    add(
        "viewer.zoom-in",
        "Zoom viewed image in",
        ViewerContent,
        &["Ctrl+Plus", "Ctrl+Equal"],
    );
    add(
        "viewer.zoom-out",
        "Zoom viewed image out",
        ViewerContent,
        &["Ctrl+Minus"],
    );
    add(
        "viewer.fit",
        "Fit viewed image to window",
        ViewerContent,
        &["Ctrl+Digit0"],
    );
    add(
        "viewer.actual-pixels",
        "View actual image pixels",
        ViewerContent,
        &["Ctrl+Digit1"],
    );
    add(
        "viewer.find-next",
        "Next match in viewed file",
        ViewerAny,
        &["F3"],
    );
    add(
        "viewer.find-previous",
        "Previous match in viewed file",
        ViewerAny,
        &["Shift+F3"],
    );
    add(
        "viewer.copy",
        "Copy viewer selection",
        ViewerContent,
        &["Ctrl+C"],
    );
    add(
        "viewer.select-page",
        "Select loaded viewer page",
        ViewerContent,
        &["Ctrl+A"],
    );
    add("viewer.up", "Scroll viewer up", ViewerContent, &["Up"]);
    add(
        "viewer.down",
        "Scroll viewer down",
        ViewerContent,
        &["Down"],
    );
    add(
        "viewer.left",
        "Scroll viewer left",
        ViewerContent,
        &["Left"],
    );
    add(
        "viewer.right",
        "Scroll viewer right",
        ViewerContent,
        &["Right"],
    );
    add(
        "viewer.page-up",
        "Previous viewer screen",
        ViewerContent,
        &["PageUp"],
    );
    add(
        "viewer.page-down",
        "Next viewer screen",
        ViewerContent,
        &["PageDown"],
    );
    add(
        "viewer.first",
        "Beginning of viewed file",
        ViewerContent,
        &["Ctrl+Home"],
    );
    add(
        "viewer.document-previous",
        "Previous document page",
        ViewerContent,
        &["Ctrl+PageUp"],
    );
    add(
        "viewer.document-next",
        "Next document page",
        ViewerContent,
        &["Ctrl+PageDown"],
    );
    add(
        "viewer.last",
        "End of viewed file",
        ViewerContent,
        &["Ctrl+End"],
    );
    add("panel.filter", "Edit panel filter", FilesAny, &["Ctrl+F12"]);
    add(
        "panel.drive-info",
        "Show drive information",
        FilesAny,
        &["Ctrl+F1", "Ctrl+L"],
    );
    add(
        "panel.folder-sizes",
        "Calculate or cancel folder sizes",
        FilesAny,
        &["Ctrl+Shift+S", "Ctrl+Shift+F10"],
    );
    add(
        "selection.toggle-size",
        "Toggle selection and calculate focused folder size",
        FilesIdle,
        &["Space"],
    );
    add(
        "selection.toggle",
        "Toggle selection and advance",
        FilesAny,
        &["Insert"],
    );
    add(
        "selection.all",
        "Select files and folders",
        FilesAny,
        &["Ctrl+Shift+A"],
    );
    add("files.make-list", "Make File List", FilesAny, &["Ctrl+M"]);
    add(
        "selection.files",
        "Select files only",
        FilesAny,
        &["Ctrl+A"],
    );
    add(
        "selection.clear-files",
        "Deselect files only",
        FilesAny,
        &[],
    );
    add(
        "selection.clear",
        "Deselect files and folders",
        FilesAny,
        &["Ctrl+D"],
    );
    add(
        "selection.invert",
        "Invert files only",
        FilesAny,
        &["Ctrl+I"],
    );
    add(
        "selection.invert-all",
        "Invert top-level files and folders",
        FilesAny,
        &["Ctrl+Shift+I"],
    );
    add("sort.name", "Sort by name", FilesAny, &["Ctrl+F3"]);
    add(
        "sort.extension",
        "Sort by extension",
        FilesAny,
        &["Ctrl+F4"],
    );
    add(
        "sort.modified",
        "Sort by modification time",
        FilesAny,
        &["Ctrl+F5"],
    );
    add("sort.size", "Sort by size", FilesAny, &["Ctrl+F6"]);
    add(
        "search.clear",
        "Leave filename search",
        FilesSearch,
        &["Escape"],
    );
    add(
        "search.erase",
        "Erase previous search grapheme",
        FilesSearch,
        &["Backspace"],
    );
    add("files.rename", "Rename focused item", FilesAny, &["F2"]);
    add(
        "files.attributes",
        "Edit attributes and times",
        FilesAny,
        &["Ctrl+F2"],
    );
    add("files.case", "Change filename case", FilesAny, &["Ctrl+F7"]);
    add(
        "files.create-file",
        "Create empty file",
        FilesAny,
        &["Shift+F4"],
    );
    add("files.create-directory", "Create folder", FilesAny, &["F7"]);
    add(
        "files.trash",
        "Move selection to Trash",
        FilesAny,
        &["Delete", "F8"],
    );
    add(
        "files.delete",
        "Permanently delete selection",
        FilesAny,
        &["Shift+Delete", "Shift+F8"],
    );
    add(
        "files.properties",
        "Properties of selected or focused items",
        FilesAny,
        &["Alt+Enter", "Ctrl+Alt+Enter"],
    );
    add(
        "files.copy-to",
        "Copy selected files to folder",
        FilesAny,
        &["F5"],
    );
    add(
        "files.copy",
        "Copy files to clipboard",
        FilesIdle,
        &["Ctrl+C"],
    );
    add(
        "files.move-to",
        "Move selected files to folder with recovery",
        FilesAny,
        &["F6"],
    );
    add(
        "files.cut",
        "Cut native files to clipboard",
        FilesIdle,
        &["Ctrl+X"],
    );
    add(
        "files.paste",
        "Paste files into active folder",
        FilesIdle,
        &["Ctrl+V"],
    );
    add("tab.new", "New tab", FilesAny, &["Ctrl+T"]);
    add("tab.close", "Close active tab", FilesAny, &["Ctrl+W"]);
    add(
        "tab.previous",
        "Previous tab",
        FilesAny,
        &["Ctrl+Shift+Tab"],
    );
    add(
        "bookmark.add",
        "Bookmark current folder",
        FilesAny,
        &["Ctrl+Shift+Plus", "Ctrl+Shift+Equal"],
    );
    add(
        "bookmark.assign.10",
        "Assign current folder to Hot Path 10",
        FilesAny,
        &["Ctrl+Shift+Digit0"],
    );
    add(
        "command.focus",
        "Toggle command editor focus",
        Global,
        &["Ctrl+Tab"],
    );
    add(
        "command.insert-name",
        "Append focused basename to command",
        FilesOrCommand,
        &["Ctrl+Enter"],
    );
    add(
        "command.insert-full-path",
        "Append focused full path to command",
        FilesOrCommand,
        &["Ctrl+Shift+P"],
    );
    add(
        "command.insert-path",
        "Insert active panel path into command",
        FilesOrCommand,
        &["Ctrl+P"],
    );
    add(
        "command.insert-left-path",
        "Insert left panel path into command",
        Global,
        &["Ctrl+BracketLeft", "Ctrl+Shift+BracketLeft"],
    );
    add(
        "command.insert-right-path",
        "Insert right panel path into command",
        Global,
        &["Ctrl+BracketRight", "Ctrl+Shift+BracketRight"],
    );
    add(
        "command.history-older",
        "Recall older command",
        Command,
        &["Alt+Up"],
    );
    add(
        "command.history-newer",
        "Recall newer command",
        Command,
        &["Alt+Down"],
    );
    add(
        "command.execute",
        "Execute command draft",
        Command,
        &["Enter"],
    );
    add(
        "command.focus-files",
        "Return to file list",
        Command,
        &["Escape"],
    );
    add(
        "drive.left",
        "Choose left panel volume",
        Global,
        &["Alt+F1"],
    );
    add(
        "drive.right",
        "Choose right panel volume",
        Global,
        &["Alt+F2"],
    );
    add("app.help", "Help", Global, &["F1"]);
    add("app.user-menu", "User menu", Global, &["F9"]);
    add("app.main-menu", "Main menu", Global, &["F10"]);
    add("app.options", "Options", Global, &[]);
    add("app.jobs", "Operation jobs", Global, &[]);
    add("app.about", "About Midnight Explorer", Global, &[]);
    add(
        "app.custom-commands",
        "Configure Custom Commands",
        Global,
        &[],
    );
    add("bookmark.manage", "Manage Hot Paths", FilesAny, &[]);
    add("view.brief", "Brief panel view", FilesAny, &["Alt+Digit2"]);
    add(
        "view.detailed",
        "Detailed panel view",
        FilesAny,
        &["Alt+Digit3"],
    );
    add(
        "view.thumbnails",
        "Thumbnail panel view",
        FilesAny,
        &["Alt+Digit5"],
    );
    add("app.exit", "Exit application", Global, &["Alt+F4"]);
    add("network.connect", "Connect network drive", Global, &["F11"]);
    add(
        "network.disconnect",
        "Disconnect network drive",
        Global,
        &["F12"],
    );
    for number in 1..=9 {
        add(
            &format!("tab.{number}"),
            &format!("Switch to tab {number}"),
            FilesAny,
            &[&format!("Ctrl+Shift+Digit{number}")],
        );
    }
    for slot in 1..=10 {
        let digit = slot % 10;
        add(
            &format!("bookmark.open.{slot}"),
            &format!("Open Hot Path {slot}"),
            FilesAny,
            &[
                &format!("Digit{digit}"),
                &format!("Ctrl+Digit{digit}"),
                &format!("Shift+Digit{digit}"),
            ],
        );
        add(
            &format!("bookmark.other.{slot}"),
            &format!("Open Hot Path {slot} in other panel"),
            FilesAny,
            &[
                &format!("Ctrl+Alt+Digit{digit}"),
                &format!("Alt+Shift+Digit{digit}"),
            ],
        );
    }
    for letter in 'A'..='Z' {
        add(
            &format!("drive.{}", letter.to_ascii_lowercase()),
            &format!("Switch to drive {letter}"),
            FilesAny,
            &[&format!("Shift+{letter}")],
        );
    }
    catalog
}
