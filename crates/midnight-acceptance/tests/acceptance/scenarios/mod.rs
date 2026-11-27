//! The scenario registry. Add a scenario by writing `fn(&mut Context) -> Result<(), String>`
//! in a module here and listing it in [`all`] (docs/developer/acceptance.md). A module
//! listed in [`OWNED`] registers its own scenarios (`pub fn scenarios() -> Vec<Scenario>`),
//! so they change without an edit here.
pub mod about_box;
pub mod checksums;
pub mod create_rename;
pub mod files_archive;
pub mod files_attributes;
pub mod files_busy;
pub mod files_clipboard;
pub mod files_conflicts;
pub mod files_create;
pub mod files_delete;
pub mod files_errors;
pub mod files_filter_search;
pub mod files_folder_dialogs;
pub mod files_jobs;
pub mod files_links;
pub mod files_links_volumes;
pub mod files_options;
pub mod files_panels;
pub mod files_rename;
pub mod files_selection;
pub mod files_transfers;
pub mod files_trash;
pub mod files_tree;
pub mod find;
pub mod find_actions;
pub mod find_archives;
pub mod find_budgets;
pub mod find_feed;
pub mod find_layout;
pub mod find_query;
pub mod find_roots;
pub mod find_support;
pub mod find_viewers;
pub mod harness_checks;
pub mod journeys_archive;
pub mod journeys_conflicts;
pub mod journeys_find;
pub mod journeys_jobs;
pub mod journeys_keyboard;
pub mod journeys_links;
pub mod journeys_options;
pub mod journeys_pointer;
pub mod journeys_support;
pub mod navigation;
pub mod options_appearance;
pub mod options_archives;
pub mod options_buttons;
pub mod options_colors;
pub mod options_comparators;
pub mod options_custom_commands;
pub mod options_editors;
pub mod options_general;
pub mod options_hot_paths;
pub mod options_keyboard;
pub mod options_palette;
pub mod options_previews;
pub mod options_settings_file;
pub mod options_themes;
pub mod options_visual;
pub mod panels_busy_ops;
pub mod panels_dialogs;
pub mod panels_drive_names;
pub mod panels_file_list;
pub mod panels_hot_digits;
pub mod panels_notices;
pub mod panels_paths;
pub mod panels_properties;
pub mod panels_root_keys;
pub mod panels_scrolling;
pub mod panels_search_filter;
pub mod panels_selection_more;
pub mod panels_support;
pub mod panels_tabs;
pub mod panels_times;
pub mod panels_tree_keys;
pub mod panels_view_sort;
pub mod shortcuts_catalog;
pub mod shortcuts_hints;
pub mod startup_paths;
pub mod tools_checksum_unlimited;
pub mod tools_checksums;
pub mod tools_compare;
pub mod tools_find;
pub mod tools_jobs;
pub mod tools_menu;
pub mod tools_notices;
pub mod tools_sizes;
pub mod tools_user_menu;
pub mod transfers;
pub mod tree;
pub mod ui_align;
pub mod ui_dialogs;
pub mod ui_gate;
pub mod ui_interact;
pub mod ui_red;
pub mod ui_states;
pub mod viewers_archives;
pub mod viewers_combos;
pub mod viewers_compare;
pub mod viewers_controls;
pub mod viewers_files;
pub mod viewers_office;
pub mod viewers_passwords;
pub mod viewers_presentation;
pub mod viewers_scroll;
pub mod viewers_selection;
pub mod viewers_thumbnails;
pub mod viewers_video;

use crate::{Context, Scenario, app};
use midnight_core::PaneId;
use std::time::Duration;

fn minutes(minutes: u64) -> Duration {
    Duration::from_secs(minutes * 60)
}

/// Modules that register their own scenarios; they run after those listed in [`all`], in
/// this order.
const OWNED: [fn() -> Vec<Scenario>; 9] = [
    panels_scrolling::scenarios,
    files_links_volumes::scenarios,
    tools_checksum_unlimited::scenarios,
    viewers_passwords::scenarios,
    viewers_office::scenarios,
    options_settings_file::scenarios,
    options_palette::scenarios,
    panels_properties::scenarios,
    panels_drive_names::scenarios,
];

