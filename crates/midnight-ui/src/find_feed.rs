//! Search results fed to a panel: the list's models, its callbacks and the routing
//! of panel actions (keys and the function bar) to the results. No filesystem
//! access or ordering work happens here.
use crate::{
    FeedPresentation, FeedRow, FeedView, MainWindow, Presenter,
    file_keys::FileKey,
    presenter::find::feed::{FeedAction, FeedMove},
    shown_text,
};
use midnight_core::*;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct PaneModel {
    rows: Rc<VecModel<FeedRow>>,
    /// The rows serial and revision last shown.
    shown: Cell<Option<(u64, u64)>>,
    open: Cell<bool>,
}

pub(crate) struct FeedModels {
    panes: [PaneModel; 2],
}

// Codes of `FeedPresentation.action`, sent by `dispatch`.
const LEAVE: i32 = 0;
const GO_TO: i32 = 1;
const UP: i32 = 2;
const DOWN: i32 = 3;
const PAGE_UP: i32 = 4;
const PAGE_DOWN: i32 = 5;
const FIRST: i32 = 6;
const LAST: i32 = 7;
const PREVIOUS_SELECTED: i32 = 8;
const NEXT_SELECTED: i32 = 9;
const TOGGLE: i32 = 10;
const TOGGLE_ADVANCE: i32 = 11;
const BULK: i32 = 20;
const VIEW: i32 = 30;
const VIEW_BASIC: i32 = 31;
const EDIT: i32 = 32;
const COPY_TO: i32 = 33;
const MOVE_TO: i32 = 34;
const TRASH: i32 = 35;
const DELETE: i32 = 36;
const CLIPBOARD_COPY: i32 = 37;
const CLIPBOARD_CUT: i32 = 38;
const PROPERTIES: i32 = 39;

fn action(code: i32) -> Option<FeedAction> {
    Some(match code {
        LEAVE => FeedAction::Leave,
        GO_TO => FeedAction::GoTo,
        UP => FeedAction::Move(FeedMove::Up),
        DOWN => FeedAction::Move(FeedMove::Down),
        PAGE_UP => FeedAction::Move(FeedMove::PageUp),
        PAGE_DOWN => FeedAction::Move(FeedMove::PageDown),
        FIRST => FeedAction::Move(FeedMove::First),
        LAST => FeedAction::Move(FeedMove::Last),
        PREVIOUS_SELECTED => FeedAction::Move(FeedMove::PreviousSelected),
        NEXT_SELECTED => FeedAction::Move(FeedMove::NextSelected),
        TOGGLE => FeedAction::Toggle { advance: false },
        TOGGLE_ADVANCE => FeedAction::Toggle { advance: true },
        code if (BULK..BULK + 6).contains(&code) => FeedAction::Bulk(bulk_operation(code - BULK)?),
        VIEW => FeedAction::View(false),
        VIEW_BASIC => FeedAction::View(true),
        EDIT => FeedAction::Edit,
        COPY_TO => FeedAction::Transfer(TransferIntent::Copy),
        MOVE_TO => FeedAction::Transfer(TransferIntent::Move),
        TRASH => FeedAction::Delete(DeleteMode::Trash),
        DELETE => FeedAction::Delete(DeleteMode::Permanent),
        CLIPBOARD_COPY => FeedAction::Clipboard { cut: false },
        CLIPBOARD_CUT => FeedAction::Clipboard { cut: true },
        PROPERTIES => FeedAction::Properties,
        _ => return None,
    })
}

/// A bulk selection code (`FileKey::Bulk` 0–5) as the operation a panel applies for
/// it (`MainWindow.bulk-selection`), so that the list marks rows as a panel does.
fn bulk_operation(code: i32) -> Option<SelectionOperation> {
    Some(match code {
        0 => SelectionOperation::All,
        1 => SelectionOperation::Clear,
        2 => SelectionOperation::Invert,
        3 => SelectionOperation::FilesOnly,
        4 => SelectionOperation::InvertAll,
        5 => SelectionOperation::ClearFilesOnly,
        _ => return None,
    })
}

