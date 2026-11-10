use super::*;
use std::{cell::RefCell, io};

fn loaded() -> Presenter {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.preferences_loaded(Ok(UserPreferences::default()));
    presenter
}

#[test]
fn explicit_save_retries_a_failed_backend_write_with_a_new_revision() {
    let mut presenter = loaded();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        let Command::SavePreferences {
            revision,
            preferences,
        } = command
        else {
            panic!("Expected preferences persistence");
        };
        sent.borrow_mut().push((revision, preferences));
        true
    };
    presenter.change_preferences(1, 0, 0, &send);
    let first = sent.borrow()[0].0;
    presenter.preferences_saved(
        first,
        Err(FsError::from_io(
            "save preferences",
            Path::new("options"),
            io::Error::from(io::ErrorKind::PermissionDenied),
        )),
    );
    assert!(!presenter.preferences.saving);
    assert!(presenter.preferences.unsaved);
    assert!(!presenter.preferences.save_error.is_empty());
    presenter.change_preferences(2, 1, 2, &send);
    let retry = sent.borrow()[1].0;
    assert!(
        retry > first,
        "The backend coalesces an already accepted revision"
    );
    assert_eq!(sent.borrow()[0].1, sent.borrow()[1].1);
    assert!(presenter.preferences.saving);
    assert!(presenter.preferences.save_error.is_empty());
    // A queued failure/success from the first request must not finish the retry.
    presenter.preferences_saved(first, Ok(()));
    assert!(presenter.preferences.saving);
    assert!(presenter.preferences.unsaved);
    presenter.preferences_saved(retry, Ok(()));
    assert!(!presenter.preferences.saving);
    assert!(!presenter.preferences.unsaved);
    assert_eq!(presenter.preferences_save_status(), "Changes saved");
}

#[test]
fn save_coalesces_clicks_while_busy_and_preserves_values_on_queue_failure() {
    let mut presenter = loaded();
    let values = presenter.preferences.values.clone();
    presenter.change_preferences(2, 1, 2, &|_| false);
    assert_eq!(presenter.preferences.values, values);
    assert!(presenter.preferences.unsaved);
    assert!(!presenter.preferences.saving);
    assert!(presenter.preferences_save_status().contains("queue"));
    presenter.change_preferences(2, 1, 2, &|_| true);
    let revision = presenter.preferences.revision;
    presenter.change_preferences(0, 0, 2, &|_| panic!("Already saving"));
    assert_eq!(presenter.preferences.revision, revision);
    assert!(presenter.preferences.saving);
}

#[test]
fn save_waits_for_initial_load_and_never_overwrites_an_unreadable_file() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.change_preferences(0, 0, 2, &|_| panic!("Settings not loaded"));
    assert_eq!(presenter.preferences_save_status(), "Loading settings…");
    presenter.preferences_loaded(Err(FsError::from_io(
        "load preferences",
        Path::new("options"),
        io::Error::from(io::ErrorKind::PermissionDenied),
    )));
    presenter.change_preferences(0, 0, 2, &|_| panic!("Existing file is protected"));
    assert!(!presenter.preferences.saving);
    assert!(
        presenter
            .preferences_save_status()
            .contains("Saving is disabled")
    );
}
