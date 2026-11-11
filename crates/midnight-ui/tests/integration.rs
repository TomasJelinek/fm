//! Shared software-UI integration target; each libtest test owns its Slint platform.
//! Register new integration test files here: Cargo automatic discovery is disabled.

// Support modules share code only; each test creates its own state and Slint platform.
#[cfg(windows)]
#[path = "support/windows_keys.rs"]
mod platform_keys;
#[cfg(not(windows))]
#[path = "support/default_keys.rs"]
mod platform_keys;
#[cfg(windows)]
#[path = "support/windows_raw_paths.rs"]
mod raw_paths;
#[cfg(unix)]
#[path = "support/unix_raw_paths.rs"]
mod raw_paths;
#[cfg(windows)]
#[path = "support/windows_search_keys.rs"]
mod search_keys;
#[path = "support/selection.rs"]
mod selection;
#[path = "support/ui.rs"]
mod ui;

mod bottom_bar;
mod bulk_selection;
mod command_bindings;
mod command_menus;
mod compact_controls;
mod compact_feedback;
mod detail_font;
mod draft_shell;
mod focus;
mod font_catalog;
mod fonts;
mod interaction_controls;
mod interaction_performance;
mod mouse;
mod mutation_controls;
mod native_controls;
mod navigation_bindings;
mod options;
mod panel_refinements;
mod panel_scroll;
mod panel_search;
mod panel_search_layout;
mod panel_search_render;
mod panel_view_modes;
mod salamander_shortcuts;
mod selection_bindings;
mod selection_feedback;
mod tabs_bookmarks;
mod tooltips;
