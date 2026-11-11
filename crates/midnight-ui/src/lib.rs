//! In-memory presentation and a virtualized Slint shell. All I/O lives behind commands.
#![deny(unsafe_code)]
mod action_icons;
mod archive_options;
mod argument_rows;
mod checksum_windows;
mod color_options;
mod command_line;
mod commander_presentation;
mod copy_draft_windows;
mod copy_presentation;
mod detail_attributes;
mod dialog_focus;
mod dialog_host;
mod dialog_native;
mod dialog_presentation;
pub use dialog_native::configure_dialog_window_attributes;
mod decision_windows;
mod drag_bridge;
mod drag_lifecycle;
mod editor_options_presentation;
mod error_presentation;
mod file_key_bridge;
mod file_keys;
mod file_list_windows;
mod filename_colors;
mod find_windows;
mod font_model;
mod hot_path_windows;
mod information_windows;
mod job_presentation;
mod job_row_cache;
mod job_speed;
mod jobs_windows;
mod metadata_input;
mod metadata_presentation;
#[cfg(test)]
mod native_operation_tests;
mod native_path_edit;
mod native_shortcuts;
mod navigation_models;
mod operation_detail;
mod operation_presentation;
mod operation_windows;
mod panel_filter_windows;
mod panel_models;
mod panel_pointer;
mod panel_view_navigation;
mod path_paste;
mod path_paste_input;
mod picker_window;
pub use picker_window::native_path_picker_owner;
mod plugin_options;
mod presenter;
mod scroll_input;
mod search_input;
mod selection_bridge;
mod selection_rectangle;
mod settings_picker_presentation;
mod shell_bridge;
mod shortcut_bridge;
mod shortcut_editor;
mod shortcut_input;
mod size_presentation;
mod thumbnail_content;
mod thumbnail_presentation;
mod thumbnail_refresh;
pub use thumbnail_refresh::refresh_thumbnail_rows;
mod metadata_column_presentation;
mod time_presentation;
mod tool_window_failure;
mod tooltip;
mod user_command_presentation;
mod user_command_settings_presentation;
pub use thumbnail_content::ThumbnailContent;
pub use thumbnail_presentation::ThumbnailTarget;
mod drive_information_presentation;
mod volume_presentation;
use command_line::{command_directory_text, command_path_text, replace_command_selection};
use midnight_core::*;
pub use presenter::{DeferredDrop, Presenter, TabSummary};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub use midnight_ui_bindings::*;

struct Rows {
    presenter: Rc<RefCell<Presenter>>,
    pane: PaneId,
    notify: ModelNotify,
    structure: Cell<RowStructure>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RowStructure {
    navigation: u64,
    displayed: NavigationId,
    entries: usize,
    length: usize,
    parent: usize,
    streaming: bool,
    view: u64,
    tree_layout: Option<(TreeSessionId, TreeLayoutRevision)>,
}

impl RowStructure {
    fn from_pane(pane: &presenter::Pane) -> Self {
        Self {
            navigation: pane.generation,
            displayed: pane.displayed_generation(),
            entries: pane.rows().as_ptr() as usize,
            length: pane.display_len(),
            parent: pane.parent_offset(),
            streaming: pane.showing_stream(),
            view: pane.search.view_revision,
            tree_layout: pane
                .tree_view()
                .map(|view| (view.stamp.session, view.layout_revision)),
        }
    }
}

impl Rows {
    fn new(presenter: Rc<RefCell<Presenter>>, pane: PaneId) -> Self {
        let structure = RowStructure::from_pane(&presenter.borrow().panes[pane.index()]);
        Self {
            presenter,
            pane,
            notify: ModelNotify::default(),
            structure: Cell::new(structure),
        }
    }

    fn refresh(&self, pane: &presenter::Pane, first: usize, end: usize, data_changed: bool) {
        self.refresh_structure(RowStructure::from_pane(pane), first, end, data_changed);
    }

