//! The update check's presentation (docs/developer/UPDATES.md): the backend's state
//! mirrored into `UpdateState`, the dialog's opening and closing, and the Options draft.
//! Network and disk work stay in the application's update service; the application
//! replaces the action callbacks (`check-now`, `download`, `restart`, `skip`) and the
//! dialog hooks with its own after `install`.
use crate::{MainWindow, Presenter, UpdateState};
use midnight_core::Command;
use slint::{ComponentHandle, SharedString};
use std::{cell::RefCell, rc::Rc};

/// What the update button and dialog show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Verifying,
    Ready,
    Failed,
}

impl UpdatePhase {
    fn index(self) -> i32 {
        match self {
            Self::Idle => 0,
            Self::Checking => 1,
            Self::UpToDate => 2,
            Self::Available => 3,
            Self::Downloading => 4,
            Self::Verifying => 5,
            Self::Ready => 6,
            Self::Failed => 7,
        }
    }
}

/// The update state as the application hands it over; formatting happens here.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateView {
    pub phase: UpdatePhase,
    pub current_version: String,
    pub latest_version: String,
    pub notes: String,
    pub notes_cut: bool,
    pub preview: bool,
    pub skipped: bool,
    /// GitHub's publication time (`2026-11-23T10:00:00Z`); the date is shown.
    pub published: String,
    /// Unix seconds of the last answered check.
    pub last_checked: Option<u64>,
    pub message: String,
    pub blocked: String,
    pub notice: String,
    pub received: u64,
    pub total: u64,
    pub page_url: String,
}

thread_local! {
    static PAGE_URL: RefCell<String> = const { RefCell::new(String::new()) };
    static OPENINGS: RefCell<u64> = const { RefCell::new(0) };
}

fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn local_time(seconds: u64) -> String {
    use chrono::TimeZone;
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| chrono::Local.timestamp_opt(seconds, 0).single())
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Mirrors `view` into the button and the dialog.
pub fn show_update(window: &MainWindow, view: &UpdateView) {
    let state = window.global::<UpdateState>();
    state.set_phase(view.phase.index());
    state.set_current_version(view.current_version.as_str().into());
    state.set_latest_version(view.latest_version.as_str().into());
    state.set_notes(view.notes.as_str().into());
    state.set_notes_cut(view.notes_cut);
    state.set_preview(view.preview);
    state.set_skipped(view.skipped);
    state.set_published(view.published.get(..10).unwrap_or_default().into());
    state.set_last_checked(view.last_checked.map(local_time).unwrap_or_default().into());
    state.set_message(view.message.as_str().into());
    state.set_blocked(view.blocked.as_str().into());
    state.set_notice(view.notice.as_str().into());
    let progress = if view.total > 0 {
        (view.received as f64 / view.total as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    state.set_progress(progress);
    state.set_progress_text(if view.total > 0 {
        SharedString::from(format!(
            "{} of {}",
            mebibytes(view.received),
            mebibytes(view.total)
        ))
    } else {
        SharedString::default()
    });
    state.set_has_release_page(view.page_url.starts_with("https://"));
    PAGE_URL.with(|url| url.borrow_mut().clone_from(&view.page_url));
}

/// Opens the update dialog; a no-op while it is open.
pub fn open_update_dialog(window: &MainWindow) {
    let state = window.global::<UpdateState>();
    if !state.get_token().is_empty() {
        return;
    }
    crate::tooltip::dismiss(window.window());
    let opening = OPENINGS.with(|openings| {
        let mut openings = openings.borrow_mut();
        *openings += 1;
        *openings
    });
    state.set_token(opening.to_string().into());
}

/// Closes the update dialog and returns the keyboard to the file list.
pub fn close_update_dialog(window: &MainWindow) {
    let state = window.global::<UpdateState>();
    if state.get_token().is_empty() {
        return;
    }
    crate::tooltip::dismiss(window.window());
    state.set_token(SharedString::default());
    window.invoke_focus_files();
}

/// The Options draft's update choices; called on every presenter refresh.
pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = window.global::<UpdateState>();
    let updates = presenter.preferences.values.updates;
    if state.get_check_at_startup() != updates.check_at_startup {
        state.set_check_at_startup(updates.check_at_startup);
    }
    if state.get_include_previews() != updates.previews {
        state.set_include_previews(updates.previews);
    }
}

/// Default handlers: the Options choices go to the presenter, the release page to the
/// desktop's browser, and the dialog opens and closes without an update service.
pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let state = window.global::<UpdateState>();
    {
        let window = window.as_weak();
        let presenter = Rc::downgrade(&presenter);
        let send = send.clone();
        state.on_options_changed(move |check_at_startup, previews| {
            let (Some(main), Some(presenter)) = (window.upgrade(), presenter.upgrade()) else {
                return;
            };
            if let Ok(mut p) = presenter.try_borrow_mut() {
                p.change_update_preferences(
                    midnight_core::UpdatePreferences {
                        check_at_startup,
                        previews,
                    },
                    send.as_ref(),
                );
            }
            crate::refresh(&main, &presenter.borrow());
        });
    }
    {
        let presenter = Rc::downgrade(&presenter);
        state.on_open_release_page(move || {
            let Some(presenter) = presenter.upgrade() else {
                return;
            };
            let url = PAGE_URL.with(|url| url.borrow().clone());
            let Ok(url) = midnight_core::ExternalUrl::parse(&url) else {
                return;
            };
            if let Ok(mut p) = presenter.try_borrow_mut() {
                p.open_external_url(url, send.as_ref(), Box::new(|_| {}));
            }
        });
    }
    {
        let window = window.as_weak();
        state.on_open_dialog(move |_check| {
            if let Some(main) = window.upgrade() {
                open_update_dialog(&main);
            }
        });
    }
    {
        let window = window.as_weak();
        state.on_close_dialog(move || {
            if let Some(main) = window.upgrade() {
                close_update_dialog(&main);
            }
        });
    }
}