/// A panel action for `pane`, taken before the folder under a shown result list
/// sees it. None lets the ordinary dispatch run (panel switch, path, windows).
pub(crate) fn dispatch(window: &MainWindow, pane: i32, key: &FileKey) -> Option<i32> {
    let feeds = window.global::<FeedPresentation>();
    let open = if pane == 0 {
        feeds.get_left().open
    } else {
        feeds.get_right().open
    };
    if !open {
        return None;
    }
    let refuse = |label: &str| {
        feeds.invoke_refuse(pane, label.into());
        Some(1)
    };
    let code = match key {
        FileKey::Panel(0) => return None,
        FileKey::Panel(1 | 10) => GO_TO,
        FileKey::Panel(2) => UP,
        FileKey::Panel(3) => DOWN,
        FileKey::Panel(4) => PAGE_UP,
        FileKey::Panel(5) => PAGE_DOWN,
        FileKey::Panel(6) => FIRST,
        FileKey::Panel(7) => LAST,
        FileKey::Panel(8) => TOGGLE,
        FileKey::Panel(9) | FileKey::ClearAndInsert => TOGGLE_ADVANCE,
        FileKey::Panel(13) => PREVIOUS_SELECTED,
        FileKey::Panel(14) => NEXT_SELECTED,
        FileKey::Panel(_) => return Some(1),
        // Keypad + and −, Ctrl+= and Ctrl+−: masks select folder entries.
        FileKey::Bulk(6) => return refuse("Select by mask"),
        FileKey::Bulk(7) => return refuse("Deselect by mask"),
        // Keypad *: inverts the marks, as Ctrl+I does in the list.
        FileKey::Bulk(8) => BULK + 2,
        FileKey::Bulk(operation) => BULK + operation,
        FileKey::Action(0 | 2 | 4) => LEAVE,
        FileKey::Action(5) => return refuse("Show hidden files"),
        FileKey::Action(_) => return Some(1),
        FileKey::View(basic) => {
            if *basic {
                VIEW_BASIC
            } else {
                VIEW
            }
        }
        FileKey::Edit => EDIT,
        FileKey::Mutation(1) => TRASH,
        FileKey::Mutation(2) => DELETE,
        FileKey::Mutation(0) => return refuse("Rename"),
        FileKey::Mutation(3) => return refuse("Create folder"),
        FileKey::Mutation(_) => return refuse("New file"),
        FileKey::Clipboard(0) => CLIPBOARD_COPY,
        FileKey::Clipboard(2) => COPY_TO,
        FileKey::Clipboard(3) => MOVE_TO,
        FileKey::Clipboard(4) => CLIPBOARD_CUT,
        FileKey::Clipboard(_) => return refuse("Paste"),
        FileKey::Properties => PROPERTIES,
        FileKey::Sort(_) => {
            return refuse("Sorting (sort the results in the Find window before feeding them)");
        }
        FileKey::FolderSizes => return refuse("Folder sizes"),
        FileKey::Metadata(_) => return refuse("Attributes, times and case"),
        FileKey::CompareFiles => return refuse("Compare files"),
        FileKey::Checksums => return refuse("Checksums"),
        FileKey::MakeFileList => return refuse("Make file list"),
        FileKey::UserCommand(_) => return refuse("A custom command"),
        FileKey::Compare => return refuse("Compare directories"),
        FileKey::Filter => return refuse("The panel filter"),
        FileKey::Native | FileKey::Consumed | FileKey::ClearSearch | FileKey::ErasePrevious => {
            return Some(1);
        }
        FileKey::FocusPath
        | FileKey::Find
        | FileKey::DriveInfo
        | FileKey::Zoom
        | FileKey::TryDrive(_)
        | FileKey::Window(_) => return None,
    };
    feeds.invoke_action(pane, code);
    Some(1)
}

pub(crate) fn install(
    main: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
    begin_owned: &crate::BeginOwned,
) -> Rc<FeedModels> {
    let models = Rc::new(FeedModels {
        panes: std::array::from_fn(|_| PaneModel {
            rows: Rc::new(VecModel::default()),
            shown: Cell::new(None),
            open: Cell::new(false),
        }),
    });
    let feeds = main.global::<FeedPresentation>();
    feeds.set_left_rows(ModelRc::from(models.panes[0].rows.clone()));
    feeds.set_right_rows(ModelRc::from(models.panes[1].rows.clone()));
    // Like the panel's own callbacks: a Copy/Move draft or a delete capture is built
    // inside the host's owned-command scope, so its owners reach reclamation.
    macro_rules! callback {
        ($name:ident, |$p:ident, $send:ident $(, $arg:ident)*| $body:block) => {{
            let state = state.clone();
            let sender = send.clone();
            let begin = begin_owned.clone();
            let weak = main.as_weak();
            feeds.$name(move |$($arg),*| {
                if crate::owned_commands::with_control_commands(
                    &state,
                    &begin,
                    sender.as_ref(),
                    |$p, $send| $body,
                )
                .is_none()
                {
                    return;
                }
                if let Some(main) = weak.upgrade() {
                    crate::refresh(&main, &state.borrow());
                }
            });
        }};
    }
    callback!(on_click, |p, _send, pane, row, control, shift| {
        if let Ok(row) = usize::try_from(row) {
            p.feed_click(crate::pane(pane), row, control, shift);
        }
    });
    callback!(on_open_row, |p, send, pane, row| {
        if let Ok(row) = usize::try_from(row) {
            p.feed_click(crate::pane(pane), row, false, false);
            p.feed_action(crate::pane(pane), FeedAction::GoTo, send);
        }
    });
    callback!(on_leave, |p, send, pane| {
        p.feed_action(crate::pane(pane), FeedAction::Leave, send);
    });
    {
        // Only the page size for Page Up/Down: nothing to show or send. Both lists
        // exist while hidden and report on every resize and font change, so a window
        // refresh here would re-apply the saved appearance and repaint everything.
        let state = state.clone();
        feeds.on_page_rows(move |pane, rows| {
            if let Ok(mut p) = state.try_borrow_mut() {
                p.feed_page_rows(crate::pane(pane), usize::try_from(rows).unwrap_or(1));
            }
        });
    }
    callback!(on_action, |p, send, pane, code| {
        if let Some(action) = action(code) {
            p.feed_action(crate::pane(pane), action, send);
        }
    });
    callback!(on_refuse, |p, _send, pane, label| {
        let _ = pane;
        p.feed_refuse(&label);
    });
    models
}