    fn refresh_structure(
        &self,
        structure: RowStructure,
        first: usize,
        end: usize,
        data_changed: bool,
    ) {
        if self.structure.replace(structure) != structure {
            // New identities or ordering deliberately invalidate an in-flight
            // pointer gesture. Ordinary acknowledgments must keep delegates alive.
            self.notify.reset();
        } else if data_changed {
            let length = structure.length + structure.parent;
            // Rows are virtualized: offscreen delegates obtain fresh row_data
            // when they enter the viewport. Never scan a complete listing here.
            for row in first.min(length)..end.min(length) {
                self.notify.row_changed(row);
            }
        }
    }
}
impl Model for Rows {
    type Data = FileRow;
    fn row_count(&self) -> usize {
        let state = self.presenter.borrow();
        let pane = &state.panes[self.pane.index()];
        pane.display_len() + pane.parent_offset()
    }
    fn row_data(&self, index: usize) -> Option<FileRow> {
        let state = self.presenter.borrow();
        let pane = &state.panes[self.pane.index()];
        if pane.has_parent() && index == 0 {
            return Some(FileRow {
                identity: format!(
                    "{}:{}:parent",
                    pane.generation,
                    pane.displayed_generation().0
                )
                .into(),
                name: "..".into(),
                parent_entry: true,
                prefix_end: 0,
                compressed: false,
                browsable_archive: false,
                size: "".into(),
                size_state: 0,
                size_hint: "Parent folder".into(),
                kind: "Parent folder".into(),
                icon: state.desktop.image(&IconKey::action(ActionIcon::Parent)),
                directory: true,
                selected: false,
                cursor: pane.parent_cursor,
                ..Default::default()
            });
        }
        let index = index.checked_sub(pane.parent_offset())?;
        let entry = pane.display_entry(index)?;
        let metadata = if pane.showing_stream() {
            pane.metadata.get(&entry.id).unwrap_or(&entry.metadata)
        } else {
            &entry.metadata
        };
        let directory = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
        let (size, size_state, size_hint) = if directory {
            size_presentation::folder_size(
                pane.folder_size(entry),
                pane.folder_size_running(entry),
                pane.size_scan_attempted && pane.tree_view().is_none(),
                state.preferences.values.size_display,
                &state.number_locale,
            )
        } else {
            (
                metadata
                    .size
                    .map(|bytes| {
                        size_presentation::file_size(
                            bytes,
                            state.preferences.values.size_display,
                            &state.number_locale,
                        )
                    })
                    .unwrap_or_else(|| "—".into()),
                0,
                "File length from backend metadata.".into(),
            )
        };
        let (modified_date, modified_time) = time_presentation::modified(metadata)
            .map(|value| (value.date, value.time))
            .unwrap_or_else(|| ("—".into(), "—".into()));
        let kind = match entry.kind {
            EntryKind::Directory => "Folder".into(),
            EntryKind::DirectoryLink => "Folder link".into(),
            EntryKind::Symlink => "Link".into(),
            EntryKind::Other => "Other".into(),
            EntryKind::File => entry
                .path
                .extension()
                .map(|ext| ext.to_string_lossy().to_uppercase())
                .unwrap_or_else(|| "File".into()),
        };
        let filename_style = state.filename_style(self.pane, entry);
        Some(FileRow {
            // Include intent and committed generations: a pending refresh may keep
            // the old rows visible, and a provider may recycle IDs at its commit.
            identity: pane.row_identity(entry).into(),
            name: entry.display_name.clone().into(),
            parent_entry: false,
            tree_depth: pane.tree_row(index).map_or(0, |row| row.depth as i32),
            tree_disclosure: pane.tree_row(index).map_or_else(
                || pane.root_presence_disclosure(entry),
                |row| match row.disclosure {
                    TreeDisclosure::None => 0,
                    TreeDisclosure::Unknown => 1,
                    TreeDisclosure::Loading => 2,
                    TreeDisclosure::Collapsed => 3,
                    TreeDisclosure::Expanded => 4,
                    TreeDisclosure::Empty => 5,
                    TreeDisclosure::Failed => 6,
                },
            ),
            tree_error: pane
                .tree_row(index)
                .map_or_else(
                    || pane.root_presence_error(entry.id),
                    |row| row.error.as_deref().unwrap_or_default(),
                )
                .into(),
            tree_expanded: pane.tree_row(index).is_some_and(|row| row.expanded),
            prefix_end: i32::try_from(pane.search_prefix_end(index)).unwrap_or(0),
            compressed: metadata.compression == CompressionStatus::Compressed,
            browsable_archive: state.can_browse_archive(entry),
            dim_icon: detail_attributes::dim_icon(metadata),
            size: size.into(),
            size_state,
            size_hint: size_hint.into(),
            warning_revision: if size_state == 3 {
                pane.folder_warning_revision(entry).into()
            } else {
                Default::default()
            },
            modified_date: modified_date.into(),
            modified_time: modified_time.into(),
            kind: kind.into(),
            icon: state.thumbnail_image(self.pane, entry).unwrap_or_else(|| {
                state
                    .desktop
                    .entry_image(&IconKey::for_entry(entry), state.can_browse_archive(entry))
            }),
            directory,
            selected: pane.selection_visible(entry),
            cursor: pane.cursor == Some(entry.id),
            color_rule: filename_style.is_some(),
            custom_foreground: filename_style.is_some_and(|style| style.foreground.is_some()),
            foreground: color_options::color(
                filename_style
                    .and_then(|style| style.foreground)
                    .unwrap_or_default(),
            ),
            custom_background: filename_style.is_some_and(|style| style.background.is_some()),
            background: color_options::color(
                filename_style
                    .and_then(|style| style.background)
                    .unwrap_or_default(),
            ),
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn format_size(bytes: u64, locale: &NumberLocale) -> String {
    for (unit, divisor) in [
        ("TiB", 1u64 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ] {
        if bytes >= divisor {
            let tenths = (u128::from(bytes) * 10 + u128::from(divisor) / 2) / u128::from(divisor);
            return format!(
                "{}{}{} {unit}",
                locale.format_u64((tenths / 10) as u64),
                locale.decimal_separator,
                tenths % 10
            );
        }
    }
    format!("{} B", locale.format_u64(bytes))
}
fn pane(id: i32) -> PaneId {
    if id == 0 { PaneId::Left } else { PaneId::Right }
}
fn column(id: i32) -> SortColumn {
    match id {
        1 => SortColumn::Size,
        2 => SortColumn::Modified,
        3 => SortColumn::Type,
        _ => SortColumn::Name,
    }
}
fn column_index(column: SortColumn) -> i32 {
    match column {
        SortColumn::Name => 0,
        SortColumn::Size => 1,
        SortColumn::Modified => 2,
        SortColumn::Type => 3,
    }
}

/// Refresh visible properties and lazy model notifications from current presenter state.
/// Release mutable presenter borrows first: Slint can synchronously read its row models.
pub fn refresh(window: &MainWindow, presenter: &Presenter) {
    user_command_presentation::refresh(window, presenter);
    refresh_properties(window, presenter, true);
}

fn refresh_properties(window: &MainWindow, presenter: &Presenter, reset_rows: bool) {
    // Native tools copy the owner appearance below; publish this revision first.
    window.invoke_apply_fonts(
        presenter.preferences.values.font_family.as_str().into(),
        i32::from(presenter.preferences.values.desktop_font_size),
        i32::from(presenter.preferences.values.touch_font_size),
    );
    window.invoke_apply_preferences(
        presenter.preferences.theme_index(),
        presenter.preferences.density_index(),
    );
    window.set_left_native_filesystem(presenter.panes[0].native_capabilities());
    window.set_right_native_filesystem(presenter.panes[1].native_capabilities());
    window.set_left_can_measure_folders(presenter.panes[0].can_measure_folders());
    window.set_right_can_measure_folders(presenter.panes[1].can_measure_folders());
    user_command_settings_presentation::refresh(window, presenter);
    editor_options_presentation::refresh(window, presenter);
    window.invoke_refresh_find_windows();
    window.invoke_refresh_checksum_windows();
    window.invoke_refresh_file_list_window();
    window.invoke_refresh_file_list_viewer();
    window.invoke_refresh_warning_viewer();
    archive_options::refresh(window, presenter);
    color_options::refresh(window, presenter);
    plugin_options::refresh(window, presenter);
    drag_bridge::refresh(window, presenter);
    commander_presentation::refresh(window, presenter);
    metadata_presentation::refresh(window, presenter);
    shortcut_bridge::refresh_bindings(window, presenter);
    shell_bridge::update(window, presenter);
    operation_presentation::refresh(window, presenter);
    window.invoke_refresh_operation_windows();
    window.set_action_icons(action_icons::palette(presenter));
    panel_models::refresh(window, presenter);
    let mut choices = navigation_models::ordered_choices(presenter);
    window.set_network_discovery_available(presenter.network_available);
    let locations = navigation_models::toolbar_choices(presenter, &choices);
    let selected_key = |pane: PaneId| {
        if presenter.panes[pane.index()].visible_storage_kind() == StorageKind::Network {
            return Some(SharedString::from(presenter.network_choice_token()));
        }
        usize::try_from(presenter.navigation_choice_index(pane))
            .ok()
            .map(|index| SharedString::from(presenter.navigation_choice_token(index)))
    };
    let left_key = selected_key(PaneId::Left);
    let right_key = selected_key(PaneId::Right);
    let hot_paths = panel_models::bookmark_choices(presenter);
    choices.extend(hot_paths.iter().cloned());
    // Hot Paths retain their explicit numbering and order after system locations.
    let as_other_key = presenter.shortcut_key("panel.as-other");
    choices.push(NavigationChoiceRow {
        identity: "other-panel".into(),
        label: if as_other_key.is_empty() {
            "As Other Panel".into()
        } else {
            format!("{as_other_key} As Other Panel").into()
        },
        compact_label: "As Other Panel".into(),
        tooltip: if as_other_key.is_empty() {
            "Open the other panel's current folder.".into()
        } else {
            format!(
                "Open the other panel's current folder ({}).",
                presenter.shortcut_hint("panel.as-other")
            )
            .into()
        },
        icon: presenter
            .desktop
            .image(&IconKey::action(ActionIcon::OpenPath)),
        kind: LocationRole::Documents,
    });
    let position = |key: Option<SharedString>| {
        key.and_then(|key| choices.iter().position(|choice| choice.identity == key))
            .map_or(-1, |index| index as i32)
    };
    let left_choice = position(left_key);
    let right_choice = position(right_key);
    navigation_models::update(
        window.get_navigation_choices(),
        choices.clone(),
        |choice| choice.identity.clone(),
        |model| window.set_navigation_choices(model),
    );
    window.set_navigation_revision(presenter.navigation_choices_revision() as i32);
    window.set_left_navigation_index(left_choice);
    window.set_right_navigation_index(right_choice);
    let tab_rows = |pane| {
        presenter
            .tabs(pane)
            .into_iter()
            .map(|tab| TabRow {
                identity: tab.id.0.to_string().into(),
                label: tab.title.into(),
                path: tab.path.to_string_lossy().into_owned().into(),
                active: tab.active,
            })
            .collect::<Vec<_>>()
    };
    update_model(window.get_left_tabs(), tab_rows(PaneId::Left), |model| {
        window.set_left_tabs(model)
    });
    update_model(window.get_right_tabs(), tab_rows(PaneId::Right), |model| {
        window.set_right_tabs(model)
    });
    update_model(
        window.get_bookmarks(),
        presenter
            .bookmarks()
            .iter()
            .map(|bookmark| BookmarkRow {
                identity: bookmark.id.0.to_string().into(),
                label: bookmark.name.as_str().into(),
                path: bookmark.display_text().into(),
                slot: i32::from(bookmark.slot.unwrap_or(0)),
            })
            .collect(),
        |model| window.set_bookmarks(model),
    );
    let bookmark_name = |pane: PaneId| {
        let path = presenter.panes[pane.index()].display_path();
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        if name.len() > midnight_core::MAX_BOOKMARK_NAME_BYTES
            || name.trim().is_empty()
            || name.chars().any(char::is_control)
        {
            SharedString::from("Hot Path")
        } else {
            SharedString::from(name.as_ref())
        }
    };
    window.set_left_bookmark_name(bookmark_name(PaneId::Left));
    window.set_right_bookmark_name(bookmark_name(PaneId::Right));
    window.invoke_refresh_panel_filter_window();
    window.invoke_refresh_hot_paths_window();
    let volume_icons = presenter
        .volumes
        .iter()
        .map(|volume| {
            presenter.desktop.image(&if volume.id == "home" {
                IconKey::folder()
            } else {
                IconKey::for_drive(&volume.root)
            })
        })
        .collect();
    update_model(window.get_volume_icons(), volume_icons, |model| {
        window.set_volume_icons(model)
    });
    if let Some(fonts) = window
        .get_font_choices()
        .as_any()
        .downcast_ref::<font_model::FontRows>()
    {
        window.set_font_index(fonts.refresh(presenter));
    }
    window.set_font_list_ready(presenter.fonts_ready());
    window.set_restored_panel_ratio(
        f32::from(presenter.preferences.values.panel_split_basis_points) / 10_000.0,
    );
    window.set_desktop_feedback_important(
        presenter.desktop.terminal_guidance.is_some() || !presenter.desktop.error.is_empty(),
    );
    window.set_preferences_unsaved(presenter.preferences.unsaved);
    window.set_preferences_saving(presenter.preferences.saving);
    window.set_preferences_ready(presenter.window_geometry_ready());
    window.set_preferences_save_status(presenter.preferences_save_status().into());
    window.set_preferences_save_failed(!presenter.preferences.save_error.is_empty());
    let options = &presenter.preferences.options;
    window.set_options_busy(options.accepting || options.saving);
    window.set_options_committing(options.saving);
    let feedback = options.feedback_revision.to_string();
    if window.get_options_feedback_revision() != feedback {
        window.set_options_feedback_revision(feedback.into());
        if let Some(topic) = options.error_topic {
            window.set_options_topic_index(topic);
        }
    }
    window.set_options_open(options.open);
    window.set_editor_default_label(presenter.editor.default_label.as_str().into());
    window.set_editor_executable(
        presenter
            .preferences
            .values
            .editor
            .executable
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into(),
    );
    window.set_editor_arguments(
        serde_json::to_string(&presenter.preferences.values.editor.arguments)
            .unwrap_or_default()
            .into(),
    );
    window.set_editor_revision(
        format!(
            "{}:{}",
            presenter.preferences.revision.0, presenter.editor.revision
        )
        .into(),
    );
    window.set_editor_error(presenter.editor.error.as_str().into());
    let mode_index = |mode| match mode {
        PanelViewMode::Detailed => 0,
        PanelViewMode::Brief => 1,
        PanelViewMode::Thumbnails => 2,
    };
    window.set_left_view_mode(mode_index(presenter.preferences.values.panel_view_modes[0]));
    window.set_right_view_mode(mode_index(presenter.preferences.values.panel_view_modes[1]));
    window.set_thumbnails_enabled(presenter.preferences.values.thumbnails_enabled);
    window.set_thumbnails_network(presenter.preferences.values.thumbnails_network);
    window.set_thumbnail_size(i32::from(presenter.preferences.values.thumbnail_size));
    window.set_quick_search_filter(presenter.preferences.values.quick_search_filter);
    window.set_active_pane(presenter.active.index() as i32);
    window.set_volumes_revision(presenter.volumes_revision);
    // Capture presentation before a pending listing can shrink the virtualized model.
    // Only a terminal backend result for this pane can finish this snapshot.
    let was_loading = [window.get_left_loading(), window.get_right_loading()];
    for (index, loading) in was_loading.into_iter().enumerate() {
        if !loading && presenter.panes[index].loading {
            window.invoke_remember_navigation_viewport(index as i32);
        }
    }
    let mut reveal = [false; 2];
    for (index, model) in [window.get_left_rows(), window.get_right_rows()]
        .iter()
        .enumerate()
    {
        let p = &presenter.panes[index];
        let context = p.search.input_context.0.to_string().into();
        let query = p.search.query.as_str().into();
        let matches = i32::try_from(p.search_match_count()).unwrap_or(i32::MAX);
        if index == 0 {
            if window.get_left_search_context() != context {
                window.invoke_reset_files_input(0);
            }
            window.set_left_search_context(context);
            window.set_left_search_query(query);
            window.set_left_search_pending(p.search.pending);
            window.set_left_search_count(matches);
        } else {
            if window.get_right_search_context() != context {
                window.invoke_reset_files_input(1);
            }
            window.set_right_search_context(context);
            window.set_right_search_query(query);
            window.set_right_search_pending(p.search.pending);
            window.set_right_search_count(matches);
        }
        if let Some(rows) = model.as_any().downcast_ref::<Rows>() {
            let (first, end) = if index == 0 {
                (
                    window.get_left_visible_row_start(),
                    window.get_left_visible_row_end(),
                )
            } else {
                (
                    window.get_right_visible_row_start(),
                    window.get_right_visible_row_end(),
                )
            };
            rows.refresh(p, first.max(0) as usize, end.max(0) as usize, reset_rows);
        }
        let location = p.display_location_text();
        let cursor = p.cursor_index().map_or(-1, |i| i as i32);
        reveal[index] = !p.preserve_viewport
            && if index == 0 {
                window.get_left_path().as_str() != location || window.get_left_cursor() != cursor
            } else {
                window.get_right_path().as_str() != location || window.get_right_cursor() != cursor
            };
        let volume = presenter
            .volumes
            .iter()
            .enumerate()
            .filter(|(_, v)| p.display_path().starts_with(&v.root))
            .max_by_key(|(_, v)| v.root.components().count())
            .map_or(-1, |(i, _)| i as i32);
        let mut status = format!(
            "{} items  ·  {} selected",
            p.tree_view()
                .map_or_else(|| p.rows().len(), |view| view.rows.len()),
            p.selected_count()
        );
        if p.size_scan.is_some() {
            status.push_str("  ·  Measuring folders…");
            let keys = presenter.shortcut_hint("panel.folder-sizes");
            if !keys.is_empty() {
                status.push_str(&format!(" ({keys} cancels)"));
            }
        }
        if let Some(space) = p.visible_space() {
            status.push_str(&format!(
                "  ·  {} free / {}",
                format_size(space.available, &presenter.number_locale),
                format_size(space.total, &presenter.number_locale)
            ));
        }
        let error = if p.error.is_empty() {
            &presenter.volumes_error
        } else {
            &p.error
        };
        if index == 0 {
            window.set_left_path(location.into());
            window.set_left_volume(volume);
            window.set_left_status(status.into());
            window.set_left_can_elevate(presenter.can_retry_directory_elevated(PaneId::Left));
            window.set_left_error(error.into());
            window.set_left_error_revision(p.error_revision().into());
            window.set_left_loading(p.loading);
            window.set_left_cursor(p.cursor_index().map_or(-1, |i| i as i32));
            window.set_left_sort(column_index(p.sort.column));
            window.set_left_descending(p.sort.descending);
        } else {
            window.set_right_path(location.into());
            window.set_right_volume(volume);
            window.set_right_status(status.into());
            window.set_right_can_elevate(presenter.can_retry_directory_elevated(PaneId::Right));
            window.set_right_error(error.into());
            window.set_right_error_revision(p.error_revision().into());
            window.set_right_loading(p.loading);
            window.set_right_cursor(p.cursor_index().map_or(-1, |i| i as i32));
            window.set_right_sort(column_index(p.sort.column));
            window.set_right_descending(p.sort.descending);
        }
    }
    navigation_models::update(
        window.get_left_locations(),
        navigation_models::locations(
            &locations,
            usize::try_from(left_choice)
                .ok()
                .and_then(|index| choices.get(index))
                .map(|row| row.identity.as_str()),
        ),
        |location| location.identity.clone(),
        |model| window.set_left_locations(model),
    );
    navigation_models::update(
        window.get_right_locations(),
        navigation_models::locations(
            &locations,
            usize::try_from(right_choice)
                .ok()
                .and_then(|index| choices.get(index))
                .map(|row| row.identity.as_str()),
        ),
        |location| location.identity.clone(),
        |model| window.set_right_locations(model),
    );
    window.set_terminal_ready(
        presenter.desktop.shell.is_some()
            && presenter.desktop.terminal_pending.is_none()
            && presenter.panes[presenter.active.index()].native_capabilities(),
    );
    window.set_terminal_available(presenter.desktop.shell.is_some());
    window.set_windows_shells(matches!(
        presenter.desktop.shell,
        Some(ShellFlavor::PowerShell | ShellFlavor::Cmd)
    ));
    let terminal_choices = presenter.terminal_choices();
    let previous_choices = window.get_terminal_choices();
    if previous_choices.row_count() != terminal_choices.len()
        || terminal_choices
            .iter()
            .enumerate()
            .any(|(index, label)| previous_choices.row_data(index).as_deref() != Some(*label))
    {
        window.set_terminal_choices(
            Rc::new(VecModel::from(
                terminal_choices
                    .iter()
                    .map(|label| SharedString::from(*label))
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
    }
    window.set_terminal_index(presenter.terminal_index());
    window.set_command_hint(presenter.terminal_hint().into());
    window.set_desktop_status(
        if let Some(guidance) = presenter.desktop.terminal_guidance {
            guidance.into()
        } else if presenter.desktop.error.is_empty() {
            presenter.desktop.status.clone()
        } else {
            presenter.desktop.error.clone()
        }
        .into(),
    );
    if let Some((title, body)) = &presenter.desktop.notice {
        window.set_message_title(title.clone().into());
        window.set_message_body(body.clone().into());
        window.set_message_error(presenter.desktop.notice_error);
        window.set_message_generation(presenter.desktop.notice_generation);
    }
    window.invoke_refresh_information_windows();
    window.invoke_refresh_jobs_window(false);
    window.invoke_refresh_decision_windows();
    let labels: Vec<SharedString> = presenter
        .volumes
        .iter()
        .map(|v| {
            volume_presentation::menu_label(
                v,
                volume_presentation::details(presenter, v),
                &presenter.number_locale,
            )
            .into()
        })
        .collect();
    update_strings(window.get_volumes(), labels, |model| {
        window.set_volumes(model)
    });
    update_strings(
        window.get_volume_short_labels(),
        presenter
            .volumes
            .iter()
            .map(|v| SharedString::from(v.label.as_str()))
            .collect(),
        |model| window.set_volume_short_labels(model),
    );
    update_strings(
        window.get_volume_tooltips(),
        presenter
            .volumes
            .iter()
            .map(|v| {
                SharedString::from(volume_presentation::volume_tooltip(
                    v,
                    volume_presentation::details(presenter, v),
                    &presenter.number_locale,
                ))
            })
            .collect(),
        |model| window.set_volume_tooltips(model),
    );
    if reveal[0] {
        window.invoke_reveal_left();
    }
    if reveal[1] {
        window.invoke_reveal_right();
    }
    for (index, loading) in was_loading.into_iter().enumerate() {
        let pane = &presenter.panes[index];
        if loading && !pane.loading {
            window.invoke_finish_navigation_viewport(index as i32, !pane.error.is_empty());
        }
    }
    tooltip::synchronize(window);
}

fn update_strings(
    current: ModelRc<SharedString>,
    values: Vec<SharedString>,
    set: impl FnOnce(ModelRc<SharedString>),
) {
    if current.row_count() != values.len()
        || values
            .iter()
            .enumerate()
            .any(|(index, value)| current.row_data(index).as_ref() != Some(value))
    {
        set(Rc::new(VecModel::from(values)).into());
    }
}

fn update_model<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    values: Vec<T>,
    set: impl FnOnce(ModelRc<T>),
) {
    if current.row_count() != values.len()
        || values
            .iter()
            .enumerate()
            .any(|(index, value)| current.row_data(index).as_ref() != Some(value))
    {
        set(Rc::new(VecModel::from(values)).into());
    }
}

/// Register default application keys and cached hints on a standalone Slint window.
///
/// [`install`] includes this initializer. UI-only hosts that attach individual
/// callbacks can use it without a presenter or backend; native editors retain focus.
pub fn install_file_keys(window: &MainWindow) {
    filename_colors::install(window);
    job_presentation::install(window);
    shortcut_bridge::install_default(window);
    tooltip::install(window).expect("install UI event observer after Slint initialization");
}

/// Register UI callbacks and request the initial locations. The sender must be nonblocking.
pub fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let owner = dialog_host::DialogOwner::new(window);
    let weak = window.as_weak();
    owner.set_input_gate(move |blocked| {
        if let Some(window) = weak.upgrade() {
            window.set_dialog_input_blocked(blocked);
            if blocked {
                window.invoke_cancel_function_press();
                window.invoke_release_function_press();
                window.set_scroll_interaction_active(false);
            }
        }
    });
    let weak = window.as_weak();
    owner.set_focus_fallback(move || {
        if let Some(window) = weak.upgrade() {
            window.invoke_focus_files();
        }
    });
    information_windows::install(window, presenter.clone(), send.clone());
    shortcut_bridge::install(window, presenter.clone(), send.clone());
    path_paste::install(window, presenter.clone(), send.clone());
    find_windows::install(window, presenter.clone(), send.clone());
    checksum_windows::install(window, presenter.clone(), send.clone());
    file_list_windows::install(window, presenter.clone(), send.clone());
    user_command_presentation::install(window, presenter.clone(), send.clone());
    user_command_settings_presentation::install(window, presenter.clone(), send.clone());
    editor_options_presentation::install(window, presenter.clone(), send.clone());
    job_presentation::install(window);
    plugin_options::install(window, presenter.clone(), send.clone());
    archive_options::install(window, presenter.clone(), send.clone());
    color_options::install(window, presenter.clone(), send.clone());
    commander_presentation::bind(window, &presenter, &send);
    panel_filter_windows::install(window, presenter.clone(), send.clone());
    hot_path_windows::install(window, presenter.clone(), send.clone());
    metadata_presentation::bind(window, &presenter, &send);
    tooltip::install(window).expect("install tooltip event observer after Slint initialization");
    scroll_input::install(window);
    shell_bridge::install(window, presenter.clone(), send.clone());
    copy_presentation::install(window, presenter.clone(), send.clone());
    operation_windows::install(window, presenter.clone(), send.clone());
    jobs_windows::install(window, presenter.clone(), send.clone());
    decision_windows::install(window, presenter.clone(), send.clone());
    drag_bridge::install(window, presenter.clone(), send.clone());
    selection_bridge::install(window, presenter.clone(), send.clone());
    window.set_font_choices(Rc::new(font_model::FontRows::new(presenter.clone())).into());
    window.set_left_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Left)));
    window.set_right_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Right)));