pub fn all() -> &'static [Scenario] {
    static ALL: std::sync::OnceLock<Vec<Scenario>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = vec![
            Scenario {
                name: "startup",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Both panels open on the fixture root and list every top-level folder.",
                run: startup,
                timeout: minutes(2),
            },
            harness_checks::scenario(),
            Scenario {
                name: "startup_paths",
                isolation: crate::Isolation::ParallelStaged,
                area: "panels",
                summary: "Starts with --left and --right, with one of them and with neither against the real settings store in a private profile: ready within the settings deadline, explicit paths win, the other panel restores its saved tabs.",
                run: startup_paths::startup_paths,
                timeout: startup_paths::TIMEOUT,
            },
            Scenario {
                name: "navigation",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Enter/Backspace into every fixture folder, a level deeper and down the 12-level chain; each listing equals the disk.",
                run: navigation::every_folder,
                timeout: minutes(10),
            },
            Scenario {
                name: "copy_move_delete",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "F5 and F6 of a file and a folder, Shift+F8 of a file and a folder, confirmed in the dialogs and verified on disk.",
                run: transfers::copy_move_delete,
                timeout: minutes(5),
            },
            Scenario {
                name: "create_rename",
                isolation: crate::Isolation::Parallel,
                area: "operations",
                summary: "Shift+F4 new empty file (and the existing-name prompt), F7 new folder, F2 rename shown at once.",
                run: create_rename::create_rename,
                timeout: minutes(5),
            },
            Scenario {
                name: "tree_expand_all",
                isolation: crate::Isolation::Parallel,
                area: "tree",
                summary: "Ctrl+click on a folder expander expands the 12-level branch inline.",
                run: tree::expand_all,
                timeout: minutes(5),
            },
            Scenario {
                name: "folder_sizes",
                isolation: crate::Isolation::Parallel,
                area: "tree",
                summary: "Ctrl+Shift+S measures every fixture folder; each size equals the logical bytes on disk.",
                run: tree::folder_sizes,
                timeout: minutes(5),
            },
            Scenario {
                name: "find_by_mask",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Alt+F7 Find by mask returns exactly the matching files on disk; Go to file locates a result.",
                run: find::by_mask,
                timeout: minutes(5),
            },
            Scenario {
                name: "checksums",
                isolation: crate::Isolation::Parallel,
                area: "checksums",
                summary: "Ctrl+F8: calculate, save, replace and verify a manifest; verify the fixture's MD5, SFV, mismatch and missing manifests.",
                run: checksums::calculate_save_verify,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_copy_kinds",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "F5 of every fixture entry kind one at a time and per-folder multi-selections into subfolders and the deep tree with Preserve times and attributes (times and attributes checked); bytes, names and trees verified.",
                run: files_transfers::copy_every_kind,
                timeout: minutes(30),
            },
            Scenario {
                name: "files_move_kinds",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "F6 of every fixture entry kind one at a time and as multi-selections into subfolders; content, names, modes and times kept.",
                run: files_transfers::move_every_kind,
                timeout: minutes(30),
            },
            Scenario {
                name: "files_conflicts",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "Copy/Move onto existing names: Replace (original recovered), Skip, Copy as, Replace all, Skip all, Merge, Cancel job midway, same-folder copy, folder into itself refused.",
                run: files_conflicts::conflicts,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_links",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "Links under F5 and F6 through the dialog's Links choice (by keys, pointer and the open list): Keep links as links, proposed on a fixed local folder, recreates every link with the same target; Copy the files links point to follows folder and file links, and a dangling or cyclic link stays a link with a note; a self-linked folder copies once; F6 keeping links moves each link itself. Windows: junctions recreated as junctions, or followed.",
                run: files_links::links,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_delete",
                isolation: crate::Isolation::ParallelStaged,
                area: "operations",
                summary: "Shift+F8 of every entry kind, links (targets kept), read-only entries, a multi-selection and a 12-level tree; F8 Trash confirmation cancelled (staged process with a private Trash).",
                run: files_delete::delete_kinds,
                timeout: files_delete::TIMEOUT,
            },
            Scenario {
                name: "files_folder_dialogs",
                isolation: crate::Isolation::Parallel,
                area: "operations",
                summary: "Create directory and Rename folder at desktop and compact sizes, including keyboard focus, cancellation and dotted Unicode folder names.",
                run: files_folder_dialogs::folder_dialogs,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_rename",
                isolation: crate::Isolation::Parallel,
                area: "operations",
                summary: "F2: Unicode, case-only, collision (Skip, Replace), `/`, empty, 255/256-byte names, a folder; new name listed at once with the cursor on it.",
                run: files_rename::rename_cases,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_create",
                isolation: crate::Isolation::Parallel,
                area: "operations",
                summary: "F7 and Shift+F4: nested segments, a path above the panel, existing names, Escape, exact spaces.",
                run: files_create::create_cases,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_attributes",
                isolation: crate::Isolation::Parallel,
                area: "operations",
                summary: "Ctrl+F2: permission check boxes and octal, typed modified/accessed date, time and offset for a file, three files and a folder; verified on disk.",
                run: files_attributes::attributes_and_times,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_selection",
                isolation: crate::Isolation::Parallel,
                area: "selection",
                summary: "Insert, Space, Ctrl+A, Ctrl+Shift+A, Ctrl+D, Ctrl+I, Ctrl+Shift+I, Ctrl/Shift+click, select/deselect by mask (keypad +/−, F10), keypad * invert, F5 of a multi-selection.",
                run: files_selection::selection,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_sorting",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+F3/F4/F5/F6 twice each: name, extension, modified time and size in both directions, folders first.",
                run: files_panels::sorting,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_filter_search",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Show hidden files by Ctrl+H, View menu, F10 and Options (cursor, marks, Ctrl+A, status, inline tree, restart); Ctrl+F12 Masks and Expression filters, Show all; filename search with Down and Escape.",
                run: files_filter_search::filter_and_search,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_navigation",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Enter/Backspace, Alt+Left/Right history, filesystem root, typed paths, breadcrumb click, Ctrl+U swap, Properties, Ctrl+R refresh.",
                run: files_panels::navigation,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_tabs",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+T, Ctrl+Shift+Tab, click to switch, hover-only close button, + button, Ctrl+W, last tab kept.",
                run: files_panels::tabs,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_hot_paths",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+Shift+Plus adds a Hot Path, the toolbar menu opens it, Manage Hot Paths renumbers, removes and adds.",
                run: files_panels::hot_paths,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_file_list",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+M with a tab template: preview with expanded tabs, File output byte for byte, selection cleared.",
                run: files_panels::file_list,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_tree_ops",
                isolation: crate::Isolation::Parallel,
                area: "tree",
                summary: "Expand/collapse inline folders, then F5, F2 and Shift+F8 of a folder three levels deep in the tree.",
                run: files_tree::tree_operations,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_busy_folder",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "A writer churns the shown folder: unchanged rows, cursor and selection stay, no loading state; Rename and Copy listed at once.",
                run: files_busy::busy_folder,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_copy_options",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "The Copy/Move dialog: summary, destination, options and policy; the Links choice proposing Keep links as links on a fixed local folder, no former link check box; refused destinations; Preserve times and attributes for files and folders; full and size-and-samples verification from Options; many small files.",
                run: files_options::copy_options,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_transfer_jobs",
                isolation: crate::Isolation::Parallel,
                area: "jobs",
                summary: "Copy jobs side by side: Wait for earlier transfers off (runs at once) and on (waits for a paused job); Jobs states and Cancel job; totals, time remaining, speed chart and per-file progress; Cancel operation; wording.",
                run: files_jobs::transfer_jobs,
                timeout: minutes(15),
            },
            Scenario {
                name: "files_transfer_errors",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "Copy/Move errors: Retry after fixing a read-only destination or unreadable file, Skip all this error type, Cancel job, Skip of an unreadable folder, Retry of a refused Move, Decide later and Resolve, Merge all folders, error windows, another filesystem.",
                run: files_errors::transfer_errors,
                timeout: minutes(10),
            },
            Scenario {
                name: "files_clipboard_drag",
                isolation: crate::Isolation::Parallel,
                area: "transfers",
                summary: "Ctrl+C/Ctrl+X/Ctrl+V through an in-process clipboard, and drag and drop between the panels: plain (Move on one volume), Ctrl (Copy), onto a folder row, Ctrl+Shift refused.",
                run: files_clipboard::clipboard_and_drag,
                timeout: minutes(5),
            },
            Scenario {
                name: "files_archive_transfers",
                isolation: crate::Isolation::ParallelPlugins,
                area: "transfers",
                summary: "Archive panels as sources and targets: a file, a nested and an empty folder copied out of a ZIP, the same tree out of a tar.gz, F6 of a member refused, a native file into an archive panel refused.",
                run: files_archive::archive_transfers,
                timeout: files_archive::TIMEOUT,
            },
            Scenario {
                name: "files_trash",
                isolation: crate::Isolation::ParallelStaged,
                area: "operations",
                summary: "F8 into a private Trash: a file, a folder, a multi-selection, awkward names (.trashinfo, labels), a read-only file, a nested tree row; write-protected folders inside and outside the deleted tree; F8 where no Trash takes a file opens the permanent delete's confirmation with why (Windows: gated network or removable volumes only).",
                run: files_trash::trash,
                timeout: files_trash::TIMEOUT,
            },
            Scenario {
                name: "tools_compare_dirs",
                isolation: crate::Isolation::Parallel,
                area: "compare",
                summary: "Ctrl+F10 on identical, asymmetric, nested and unreadable trees with every option; differences selected, dialog closes at once, no result message.",
                run: tools_compare::compare_directories,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_find_criteria",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Alt+F7 masks, case, subfolders, regex, expressions (size, date, kind, attributes), content text/case/words/UTF-16, modified within, identical files; exact answers.",
                run: tools_find::find_criteria,
                timeout: minutes(10),
            },
            Scenario {
                name: "tools_find_folders_help",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Search folders tick boxes, Add, Browse and Remove; syntax help and examples; an invalid expression's error opens its details on click.",
                run: tools_find::find_folders_help_errors,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_find_results",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Large result sets (many/, the 100,000 limit), sorting, every column resized, Go to file, Stop on a running content search.",
                run: tools_find::find_results,
                timeout: minutes(15),
            },
            Scenario {
                name: "find_query_syntax",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Masks (;;, extensionless, exclusions, Unicode case), regex ((?i)), expressions (units, =~, not, offset times), content in every encoding, whole words, CRLF, Modified within units and validation, Identical files groups under every sort.",
                run: find_query::query_syntax,
                timeout: minutes(10),
            },
            Scenario {
                name: "find_roots_windows",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Several search folders (Enter, OK, re-tick, refused, missing, Remove, 16 max) with Subfolders on/off; the right panel; Ctrl+F, toolbar, menu; four windows; two searches at once; modeless; Escape order; Close while searching.",
                run: find_roots::roots_and_windows,
                timeout: minutes(10),
            },
            Scenario {
                name: "find_result_actions",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Every column sorted both ways, Name order as in panels, result keys, Go to file by Enter, double click and button (files, folders, Open), View/F3/Alt+F3, Edit/F4 with the editor, Open, stale results refused, Stop keeps results.",
                run: find_actions::result_actions,
                timeout: minutes(10),
            },
            Scenario {
                name: "find_feed_panel",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Feed to panel: order, title, status, keys, marks and bulk keys, clicks, Enter and double click, every way to leave, F3/Alt+F3/F4, Alt+Enter, F5/F6/F8/Shift+F8, refusals of the folder-only actions by key, toolbar, function bar and menu, every other action on the results (F2, attributes, checksums, file list, sizes, sorting, masks, filter, hidden, custom command); the right panel.",
                run: find_feed::feed_panel,
                timeout: minutes(10),
            },
            Scenario {
                name: "find_budgets",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "Budgets reached and named: 128 folder levels, a file over 1 GiB not read, 100,000 duplicate candidates; a 10,001-file identical group (over the former limit) listed whole.",
                run: find_budgets::budgets,
                timeout: minutes(15),
            },
            Scenario {
                name: "find_layout",
                isolation: crate::Isolation::Parallel,
                area: "find",
                summary: "The Find window in Blue, Dark and Touch at preferred, narrow and compact sizes: clean layout, narrow stacking, sideways columns and path tails, compact Query field and status placement, column drags that never sort.",
                run: find_layout::layout,
                timeout: minutes(8),
            },
            Scenario {
                name: "find_archives",
                isolation: crate::Isolation::ParallelPlugins,
                area: "find",
                summary: "Search inside archives through the installed plugin: members by name, content, expression and time; nested and broken archives; Go to file into the archive; members refused for View/Edit/Open and Feed; 301 archives; Stop; visited entries; an archive over 100,000 entries.",
                run: find_archives::archives,
                timeout: find_archives::TIMEOUT,
            },
            Scenario {
                name: "find_viewers",
                isolation: crate::Isolation::ParallelPlugins,
                area: "find",
                summary: "Viewers from Find through the installed viewers: View, F3 and Alt+F3 on results (only the selected file), F3 and Alt+F3 in a fed list with Space and Backspace walking its results.",
                run: find_viewers::viewers,
                timeout: find_viewers::TIMEOUT,
            },
            Scenario {
                name: "tools_checksum_algorithms",
                isolation: crate::Isolation::Parallel,
                area: "checksums",
                summary: "Ctrl+F8 with all five algorithms ticked, digests checked independently, a manifest per algorithm, Replace prompt Cancel, Include subfolders.",
                run: tools_checksums::checksum_algorithms,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_checksum_manifests",
                isolation: crate::Isolation::Parallel,
                area: "checksums",
                summary: "Verify preselected for BSD, GNU, SFV, sidecar and UTF-16 manifests with mismatches and missing files; a mixed manifest's error.",
                run: tools_checksums::checksum_manifests,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_checksum_large_stop",
                isolation: crate::Isolation::Parallel,
                area: "checksums",
                summary: "Progress and Stop while calculating the large file; nothing starts on its own.",
                run: tools_checksums::checksum_large_stop,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_folder_sizes",
                isolation: crate::Isolation::Parallel,
                area: "tree",
                summary: "Ctrl+Shift+S on marked links/, readonly/ and many/, inside links/, with an unreadable subfolder, and cancelled while running.",
                run: tools_sizes::folder_sizes_tools,
                timeout: minutes(10),
            },
            Scenario {
                name: "tools_jobs_copy",
                isolation: crate::Isolation::Parallel,
                area: "jobs",
                summary: "A long copy's progress window: Pause, Resume, Cancel operation; a completed copy; the Jobs list states and Clear all finished.",
                run: tools_jobs::jobs_copy_control,
                timeout: minutes(10),
            },
            Scenario {
                name: "tools_help_drive_info",
                isolation: crate::Isolation::Parallel,
                area: "tools",
                summary: "F1 keyboard help and Ctrl+F1 drive information open, show their content and close.",
                run: tools_menu::help_and_drive_info,
                timeout: minutes(3),
            },
            Scenario {
                name: "tools_main_menu",
                isolation: crate::Isolation::Parallel,
                area: "tools",
                summary: "F10: every main menu item opens what it says (cancelled) or has its effect (undone); Exit last.",
                run: tools_menu::main_menu,
                timeout: minutes(10),
            },
            Scenario {
                name: "ui_review",
                isolation: crate::Isolation::Parallel,
                area: "ui",
                summary: "Polish gate, default matrix: main window states, menus, popups, every dialog and tool window across themes, densities, sizes (minimum included), large fonts and scale 1 and 2; any flag fails.",
                run: crate::review::run,
                timeout: minutes(40),
            },
            Scenario {
                name: "viewers_files",
                isolation: crate::Isolation::timed_plugin(),
                area: "viewers",
                summary: "F3 on every viewable fixture file through the installed viewers: content shown in time (text, source colours, Markdown, HTML with local assets, PDF, image pixels, hex), clear errors for bad files, open latency recorded (and its spread over repeated opens); SVG features and refusals, two-frame GIF/WebP first frames, the licensed AVIF fixtures against their PNG reference.",
                run: viewers_files::every_kind,
                timeout: viewers_files::TIMEOUT,
            },
            Scenario {
                name: "viewers_controls",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Viewer status bar, Find with Aa and F3/Shift+F3, Wrap, the presentation row, text/binary modes, Alt+F3, keyboard and wheel scrolling, image zoom, PDF layout, next/previous file, Escape; the window's close request, a changed file reloaded on Space/Backspace, Space/Backspace within the window of a folder over 8 MiB of paths.",
                run: viewers_controls::controls,
                timeout: viewers_controls::TIMEOUT,
            },
            Scenario {
                name: "viewers_compare",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Compare marked files (Ctrl+Shift+C): identical, three differences navigated with N/P/F7/Alt+Home/Alt+End and the button, a 3.5 MiB log, two encodings, images in three modes; binaries in the hex view (summary, the same keys between regions); the 4.25 MiB log as text and with Hex; Escape and Close.",
                run: viewers_compare::compare,
                timeout: viewers_compare::TIMEOUT,
            },
            Scenario {
                name: "viewers_thumbnails",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Thumbnail view (Alt+5): visible thumbnails appear with their images' colours, no flicker, keyboard and wheel scrolling, cached on scrolling back; every image format and a PDF.",
                run: viewers_thumbnails::thumbnails,
                timeout: viewers_thumbnails::TIMEOUT,
            },
            Scenario {
                name: "viewers_video",
                // Playback uses the installed media helpers and system codecs in real time.
                isolation: crate::Isolation::Exclusive,
                area: "viewers",
                summary: "Video through the installed thumbnailers, as an installation runs its helpers: a plain and an NTFS-compressed generated .mp4 each show a thumbnail of their own colour (not the fallback icon), and F3 opens the first frame, seeks forth and back and plays to the end; a helper that cannot start fails with the viewer's sentence, never a system dialog.",
                run: viewers_video::video,
                timeout: viewers_video::TIMEOUT,
            },
            Scenario {
                name: "viewers_archives",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Archives through the installed plugins: ZIP, tar, tar.gz and empty ZIP listings equal their members; F5 copies a member out; F3 views members; a ZIP inside a ZIP opens (listing, path, F3, F5, Backspace out) and a text file named .zip gives a clear error.",
                run: viewers_archives::archives,
                timeout: viewers_archives::TIMEOUT,
            },
            Scenario {
                name: "viewers_scroll",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Scrolling in every viewer kind: scrollbar drags swept from 0 to 100 % land without an error (log, CSV, TSV, CSV as text, Markdown with tables, source, hex, PDF, image), a Slint pixel delta moves exactly its pixels everywhere; a native notch (acceptance seam) 100 px in every viewer kind, HTML included; the system's lines per notch, one screen per notch and reduced motion; PDF frames during a scrollbar drag and sharp text at a typed 400 %; source highlighting while scrolling (pending its fix), arrows, PageUp/PageDown, Ctrl+Home/End; the long line's horizontal bar.",
                run: viewers_scroll::scrolling,
                timeout: viewers_scroll::TIMEOUT,
            },
            Scenario {
                name: "viewers_selection",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Ctrl+A, mouse drag and Shift+click selections; Ctrl+C copies exactly the selected text (source without line numbers, rendered Markdown, hex rows); the selection and selected glyphs in the theme's own pair, which keeps 4.5:1 (7:1 Contrast, 12:1 High Contrast) in every theme, and the inactive pair when the window loses the focus (acceptance seam); strikethrough lines in the selected-text colour; unselected text 5:1 (7:1) (Rust, CSV columns; plain, Markdown, hex and five more languages in Blue, Dark and Light).",
                run: viewers_selection::selection,
                timeout: viewers_selection::TIMEOUT,
            },
            Scenario {
                name: "viewers_presentation",
                isolation: crate::Isolation::ParallelPlugins,
                area: "viewers",
                summary: "Find bar (Find button, Enter, F3, Shift+F3, Next/Previous buttons in order, Whole words, Aa, Clear, Escape from the field), Source/Text and Markdown/Text modes, Lines, Markdown presets, font size and family kept for the next file, PDF pages, image 1:1/Fit/drag, HTML zoom and policy boxes; typed zoom (Ctrl+Y, a click on the percentage; limits and hints; HTML 10–1000 %; a decimal-comma number format), the PDF layout kept for the next PDF, viewer preferences after a restart.",
                run: viewers_presentation::presentation,
                timeout: viewers_presentation::TIMEOUT,
            },
            Scenario {
                name: "viewers_combinations",
                isolation: crate::Isolation::timed_plugin(),
                area: "viewers",
                summary: "F3 from Find results and a fed list (Space through the results, Compare files with the focused result), F3 in an inline tree branch (Space in visible order), six viewer windows at once (closing one keeps the others), a previewed theme recolouring an open viewer, a viewer at twice the scale.",
                run: viewers_combos::combinations,
                timeout: viewers_combos::TIMEOUT,
            },
            Scenario {
                name: "options_general",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Options › General: every option with its visible effect; Cancel discards, OK saves, a restart keeps (the seeded history recalled first, a cleared one not), Reset to defaults restores (Hot Path and Custom Command kept); the page list by keyboard, the remembered page, Enter on a combo box.",
                run: options_general::general,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_appearance",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Options › Appearance: theme, spacing, font family and sizes, text rendering; preview, Cancel, OK, restart, Reset to defaults.",
                run: options_appearance::appearance,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_custom_commands",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Custom Commands: create, edit, drag, delete, separators, placeholders; grouped and per-item runs on disk; failure notice; preview; user-command key; restart. F9 lists the commands; right-click \"Custom Commands settings…\"; the list's Alt+Up/Alt+Down/Delete/Insert; Escape cancels a drag; a terminal command's dry run; a disabled command.",
                run: options_custom_commands::custom_commands,
                timeout: minutes(8),
            },
            Scenario {
                name: "options_keyboard",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Keyboard editor: rebind Create folder; new key works, old does not; the Replace question (Cancel, Enter on its focused Cancel, Escape; Replace by Tab and Enter then OK moves F5); Restore command; Restore all defaults (Keep, Reset); Clear; restart.",
                run: options_keyboard::keyboard,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_colors",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Colors: an added rule colours matching names; priority, Cancel, OK, restart; the colour picker and every rule field seen in the panel.",
                run: options_colors::colors,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_archives",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Archives: custom extension chips per format (add, fold, remove, invalid, built-in conflict), an association; Cancel, OK, restart.",
                run: options_archives::archives,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_editors",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Editors: a harmless custom fallback and *.md rule; F4 runs the matching one on disk; an invalid rule is refused; restart.",
                run: options_editors::editors,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_comparators",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Comparators: built-in text and external-program overrides saved and listed; an invalid expression is refused; restart.",
                run: options_comparators::comparators,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_previews_plugins",
                isolation: crate::Isolation::ParallelPlugins,
                area: "options",
                summary: "Previews: switches and thumbnail size seen in the Thumbnails view; Plugins sections (empty states asserted), Rescan and the Archives link; restart; then, staged with the installed packages, every section lists them and Rescan reports their counts.",
                run: options_previews::previews_plugins,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_hot_paths",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Hot Paths: add three, exchange numbers, remove one; Ctrl+digit opens them; restart.",
                run: options_hot_paths::hot_paths,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_visual",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Every Options page in Light and Dark and a narrow touch layout, with the UI review's layout checks.",
                run: options_visual::visual,
                timeout: minutes(8),
            },
            ui_red::scenario(),
            crate::review::themes_scenario(),
            crate::review::full_scenario(),
            ui_dialogs::scenario(),
            ui_states::jobs_scenario(),
            ui_states::viewers_scenario(),
            Scenario {
                name: "shortcuts_catalog",
                isolation: crate::Isolation::Parallel,
                area: "keyboard",
                summary: "Every default binding of the shortcut catalog, alternates included, pressed in its context: it does what its action says and nothing else; keys outside their context stay text.",
                run: shortcuts_catalog::catalog,
                timeout: minutes(20),
            },
            Scenario {
                name: "panels_dialogs",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Every panel dialog by key and pointer, closed by Escape, header × and Cancel/Close: nothing left open, no notice, keyboard back in the list, disk unchanged; each still works afterwards.",
                run: panels_dialogs::dialogs,
                timeout: minutes(10),
            },
            Scenario {
                name: "panels_notices",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "An unreadable folder at the end of a long path (Enter, double click, Ctrl+G, missing path, failed branch, touch density): the notice shows the whole path, Details, Copy details, every close.",
                run: panels_notices::notices,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_times",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+F2 Times: Up/Down steppers per part, calendar keys and buttons, time picker keys and ▲/▼, Now, Today; applied to the second; Escape closes a picker only; Ctrl+F7 lowercase.",
                run: panels_times::times,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_view_sort",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Sort by keys, header clicks and the sort menu with marks and cursor kept in view; view modes and their arrow keys; splitter drag, minimum, double click, keys, restart; Ctrl+F11; compact window; touch.",
                run: panels_view_sort::view_sort,
                timeout: minutes(8),
            },
            Scenario {
                name: "panels_paths",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Toolbar Back/Forward/Parent/Refresh and the history menus; Ctrl+PageUp/PageDown, Shift+Backspace, Ctrl+Backslash, double clicks, Tab; path segments (hover, right-click copy, paste), long-path marker, editor; drive menu; empty folder.",
                run: panels_paths::paths,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_tabs",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Middle click and the last tab, double click after the tabs, Ctrl+Shift+digit, scroll arrows and the active tab in view, the 32-tab limit, right-panel tabs, Ctrl+U, F7/Shift+F4/F2 in second tabs, restore after a restart.",
                run: panels_tabs::tabs_more,
                timeout: minutes(6),
            },
            Scenario {
                name: "panels_search_filter",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Search: match order keys, Backspace, Space, Insert, Unicode, survives sort and refresh, ends on navigation. Filter: funnel, Regex, case, indicator and its ×, history, Cancel, invalid regex; with Ctrl+A, sorting, search, hidden files, both panels.",
                run: panels_search_filter::search_filter,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_tree_keys",
                isolation: crate::Isolation::Parallel,
                area: "tree",
                summary: "Inline branches with keys: arrows/Home/End through nested rows, Insert/Space on nested folders, Ctrl+Shift+I/Ctrl+A/Ctrl+D across levels, branch order by name and time, collapse moves the cursor, Enter and Backspace, filter context rows.",
                run: panels_tree_keys::tree_keys,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_selection_more",
                isolation: crate::Isolation::Parallel,
                area: "selection",
                summary: "Ctrl+=, Ctrl++, Ctrl+− with remembered mask and folders, an invalid mask, Ctrl+Shift+click, Alt+Up/Down, last-row clamp, `..` never marked, rubber band (plain, Shift, Ctrl); the details strip's counts and bytes.",
                run: panels_selection_more::selection_more,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_hot_digits",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Hot Path digits as physical keys: plain, Shift, Ctrl, 0 for 10, Ctrl+Alt and Alt+Shift to the other panel, unassigned swallowed, text during search, Ctrl+Shift+0; the star, Add path, and both menus.",
                run: panels_hot_digits::hot_digits,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_busy_ops",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "While a writer churns the shown folder: F7, Shift+F4 and Ctrl+F2 succeed and are listed at once with nothing left open; marks and an expanded branch stay; the listing equals the disk afterwards.",
                run: panels_busy_ops::busy_ops,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_file_list",
                isolation: crate::Isolation::Parallel,
                area: "panels",
                summary: "Ctrl+M: Clipboard output, name parts, extension, size, `$$`, `:max` and <DIR>, Browse…, an existing file's Cancel, Replace and Append, the focused entry, a panel filter, the template history; byte for byte.",
                run: panels_file_list::file_list,
                timeout: minutes(5),
            },
            Scenario {
                name: "panels_root_keys",
                isolation: crate::Isolation::ParallelPlugins,
                area: "panels",
                summary: "Shift+Backspace (and Ctrl+Backspace) to the focused panel's root through the native plugin: left and right deep folders, after Escape from a search, after a mouse click, in a second tab, and inside an archive to the archive's root.",
                run: panels_root_keys::root_keys,
                timeout: panels_root_keys::TIMEOUT,
            },
            Scenario {
                name: "about_box",
                isolation: crate::Isolation::Parallel,
                area: "tools",
                summary: "About: version and spacing; Licenses filter, selection, hover mark, pointer cursor, View license, double-click, Enter, keys, detail, Back, Escape, reopen, drag, click outside, Close.",
                run: about_box::about,
                timeout: minutes(5),
            },
            Scenario {
                name: "shortcuts_footer",
                isolation: crate::Isolation::Parallel,
                area: "keyboard",
                summary: "The F1–F12 footer in the plain, Ctrl, Shift, Alt and Ctrl+Shift layers: each cell's click does what its key does, unassigned cells are disabled, network cells hidden on Linux.",
                run: shortcuts_catalog::footer,
                timeout: minutes(10),
            },
            Scenario {
                name: "tools_user_menu",
                isolation: crate::Isolation::Parallel,
                area: "tools",
                summary: "F9 User menu: items in order, each built-in item's effect, Tab/Enter/Escape, Custom Commands on marks and on a tree row (on disk), a command's output window with its exit code, a disabled command, the empty state's Set up.",
                run: tools_user_menu::user_menu,
                timeout: minutes(5),
            },
            Scenario {
                name: "tools_notices_jobs",
                isolation: crate::Isolation::Parallel,
                area: "tools",
                summary: "Notices: Details/Hide details, Copy details, Close, Escape, header close, nothing follows, a very long path; Jobs: rows and states, selection keys, Open/Enter, Copy details, Cancel job disabled, both Clear buttons, Close, Escape.",
                run: tools_notices::notices_and_jobs,
                timeout: minutes(5),
            },
            Scenario {
                name: "shortcuts_hints",
                isolation: crate::Isolation::Parallel,
                area: "keyboard",
                summary: "F1 help, the F10 menu and the panel's View and Sort menus show every catalog key; after rebinding Create folder and adding a key to Sort by size they follow at once.",
                run: shortcuts_hints::hints,
                timeout: minutes(5),
            },
            Scenario {
                name: "options_buttons",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Every enabled button on every Options page has a visible effect (text, control, popup, window, chooser or command); Escape discards; nothing is saved.",
                run: options_buttons::every_button,
                timeout: minutes(10),
            },
            Scenario {
                name: "options_themes",
                isolation: crate::Isolation::Parallel,
                area: "options",
                summary: "Both appearance menus preview every matching theme. OS changes, Cancel, OK and restart preserve the pair. Narrow and compact topic pickers support keyboard navigation.",
                run: options_themes::themes,
                timeout: minutes(8),
            },
            journeys_find::scenario(),
            journeys_archive::scenario(),
            journeys_conflicts::scenario(),
            journeys_jobs::scenario(),
            journeys_options::scenario(),
            journeys_links::scenario(),
            journeys_keyboard::scenario(),
            journeys_pointer::scenario(),
        ];
        for scenarios in OWNED {
            all.extend(scenarios());
        }
        all
    })
}

fn startup(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = app::App::start(app::Config::new(&root, &root));
    let expected = context.fixture.top_folders();
    for pane in [PaneId::Left, PaneId::Right] {
        let names = app.names(pane);
        let missing: Vec<&String> = expected
            .iter()
            .filter(|name| !names.contains(name))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "{pane:?} panel lacks {missing:?}; it shows {names:?}"
            ));
        }
    }
    app.screenshot(context, "fixture-root", "Both panels on the fixture root");
    app.shutdown()
}