impl FeedModels {
    pub(crate) fn refresh(&self, main: &MainWindow, presenter: &Presenter) {
        let feeds = main.global::<FeedPresentation>();
        for (slot, pane) in [PaneId::Left, PaneId::Right].into_iter().enumerate() {
            let model = &self.panes[slot];
            let view = match presenter.feed(pane) {
                Some(feed) => {
                    let key = (feed.rows_serial, feed.revision);
                    if model.shown.get().map(|(rows, _)| rows) != Some(key.0) {
                        model.rows.set_vec(
                            feed.entries
                                .iter()
                                .zip(&feed.selected)
                                .map(|(entry, selected)| row(entry, *selected, presenter))
                                .collect::<Vec<_>>(),
                        );
                    } else if model.shown.get() != Some(key) {
                        for (index, selected) in feed.selected.iter().enumerate() {
                            if let Some(mut shown) = model.rows.row_data(index)
                                && shown.selected != *selected
                            {
                                shown.selected = *selected;
                                model.rows.set_row_data(index, shown);
                            }
                        }
                    }
                    model.shown.set(Some(key));
                    FeedView {
                        open: true,
                        title: shown_text(&feed.title),
                        status: shown_text(&status(feed, presenter)),
                        cursor: i32::try_from(feed.cursor).unwrap_or(i32::MAX),
                    }
                }
                None => {
                    if model.shown.take().is_some() {
                        model.rows.set_vec(Vec::new());
                    }
                    FeedView::default()
                }
            };
            let open = view.open;
            if slot == 0 {
                feeds.set_left(view);
            } else {
                feeds.set_right(view);
            }
            // Opening takes the keyboard to the list; leaving gives it back to the folder.
            if model.open.replace(open) != open && main.get_active_pane() == slot as i32 {
                main.invoke_focus_files();
            }
        }
    }
}

fn row(entry: &Entry, selected: bool, presenter: &Presenter) -> FeedRow {
    let directory = entry.kind == EntryKind::Directory;
    let folder = shown_text(
        &entry
            .path
            .parent()
            .map(|parent| parent.to_string_lossy())
            .unwrap_or_default(),
    );
    FeedRow {
        name: shown_text(&entry.display_name),
        folder_tail: crate::path_tail(&folder),
        folder,
        size: if directory {
            "<DIR>".into()
        } else {
            entry.metadata.size.map_or_else(
                || "—".into(),
                |bytes| {
                    crate::size_presentation::file_size(
                        bytes,
                        presenter.preferences.values.size_display,
                        &presenter.number_locale,
                    )
                    .into()
                },
            )
        },
        modified: crate::time_presentation::modified(&entry.metadata)
            .map(|time| format!("{} {}", time.date, time.time).into())
            .unwrap_or_default(),
        directory,
        selected,
    }
}

fn status(feed: &crate::presenter::find::feed::FeedPane, presenter: &Presenter) -> String {
    let locale = &presenter.number_locale;
    let mut text = format!(
        "{} of {} selected",
        locale.format_u64(feed.selected_count as u64),
        locale.format_u64(feed.entries.len() as u64)
    );
    if feed.selected_bytes > 0 {
        text.push_str(&format!(
            " · {}",
            crate::size_presentation::file_size(
                feed.selected_bytes,
                presenter.preferences.values.size_display,
                locale
            )
        ));
    }
    if feed.omitted > 0 {
        text.push_str(&format!(
            " · {} archive members not shown",
            locale.format_u64(feed.omitted as u64)
        ));
    }
    text.push_str(" · Enter: go to file · Backspace: back to folder");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_bulk_selection_action_reaches_the_list_with_its_panel_meaning() {
        for (id, operation) in [
            ("selection.all", SelectionOperation::All),
            ("selection.files", SelectionOperation::FilesOnly),
            ("selection.clear", SelectionOperation::Clear),
            ("selection.clear-files", SelectionOperation::ClearFilesOnly),
            ("selection.invert", SelectionOperation::Invert),
            ("selection.invert-all", SelectionOperation::InvertAll),
        ] {
            let Some(FileKey::Bulk(code)) = crate::file_keys::semantic(id, false) else {
                panic!("{id} is a bulk selection key");
            };
            assert_eq!(
                action(BULK + code),
                Some(FeedAction::Bulk(operation)),
                "{id}"
            );
        }
    }
}