    macro_rules! callback {
        ($name:ident, |$w:ident, $p:ident, $s:ident $(, $arg:ident)*| $body:block) => {
            callback!($name, true, |$w, $p, $s $(, $arg)*| $body);
        };
        ($name:ident, $reset:expr, |$w:ident, $p:ident, $s:ident $(, $arg:ident)*|
            $body:block) => {{
            let weak = window.as_weak(); let state = presenter.clone(); let sender = send.clone();
            window.$name(move |$($arg),*| {
                let Some($w) = weak.upgrade() else { return; };
                {
                    let mut state = state.borrow_mut();
                    let $p = &mut *state;
                    let $s = sender.as_ref();
                    $body
                }
                refresh_properties(&$w, &state.borrow(), $reset);
            });
        }};
    }
    callback!(on_activate, false, |w, p, _s, id| {
        p.active = pane(id);
        let _ = w;
    });
    callback!(on_copy_message, false, |_w, p, s| {
        p.copy_notice_details(s);
    });
    callback!(
        on_diagnostic_notice_closed,
        false,
        |_w, p, _s, generation| {
            p.dismiss_diagnostic_notice(generation);
        }
    );
    callback!(on_retry_directory_elevated, |_w, p, s, id| {
        p.retry_directory_elevated(pane(id), s);
    });
    callback!(on_copy_panel_error, false, |_w, p, s, id, text| {
        p.copy_panel_error(pane(id), text.as_str(), s);
    });
    callback!(on_mutation, |w, p, s, id, action| {
        let kind = match action {
            0 => MutationKind::Rename,
            1 => MutationKind::Delete(DeleteMode::Trash),
            2 => MutationKind::Delete(DeleteMode::Permanent),
            3 => MutationKind::CreateDirectory,
            4 => MutationKind::CreateFile,
            _ => return,
        };
        p.start_operation(pane(id), kind, s);
        let _ = w;
    });
    callback!(on_network_connections, |_w, p, s, disconnect| {
        if !s(Command::ManageNetworkConnections { disconnect }) {
            p.desktop.report(
                "Network connections",
                "Request queue is busy. Try again.".into(),
            );
        }
    });
    callback!(on_confirm_operation, |w, p, s, token, name| {
        p.confirm_operation(token.as_str(), name.to_string(), s);
        if p.operations.preview.is_none() {
            w.invoke_focus_files();
        }
    });
    callback!(on_cancel_operation, |w, p, s, token| {
        if let Some(token) = token.strip_prefix("metadata-job:") {
            p.cancel_operation(token, s);
            return;
        }
        if token.starts_with("copy-job:") {
            p.cancel_copy(token.as_str(), s);
        } else {
            p.cancel_operation(token.as_str(), s);
            if p.operations.preview.is_none() {
                w.invoke_focus_files();
            }
        }
    });
    callback!(on_cancel_job, |_w, p, s, token| {
        if token.starts_with("command-job:") {
            p.cancel_user_command(token.as_str(), s);
        } else if token.starts_with("copy-job:") {
            p.cancel_copy(token.as_str(), s);
        } else {
            p.cancel_operation(
                token
                    .strip_prefix("metadata-job:")
                    .unwrap_or(token.as_str()),
                s,
            );
        }
    });
    callback!(on_pause_job, false, |_w, p, s, token, paused| {
        p.set_job_paused(token.as_str(), paused, s);
    });
    callback!(on_clear_finished_jobs, |_w, p, _s, without_errors| {
        p.clear_finished_jobs(without_errors);
    });
    callback!(on_copy_job_details, false, |_w, p, s, token| {
        if let Some(row) = job_presentation::rows(p)
            .into_iter()
            .find(|row| row.identity == token)
        {
            p.copy_viewer_text(row.detail.to_string(), s);
        }
    });
    callback!(on_decide_copy, |w, p, s, token, choice, name| {
        p.decide_copy(token.as_str(), choice, name.to_string(), s);
        let _ = w;
    });
    callback!(on_defer_copy, |_w, p, _s, token| {
        p.defer_copy_decision(token.as_str());
    });
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        window.on_resolve_copy(move |token| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let (metadata, copy_selected) = {
                let mut p = state.borrow_mut();
                let metadata = p
                    .operations
                    .metadata_conflicts
                    .iter()
                    .find(|conflict| token == format!("metadata-job:{}", conflict.id.0))
                    .cloned();
                let copy_selected = if metadata.is_none() {
                    p.resolve_copy_decision(token.as_str());
                    p.active_copy_decision()
                        .is_some_and(|(id, _)| token == format!("copy-job:{}", id.0))
                        || p.active_rename_decision()
                            .is_some_and(|(id, _)| token == format!("rename-job:{}", id.0))
                } else {
                    false
                };
                (metadata, copy_selected)
            };
            // Native creation can query Presenter synchronously. Never hold its
            // mutable borrow across show/activation, including metadata Resolve.
            if let Some(conflict) = &metadata {
                metadata_presentation::show_conflict(&window, conflict, true);
            }
            refresh_properties(&window, &state.borrow(), true);
            if metadata.is_some() || copy_selected {
                window.invoke_open_decision_window(metadata.is_some(), true);
            }
        });
    }
    callback!(on_navigate, |w, p, s, id, text| {
        p.navigate_text(pane(id), text.as_str(), s);
        let _ = w;
    });
    callback!(on_navigation_choice, |w, p, s, id, token| {
        let navigated = if token == "other-panel" {
            p.navigate_as_other_panel(pane(id), s)
        } else if token.starts_with("network:") {
            p.choose_network_choice(pane(id), token.as_str(), s)
        } else if let Some(bookmark) = panel_models::bookmark_id(p, token.as_str()) {
            p.open_bookmark(pane(id), bookmark, s)
        } else {
            p.choose_navigation_choice(pane(id), token.as_str(), s)
        };
        if navigated {
            p.active = pane(id);
            w.set_active_pane(id);
            w.invoke_focus_files();
        }
    });
    callback!(on_path_part, |w, p, s, id, token| {
        if p.navigate_path_part(pane(id), token.as_str(), s) {
            p.active = pane(id);
            w.set_active_pane(id);
            w.invoke_focus_files();
        }
    });
    callback!(on_path_copy, |_w, p, s, id, token| {
        p.copy_path_part(pane(id), token.as_str(), s);
    });
    callback!(on_open_terminal, |_w, p, s| {
        p.open_terminal(s);
    });
    callback!(on_history_choice, |w, p, s, id, forward, token| {
        if p.choose_history_entry(pane(id), forward, token.as_str(), s) {
            p.active = pane(id);
            w.set_active_pane(id);
            w.invoke_focus_files();
        }
    });
    callback!(on_detail_copy, |w, p, s, id, token| {
        p.copy_panel_detail(pane(id), token.as_str(), s);
        let _ = w;
    });
    callback!(on_sizes_format_changed, |w, p, s, bytes| {
        p.change_size_display(bytes, s);
        let _ = w;
    });
    callback!(on_remember_command_history_changed, |w, p, s, remember| {
        p.change_remember_command_history(remember, s);
        let _ = w;
    });
    callback!(on_copy_verification_changed, |w, p, s, enabled| {
        p.change_copy_verification(enabled, s);
        let _ = w;
    });
    callback!(on_command_history_clear, |w, p, s| {
        p.clear_command_history(s);
        let _ = w;
    });
    callback!(on_command_history_choice, |w, p, _s, token| {
        if let Some(command) = p.recall_command(token.as_str()) {
            let cursor = i32::try_from(command.len()).unwrap_or(i32::MAX);
            w.invoke_set_command(command.into(), cursor);
            w.invoke_focus_command();
        }
    });
    callback!(on_command_history_step, |w, p, _s, older| {
        if let Some(command) = p.rotate_command_history(older, w.get_command_text().as_str()) {
            let cursor = i32::try_from(command.len()).unwrap_or(i32::MAX);
            w.invoke_set_history_command(command.into(), cursor);
        }
    });
    {
        let state = presenter.clone();
        window.on_command_history_reset(move || {
            if let Ok(mut presenter) = state.try_borrow_mut() {
                presenter.reset_command_history_cursor();
            }
        });
    }
    callback!(on_tab_number, |w, p, s, id, number| {
        if p.select_tab_number(pane(id), number, s) {
            p.active = pane(id);
            w.set_active_pane(id);
            w.invoke_focus_files();
        }
    });
    callback!(on_volume, |w, p, s, id, volume| {
        p.choose_volume(pane(id), volume as usize, s);
        let _ = w;
    });
    callback!(on_action, |w, p, s, id, action| {
        let id = pane(id);
        p.active = id;
        w.set_active_pane(id.index() as i32);
        w.invoke_focus_files();
        match action {
            0 => {
                p.history(id, -1, s);
            }
            1 => {
                p.history(id, 1, s);
            }
            2 => {
                p.up(id, s);
            }
            4 => {
                p.go_root(id, s);
            }
            _ => {
                p.refresh_location(id, s);
            }
        }
    });
    callback!(on_sort, |w, p, s, id, col| {
        let id = pane(id);
        p.active = id;
        w.set_active_pane(id.index() as i32);
        w.invoke_focus_files();
        p.sort(id, column(col), s);
    });
    callback!(on_bulk_selection, |w, p, s, id, operation| {
        let operation = match operation {
            0 => SelectionOperation::All,
            1 => SelectionOperation::Clear,
            2 => SelectionOperation::Invert,
            3 => SelectionOperation::FilesOnly,
            4 => SelectionOperation::InvertAll,
            5 => SelectionOperation::ClearFilesOnly,
            _ => return,
        };
        let id = pane(id);
        p.active = id;
        w.set_active_pane(id.index() as i32);
        p.bulk_selection(id, operation, s);
    });
    callback!(on_search_edited, |w, p, s, id, query| {
        p.change_search_query(pane(id), query.as_str(), s);
        let _ = w;
    });
    window.on_search_backspace(|text, anchor, cursor| {
        let (text, cursor) = search_input::erase_previous_grapheme(text.as_str(), anchor, cursor);
        SearchTextEdit {
            text: text.into(),
            cursor: i32::try_from(cursor).unwrap_or(i32::MAX),
        }
    });
    callback!(on_search_filter_changed, |w, p, s, enabled| {
        p.change_search_filter(enabled, s);
        p.flush_selection(s);
        let _ = w;
    });
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        let sender = send.clone();
        window.on_row_click(move |id, row, toggle, shift| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let id = pane(id);
            let old = {
                let mut presenter = state.borrow_mut();
                let old = presenter.panes[id.index()].cursor_index();
                if row < 0 {
                    return;
                }
                presenter.select_row_modified(id, row as usize, toggle, shift, sender.as_ref());
                old
            };
            let model = if id == PaneId::Left {
                window.get_left_rows()
            } else {
                window.get_right_rows()
            };
            if let Some(rows) = model.as_any().downcast_ref::<Rows>() {
                // Row notifications may synchronously read the model, so release the
                // mutable presenter borrow first. Reset would replace click targets.
                if let Some(old) = old {
                    rows.notify.row_changed(old);
                }
                if row >= 0 && (row as usize) < model.row_count() {
                    rows.notify.row_changed(row as usize);
                }
                if shift {
                    let (first, end) = if id == PaneId::Left {
                        (
                            window.get_left_visible_row_start(),
                            window.get_left_visible_row_end(),
                        )
                    } else {
                        (
                            window.get_right_visible_row_start(),
                            window.get_right_visible_row_end(),
                        )
                    };
                    rows.refresh(
                        &state.borrow().panes[id.index()],
                        first.max(0) as usize,
                        end.max(0) as usize,
                        true,
                    );
                }
            }
            refresh_properties(&window, &state.borrow(), false);
        });
    }
    callback!(
        on_panel_warning_details,
        false,
        |_w, p, s, id, identity, tree| {
            p.inspect_panel_warning(pane(id), identity.as_str(), tree, s);
        }
    );
    callback!(on_panel_error_details, false, |_w, p, s, id, text| {
        p.inspect_panel_error(pane(id), text.as_str(), s);
    });
    callback!(on_tree_disclosure, |w, p, s, id, identity| {
        p.disclose_tree(pane(id), identity.as_str(), s);
        let _ = w;
    });
    callback!(on_open, |w, p, s, id, row| {
        let id = pane(id);
        p.active = id;
        w.set_active_pane(id.index() as i32);
        w.invoke_focus_files();
        p.open(id, row as usize, s);
    });
    callback!(on_key, |w, p, s, id, key| {
        let id = pane(id);
        if key == 0 {
            p.active = id.other();
            w.set_active_pane(p.active.index() as i32);
            w.invoke_focus_files();
        } else if key == 1 {
            if p.panes[id.index()].search_allows_activation()
                && let Some(row) = p.panes[id.index()].cursor_index()
            {
                p.open(id, row, s);
            }
        } else if key == 10 {
            if p.panes[id.index()].search_allows_activation() {
                p.open_folder(id, s);
            }
        } else if (2..=7).contains(&key) && !p.panes[id.index()].search.query.is_empty() {
            p.move_search_cursor(id, key);
        } else {
            let (mode, columns, rows) = if id == PaneId::Left {
                (
                    w.get_left_view_mode(),
                    w.get_left_view_columns(),
                    w.get_left_view_rows(),
                )
            } else {
                (
                    w.get_right_view_mode(),
                    w.get_right_view_columns(),
                    w.get_right_view_rows(),
                )
            };
            let panel = &p.panes[id.index()];
            if let Some(row) = panel_view_navigation::destination(
                mode,
                key,
                panel.cursor_index().unwrap_or(0),
                panel.display_len() + panel.parent_offset(),
                columns.max(1) as usize,
                rows.max(1) as usize,
            ) {
                p.focus_display_row(id, row);
            } else {
                p.move_cursor(id, key, s);
            }
        }
    });
    callback!(on_view_mode, |w, p, s, id, mode| {
        p.change_panel_view_mode(pane(id), mode, s);
        let _ = w;
    });
    callback!(on_location, |w, p, s, id, index| {
        let id = pane(id);
        p.active = id;
        p.choose_location(id, index as usize, s);
        w.set_active_pane(id.index() as i32);
        w.invoke_focus_files();
    });
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        let sender = send.clone();
        window.on_drive_letter(move |id, letter| {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            let handled = {
                let mut presenter = state.borrow_mut();
                presenter.drive_letter(pane(id), letter.as_str(), sender.as_ref())
            };
            if handled {
                refresh(&window, &state.borrow());
            }
            handled
        });
    }
    callback!(on_properties, |w, p, s, id| {
        p.properties(pane(id), s);
        let _ = w;
    });
    callback!(on_folder_sizes, |w, p, s, id| {
        p.folder_sizes(pane(id), s);
        let _ = w;
    });
    callback!(on_preferences_changed, |w, p, s, theme, density, field| {
        p.change_preferences(theme, density, field, s);
        let _ = w;
    });
    callback!(on_terminal_changed, |w, p, s, index| {
        p.change_terminal(index, !w.get_command_text().is_empty(), s);
    });
    callback!(on_insert_path, |w, p, _s, id| {
        if !p.native_action_allowed(pane(id)) {
            return;
        }
        let path = p.panes[pane(id).index()].display_path().to_owned();
        insert_literal(&w, p, &path, CommandInsertion::Directory);
    });
    callback!(on_insert_name, |w, p, _s| {
        if let Some(name) = p.focused_name(p.active) {
            insert_literal(&w, p, &name, CommandInsertion::Filename);
        }
    });
    callback!(on_insert_full_path, |w, p, _s| {
        if let Some(path) = p.focused_full_path(p.active) {
            insert_literal(&w, p, &path, CommandInsertion::Filename);
        }
    });
    callback!(on_drive_info, |w, p, s, id| {
        p.drive_info(pane(id), s);
        let _ = w;
    });
    callback!(on_options_opened, |w, p, s| {
        p.begin_options();
        p.settings_picker_context(w.get_options_topic_index(), s);
        p.request_fonts(s);
        p.request_default_editor(s);
        p.request_viewer_plugins(s);
        let _ = w;
    });
    callback!(on_options_reset, |_w, p, s| {
        p.reset_options(s);
    });
    callback!(on_options_accepted, |w, p, s| {
        p.accept_options(s);
        let _ = w;
    });
    callback!(on_options_cancelled, |w, p, s| {
        p.cancel_options(s);
        let _ = w;
    });
    callback!(
        on_thumbnail_options_changed,
        |w, p, s, enabled, network, field| {
            p.change_thumbnail_options(enabled, network, field, s);
            let _ = w;
        }
    );
    callback!(on_thumbnail_size_changed, |_w, p, s, size| {
        p.change_thumbnail_size(size, s);
    });
    callback!(on_editor_apply, |w, p, s, executable, arguments| {
        p.change_editor(executable.as_str(), arguments.as_str(), s);
        let _ = w;
    });
    callback!(on_editor_restore, |w, p, s| {
        p.restore_editor(s);
        let _ = w;
    });
    callback!(on_edit_file, |w, p, s, id| {
        p.edit_file(pane(id), s);
        let _ = w;
    });
    callback!(
        on_font_options_changed,
        |w, p, s, index, desktop, touch, field| {
            p.change_fonts(index, desktop, touch, field, s);
            let _ = w;
        }
    );
    callback!(on_tab_new, |w, p, s, id| {
        p.active = pane(id);
        w.set_active_pane(id);
        p.new_tab(pane(id), s);
        w.invoke_focus_files();
    });
    callback!(on_tab_select, |w, p, s, id, token| {
        if let Ok(tab) = token.parse::<u64>() {
            p.active = pane(id);
            w.set_active_pane(id);
            p.select_tab(pane(id), TabId(tab), s);
            w.invoke_focus_files();
        }
    });
    callback!(on_tab_close, |w, p, s, id, token| {
        if let Ok(tab) = token.parse::<u64>() {
            p.active = pane(id);
            w.set_active_pane(id);
            p.close_tab(pane(id), TabId(tab), s);
            w.invoke_focus_files();
        }
    });
    callback!(on_tab_close_active, |w, p, s, id| {
        p.active = pane(id);
        w.set_active_pane(id);
        p.close_active_tab(pane(id), s);
        w.invoke_focus_files();
    });
    callback!(on_tab_cycle, |w, p, s, id, delta| {
        p.active = pane(id);
        w.set_active_pane(id);
        p.cycle_tab(pane(id), delta as isize, s);
        w.invoke_focus_files();
    });
    callback!(on_bookmark_add, |w, p, s, id, name, slot| {
        let slot = u8::try_from(slot).ok().filter(|value| *value != 0);
        p.add_bookmark(pane(id), name.to_string(), slot, s);
        let _ = w;
    });
    callback!(on_bookmark_open, |w, p, s, id, token| {
        if let Some(bookmark) = panel_models::bookmark_id(p, token.as_str())
            .or_else(|| token.parse::<u64>().ok().map(BookmarkId))
        {
            p.active = pane(id);
            w.set_active_pane(id);
            p.open_bookmark(pane(id), bookmark, s);
            w.invoke_focus_files();
        }
    });
    callback!(on_bookmark_remove, |w, p, s, token| {
        if let Ok(bookmark) = token.parse::<u64>() {
            p.remove_bookmark(BookmarkId(bookmark), s);
        }
        let _ = w;
    });
    callback!(on_bookmark_slot, |w, p, s, id, slot| {
        if let Ok(slot) = u8::try_from(slot) {
            p.assign_bookmark_slot(pane(id), slot, s);
        }
        let _ = w;
    });
    callback!(on_bookmark_assign, |w, p, s, token, slot| {
        if let (Ok(bookmark), Ok(slot)) = (token.parse::<u64>(), u8::try_from(slot)) {
            p.set_bookmark_slot(BookmarkId(bookmark), (slot != 0).then_some(slot), s);
        }
        let _ = w;
    });
    callback!(on_bookmark_jump, |w, p, s, id, slot| {
        if let Ok(slot) = u8::try_from(slot) {
            p.jump_bookmark(pane(id), slot, s);
        }
        let _ = w;
    });
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        window.on_command_edited(move |nonempty| {
            let changed = {
                // A Rust path insertion binds before setting text. If its text
                // update reenters this observer, that callback already owns state.
                let Ok(mut presenter) = state.try_borrow_mut() else {
                    return;
                };
                presenter.track_draft(nonempty)
            };
            if changed && let Some(window) = weak.upgrade() {
                refresh(&window, &state.borrow());
            }
        });
    }
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        let sender = send.clone();
        window.on_submit_command(move |text| {
            let Some(window) = weak.upgrade() else {
                return false;
            };
            let submitted = state
                .borrow_mut()
                .submit_command(text.as_str(), sender.as_ref());
            refresh(&window, &state.borrow());
            submitted
        });
    }
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        let sender = send.clone();
        window.on_exit(move || {
            if let Some(window) = weak.upgrade() {
                if state.borrow().has_live_operations() {
                    state
                        .borrow_mut()
                        .request_operation_shutdown(sender.as_ref());
                    window.invoke_show_operations();
                    refresh(&window, &state.borrow());
                } else {
                    state.borrow_mut().shutdown_drag_drop(sender.as_ref());
                    window.invoke_close_find_windows();
                    window.invoke_close_checksum_windows();
                    window.invoke_close_file_list_window();
                    window.invoke_close_operation_windows();
                    window.invoke_close_jobs_window();
                    window.invoke_close_decision_windows();
                    window.invoke_close_panel_filter_window();
                    window.invoke_close_hot_paths_window();
                    window.invoke_close_information_windows();
                    let _ = window.hide();
                }
            }
        });
    }
    {
        let weak = window.as_weak();
        window.window().on_close_requested(move || {
            if let Some(window) = weak.upgrade() {
                window.invoke_exit();
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }
    if !send(Command::LoadLocale) {
        presenter.borrow_mut().desktop.report(
            "Regional format unavailable",
            "Request queue is busy; invariant format is used.".into(),
        );
    }
    if !send(Command::ListVolumes) {
        presenter.borrow_mut().volumes_error = "Volume request queue is busy.".into();
    }
    if !send(Command::ListDesktopContext) {
        presenter.borrow_mut().desktop.error = "Desktop context queue is busy.".into();
    }
    if !send(Command::LoadPreferences) {
        presenter.borrow_mut().desktop.report(
            "Options queue busy",
            "Default appearance is active; saved options could not be requested.".into(),
        );
    }
    for id in [PaneId::Left, PaneId::Right] {
        let path = presenter.borrow().panes[id.index()].path.clone();
        presenter
            .borrow_mut()
            .navigate(id, path, None, send.as_ref());
    }
    refresh(window, &presenter.borrow());
    window.invoke_focus_files();
}

enum CommandInsertion {
    Directory,
    Filename,
}

fn insert_literal(
    window: &MainWindow,
    presenter: &mut Presenter,
    path: &std::path::Path,
    insertion: CommandInsertion,
) {
    window.invoke_focus_command();
    presenter.track_draft(!window.get_command_text().is_empty());
    let draft = window.get_command_text().to_string();
    let (anchor, cursor) = if matches!(insertion, CommandInsertion::Filename) {
        let Ok(end) = i32::try_from(draft.len()) else {
            presenter
                .desktop
                .report("Cannot insert name", "Command text is too large.".into());
            return;
        };
        (end, end)
    } else {
        (window.get_command_anchor(), window.get_command_cursor())
    };
    let token = match insertion {
        CommandInsertion::Directory => command_directory_text(path),
        CommandInsertion::Filename => command_path_text(path).map(std::borrow::Cow::Borrowed),
    };
    match token {
        Ok(token) => {
            let (text, offset) = replace_command_selection(&draft, anchor, cursor, &token);
            presenter.track_draft(!text.is_empty());
            window.invoke_set_command(text.into(), offset);
        }
        Err(error) => presenter.desktop.report("Cannot insert path", error.into()),
    }
}

#[cfg(test)]
mod model_tests;
