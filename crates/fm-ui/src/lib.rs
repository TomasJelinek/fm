//! In-memory presentation and a virtualized Slint shell. All I/O lives behind commands.
#![deny(unsafe_code)]
mod command_line;
mod font_model;
mod native_shortcuts;
mod presenter;
mod scroll_input;
mod size_presentation;
mod volume_presentation;
use command_line::{replace_command_selection, safe_path_context};
use fm_core::*;
pub use presenter::{DeferredDrop, Presenter, TabSummary};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::*;

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
}

impl RowStructure {
    fn from_pane(pane: &presenter::Pane) -> Self {
        Self {
            navigation: pane.generation,
            displayed: pane.displayed_generation(),
            entries: pane.rows().as_ptr() as usize,
            length: pane.rows().len(),
            parent: pane.parent_offset(),
            streaming: pane.showing_stream(),
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
        let structure = RowStructure::from_pane(pane);
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
        pane.rows().len() + pane.parent_offset()
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
                size: "".into(),
                size_state: 0,
                size_hint: "Parent folder".into(),
                modified: "".into(),
                kind: "Parent folder".into(),
                icon: state.desktop.image(&IconKey::folder()),
                directory: true,
                selected: false,
                cursor: pane.parent_cursor,
            });
        }
        let index = index.checked_sub(pane.parent_offset())?;
        let entry = pane.rows().get(index)?;
        let metadata = if pane.showing_stream() {
            pane.metadata.get(&entry.id).unwrap_or(&entry.metadata)
        } else {
            &entry.metadata
        };
        let directory = entry.kind == EntryKind::Directory;
        let (size, size_state, size_hint) = if directory {
            size_presentation::folder_size(
                pane.folder_sizes.get(&entry.path),
                pane.size_scan.is_some(),
                pane.size_scan_attempted,
            )
        } else {
            (
                metadata.size.map(format_size).unwrap_or_else(|| "—".into()),
                0,
                "File length from backend metadata.".into(),
            )
        };
        let modified = metadata
            .modified
            .map(|time| {
                let local: chrono::DateTime<chrono::Local> = time.into();
                local.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_else(|| "—".into());
        let kind = match entry.kind {
            EntryKind::Directory => "Folder".into(),
            EntryKind::Symlink => "Link".into(),
            EntryKind::Other => "Other".into(),
            EntryKind::File => entry
                .path
                .extension()
                .map(|ext| ext.to_string_lossy().to_uppercase())
                .unwrap_or_else(|| "File".into()),
        };
        Some(FileRow {
            // Include intent and committed generations: a pending refresh may keep
            // the old rows visible, and a provider may recycle IDs at its commit.
            identity: format!(
                "{}:{}:entry:{}",
                pane.generation,
                pane.displayed_generation().0,
                entry.id.0
            )
            .into(),
            name: entry.display_name.clone().into(),
            size: size.into(),
            size_state,
            size_hint: size_hint.into(),
            modified: modified.into(),
            kind: kind.into(),
            icon: state.desktop.image(&IconKey::for_entry(entry)),
            directory,
            selected: pane.selected.contains_path(&entry.path),
            cursor: pane.cursor == Some(entry.id),
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn format_size(bytes: u64) -> String {
    for (unit, divisor) in [
        ("TB", 1u64 << 40),
        ("GB", 1 << 30),
        ("MB", 1 << 20),
        ("KB", 1 << 10),
    ] {
        if bytes >= divisor {
            return format!("{:.1} {unit}", bytes as f64 / divisor as f64);
        }
    }
    format!("{bytes} B")
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

fn location_rows(presenter: &Presenter, pane: PaneId) -> Vec<LocationRow> {
    let path = presenter.panes[pane.index()].display_path();
    let closest = presenter
        .desktop
        .locations
        .iter()
        .enumerate()
        .filter(|(_, location)| path.starts_with(&location.path))
        .max_by_key(|(_, location)| location.path.components().count())
        .map(|(index, _)| index);
    presenter
        .desktop
        .locations
        .iter()
        .enumerate()
        .map(|(index, location)| LocationRow {
            label: location.label.clone().into(),
            compact_label: location
                .drive_letter
                .map(|letter| format!("{letter}:"))
                .unwrap_or_else(|| {
                    location
                        .path
                        .file_name()
                        .unwrap_or(location.path.as_os_str())
                        .to_string_lossy()
                        .into_owned()
                })
                .into(),
            is_drive: location.kind == LocationKind::Volume,
            path: location.path.to_string_lossy().into_owned().into(),
            tooltip: if location.kind == LocationKind::Volume {
                let details = volume_presentation::tooltip(
                    presenter
                        .volumes
                        .iter()
                        .find(|volume| volume.root == location.path)
                        .and_then(|volume| volume_presentation::details(presenter, volume)),
                );
                format!("{}\n{details}", location.path.display()).into()
            } else {
                format!("{}\n{}", location.label, location.path.display()).into()
            },
            icon: presenter
                .desktop
                .image(&if location.kind == LocationKind::Volume {
                    IconKey::for_drive(&location.path)
                } else {
                    IconKey::folder()
                }),
            selected: closest == Some(index),
        })
        .collect()
}

/// Refresh visible properties and lazy model notifications from current presenter state.
pub fn refresh(window: &MainWindow, presenter: &Presenter) {
    refresh_properties(window, presenter, true);
}

fn refresh_properties(window: &MainWindow, presenter: &Presenter, reset_rows: bool) {
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
                path: bookmark.path.to_string_lossy().into_owned().into(),
                slot: i32::from(bookmark.slot.unwrap_or(0)),
            })
            .collect(),
        |model| window.set_bookmarks(model),
    );
    let bookmark_name = |pane: PaneId| {
        let path = presenter.panes[pane.index()].display_path();
        SharedString::from(
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .as_ref(),
        )
    };
    window.set_left_bookmark_name(bookmark_name(PaneId::Left));
    window.set_right_bookmark_name(bookmark_name(PaneId::Right));
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
    window.invoke_apply_fonts(
        presenter.preferences.values.font_family.as_str().into(),
        i32::from(presenter.preferences.values.desktop_font_size),
        i32::from(presenter.preferences.values.touch_font_size),
    );
    window.set_desktop_feedback_important(
        presenter.desktop.terminal_guidance.is_some() || !presenter.desktop.error.is_empty(),
    );
    window.invoke_apply_preferences(
        presenter.preferences.theme_index(),
        presenter.preferences.density_index(),
    );
    window.set_preferences_unsaved(presenter.preferences.unsaved);
    window.set_active_pane(presenter.active.index() as i32);
    window.set_volumes_revision(presenter.volumes_revision);
    let mut reveal = [false; 2];
    for (index, model) in [window.get_left_rows(), window.get_right_rows()]
        .iter()
        .enumerate()
    {
        let p = &presenter.panes[index];
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
        let location = p.display_path().to_string_lossy().into_owned();
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
        let mut status = format!("{} items  ·  {} selected", p.rows().len(), p.selected.len());
        if p.size_scan.is_some() {
            status.push_str("  ·  Measuring folders… (Ctrl+Shift+S cancels)");
        }
        if let Some(space) = p.visible_space() {
            status.push_str(&format!(
                "  ·  {} free / {}",
                format_size(space.available),
                format_size(space.total)
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
            window.set_left_error(error.into());
            window.set_left_loading(p.loading);
            window.set_left_cursor(p.cursor_index().map_or(-1, |i| i as i32));
            window.set_left_sort(column_index(p.sort.column));
            window.set_left_descending(p.sort.descending);
        } else {
            window.set_right_path(location.into());
            window.set_right_volume(volume);
            window.set_right_status(status.into());
            window.set_right_error(error.into());
            window.set_right_loading(p.loading);
            window.set_right_cursor(p.cursor_index().map_or(-1, |i| i as i32));
            window.set_right_sort(column_index(p.sort.column));
            window.set_right_descending(p.sort.descending);
        }
    }
    update_model(
        window.get_left_locations(),
        location_rows(presenter, PaneId::Left),
        |model| window.set_left_locations(model),
    );
    update_model(
        window.get_right_locations(),
        location_rows(presenter, PaneId::Right),
        |model| window.set_right_locations(model),
    );
    window.set_terminal_ready(
        presenter.desktop.shell.is_some() && presenter.desktop.terminal_pending.is_none(),
    );
    window.set_terminal_available(presenter.desktop.shell.is_some());
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
        window.set_message_generation(presenter.desktop.notice_generation);
    }
    let labels: Vec<SharedString> = presenter
        .volumes
        .iter()
        .map(|v| {
            volume_presentation::menu_label(v, volume_presentation::details(presenter, v)).into()
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

/// Register UI callbacks and request the initial locations. The sender must be nonblocking.
pub fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    scroll_input::install(window);
    window.set_font_choices(Rc::new(font_model::FontRows::new(presenter.clone())).into());
    window.set_left_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Left)));
    window.set_right_rows(ModelRc::new(Rows::new(presenter.clone(), PaneId::Right)));

    macro_rules! callback {
        ($name:ident, |$w:ident, $p:ident, $s:ident| $body:block) => {
            callback!($name, true, |$w, $p, $s,| $body);
        };
        ($name:ident, |$w:ident, $p:ident, $s:ident, $($arg:ident),*| $body:block) => {
            callback!($name, true, |$w, $p, $s, $($arg),*| $body);
        };
        ($name:ident, $reset:expr, |$w:ident, $p:ident, $s:ident, $($arg:ident),*|
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
    callback!(on_navigate, |w, p, s, id, text| {
        p.navigate_text(pane(id), text.as_str(), s);
        let _ = w;
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
                let path = p.panes[id.index()].display_path().to_owned();
                p.navigate(id, path, None, s);
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
            _ => return,
        };
        let id = pane(id);
        p.active = id;
        w.set_active_pane(id.index() as i32);
        p.bulk_selection(id, operation, s);
    });
    {
        let weak = window.as_weak();
        let state = presenter.clone();
        let sender = send.clone();
        window.on_row_click(move |id, row, toggle| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let id = pane(id);
            let old = {
                let mut presenter = state.borrow_mut();
                let old = presenter.panes[id.index()].cursor_index();
                presenter.select_row(id, row as usize, toggle, sender.as_ref());
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
            }
            refresh_properties(&window, &state.borrow(), false);
        });
    }
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
            if let Some(row) = p.panes[id.index()].cursor_index() {
                p.open(id, row, s);
            }
        } else if key == 10 {
            p.open_folder(id, s);
        } else {
            p.move_cursor(id, key, s);
        }
    });
    callback!(on_location, |w, p, s, id, index| {
        let id = pane(id);
        p.active = id;
        p.choose_location(id, index as usize, s);
        w.set_active_pane(id.index() as i32);
        w.invoke_focus_files();
    });
    callback!(on_drive_letter, |w, p, s, id, letter| {
        let id = pane(id);
        p.active = id;
        p.drive_letter(id, letter.as_str(), s);
        let _ = w;
    });
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
        let path = p.panes[pane(id).index()].display_path().to_owned();
        insert_literal(&w, p, &path, false);
    });
    callback!(on_insert_name, |w, p, _s| {
        if let Some(name) = p.focused_name(p.active) {
            insert_literal(&w, p, &name, true);
        }
    });
    callback!(on_drive_info, |w, p, _s, id| {
        p.drive_info(pane(id));
        let _ = w;
    });
    callback!(on_options_opened, |w, p, s| {
        p.request_fonts(s);
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
        if let Ok(bookmark) = token.parse::<u64>() {
            p.active = pane(id);
            w.set_active_pane(id);
            p.open_bookmark(pane(id), BookmarkId(bookmark), s);
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
        window.on_exit(move || {
            if let Some(window) = weak.upgrade() {
                let _ = window.hide();
            }
        });
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

fn insert_literal(
    window: &MainWindow,
    presenter: &mut Presenter,
    path: &std::path::Path,
    append: bool,
) {
    window.invoke_focus_command();
    presenter.track_draft(!window.get_command_text().is_empty());
    let Some(shell) = presenter.effective_shell() else {
        return;
    };
    let mut draft = window.get_command_text().to_string();
    let (anchor, cursor) = if append {
        if !draft.is_empty() && !draft.ends_with([' ', '\t']) {
            draft.push(' ');
        }
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
    if !safe_path_context(&draft, anchor, cursor, shell) {
        presenter.desktop.report(
            "Place the caret between arguments",
            "Insert a name or path outside quotes, between arguments, or replace a whole argument."
                .into(),
        );
        return;
    }
    match quote_shell_path(path, shell) {
        Ok(token) => {
            let (text, offset) = replace_command_selection(&draft, anchor, cursor, &token);
            presenter.bind_inserted_path(shell);
            window.invoke_set_command(text.into(), offset);
        }
        Err(error) => presenter.desktop.report("Cannot insert path", error.into()),
    }
}

#[cfg(test)]
mod model_tests;
