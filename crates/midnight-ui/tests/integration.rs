//! The single integration-test binary of `midnight-ui`.
//!
//! Every other `tests/*.rs` file is an independent module of this crate. Linking
//! one executable instead of one per file removes per-binary link and process
//! start-up cost; libtest still runs all tests in parallel. Cargo `autotests` is
//! disabled for this package, so `every_test_file_is_a_module` fails when a new
//! file is not declared below.
// Each module keeps its own copy of any other `support` file it includes, exactly
// as when every file was a separate test crate, so their types never mix.
#![allow(clippy::duplicate_mod)]

// Compiled once for the test modules that include them directly, instead of once per
// module. Support files that include `modeless.rs` themselves (`tree_filters`,
// `operation_form`, `job_windows`, `operations_native`) keep their own copy. Neither
// file holds process-wide state, so sharing them shares only their types. Nested, so
// that `every_test_file_is_a_module` does not take them for test files.
mod support {
    pub mod link_option;
    pub mod modeless;
    pub mod ui;
}

mod about_dialog;
mod action_button_states;
mod action_icon_colors;
mod archive_badges;
mod archive_options;
mod archive_path_bar;
mod archive_recovery;
mod argument_lists;
mod bottom_bar;
mod bulk_selection;
mod busy_folder_native;
mod checksum_polish;
mod checksum_windows;
mod color_picker;
mod color_picker_popup;
mod color_priority_list;
mod command_bindings;
mod command_line_layout;
mod command_menus;
mod commander_dialogs;
mod commander_edges;
mod compact_controls;
mod compact_feedback;
mod comparator_options;
mod comparator_transaction;
mod comparison_toolbar;
mod copy_dialog;
mod copy_errors;
mod delete_confirmation;
mod delete_job_resolve;
mod detail_font;
mod dialog_drag;
mod dialog_focus_return;
mod dialog_layout;
mod dialog_polish;
mod dialog_resize;
mod dialog_theme;
mod draft_shell;
mod drag_drop;
mod drag_integration;
mod drag_ordering;
mod drive_toolbar_scroll;
mod editor_options;
mod error_notice;
mod error_notices;
mod feed_panel_keys;
mod file_actions_ui;
mod file_list_actions;
mod file_list_window;
mod filename_colors;
mod find_actions;
mod find_control_geometry;
mod find_interactions;
mod find_narrow_layout;
mod find_result_paths;
mod find_window_controls;
mod find_windows;
mod focus;
mod font_catalog;
mod fonts;
mod function_bar;
mod hidden_icons;
mod icon_edges;
mod information_window_lifetime;
mod interaction_controls;
mod interaction_performance;
mod job_progress;
mod jobs_dialog;
mod keyboard_options;
mod list_scroll;
mod main_menu;
mod main_window_polish;
mod metadata_editor;
mod metadata_review;
mod metadata_times;
mod mouse;
mod mutation_controls;
mod native_controls;
mod native_filter_hotpath;
mod native_information_windows;
mod native_job_windows;
mod native_operation_layout;
mod native_operation_windows;
mod native_tool_layout;
mod navigation_bindings;
mod navigation_loading;
mod navigation_location_layout;
mod network_bookmarks;
mod network_navigation;
mod operation_defaults;
mod operations_native_ui;
mod options;
mod options_footer;
mod options_reflow;
mod options_settings_file;
mod options_transaction;
mod panel_activation;
mod panel_choices;
mod panel_chrome;
mod panel_filter_qualification;
mod panel_fit_and_empty;
mod panel_input_hold;
mod panel_refinements;
mod panel_row_layout;
mod panel_scroll;
mod panel_scroll_costs;
mod panel_search;
mod panel_search_layout;
mod panel_search_render;
mod panel_selection;
mod panel_toolbar;
mod panel_view_modes;
mod panel_warning_details;
mod parent_entry_icon;
mod parent_focus;
mod path_elision;
mod path_paste_ui;
mod permission_install;
mod plugin_options;
mod popup_model_updates;
mod recycle_bin;
mod salamander_shortcuts;
mod selected_navigation;
mod selection_bindings;
mod selection_feedback;
mod selection_rectangle_ui;
mod shared_toolbar;
mod shift_typed_search;
mod size_errors;
mod small_actions;
mod small_operation_actions;
mod small_window;
mod splitter_preferences;
mod status_bar;
mod tabs_bookmarks;
mod themes;
mod thumbnail_grid_rows;
mod thumbnail_options;
mod tool_reflow;
mod tool_window_failures;
mod tooltips;
mod tree_bulk_selection;
mod tree_checksums;
mod tree_clipboard;
mod tree_command_insertion;
mod tree_copy;
mod tree_disclosure;
mod tree_editor;
mod tree_filters;
mod tree_folder_sizes;
mod tree_metadata;
mod tree_move;
mod tree_panel;
mod tree_prefix_search;
mod tree_rename;
mod tree_user_command_process;
mod tree_user_commands;
mod tree_viewers;
mod tree_watch;
mod unix_permissions;
mod user_command_settings_ui;
mod user_command_toolbar;
mod user_menu_forms_ui;
mod volume_information;

#[test]
fn every_test_file_is_a_module() {
    use std::collections::BTreeSet;
    let declared: BTreeSet<String> = include_str!("integration.rs")
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .map(str::to_owned)
        .collect();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let files: BTreeSet<String> = std::fs::read_dir(directory)
        .expect("tests directory")
        .map(|entry| entry.expect("tests directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
        .filter(|stem| stem != "integration")
        .collect();
    assert_eq!(files, declared, "declare every tests/*.rs file as a module");
}
