use super::*;
use std::cell::RefCell;

#[test]
fn early_archive_choices_override_only_their_saved_extensions() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    let mut local = presenter.preferences.values.clone();
    local
        .archive_association_overrides
        .insert("docx".into(), true);
    local
        .archive_association_overrides
        .insert("zip".into(), false);
    presenter.update_preferences(local, &|_| panic!("wait for initial preferences"));
    let mut loaded = UserPreferences {
        theme: PreferenceTheme::Nord,
        ..Default::default()
    };
    loaded
        .archive_association_overrides
        .insert("docx".into(), false);
    loaded
        .archive_association_overrides
        .insert("xlsx".into(), true);
    loaded
        .archive_association_overrides
        .insert("tar".into(), false);
    presenter.preferences_loaded(Ok(loaded));
    let values = &presenter.preferences.values;
    assert!(values.archive_association_enabled("docx"));
    assert!(!values.archive_association_enabled("zip"));
    assert!(values.archive_association_enabled("xlsx"));
    assert!(!values.archive_association_enabled("tar"));
    assert_eq!(values.theme, PreferenceTheme::Nord);
    let saved = RefCell::new(Vec::new());
    presenter.flush_loaded_preferences(&|command| {
        if let Command::SavePreferences { preferences, .. } = command {
            saved.borrow_mut().push(*preferences);
        }
        true
    });
    assert_eq!(
        saved.borrow().as_slice(),
        &[presenter.preferences.values.clone()]
    );
}

#[test]
fn returning_to_default_before_load_overrides_saved_choice_without_losing_others() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    let no_save = |_: Command| panic!("wait for initial preferences");
    presenter.set_archive_association_enabled("docx", true, &no_save);
    presenter.set_archive_association_enabled("docx", false, &no_save);
    let mut loaded = UserPreferences::default();
    loaded
        .archive_association_overrides
        .insert("docx".into(), true);
    loaded
        .archive_association_overrides
        .insert("xlsx".into(), true);
    presenter.preferences_loaded(Ok(loaded));
    assert!(
        !presenter
            .preferences
            .values
            .archive_association_enabled("docx")
    );
    assert!(
        presenter
            .preferences
            .values
            .archive_association_enabled("xlsx")
    );
    assert!(
        !presenter
            .preferences
            .values
            .archive_association_overrides
            .contains_key("docx")
    );
}

#[test]
fn toggling_document_browsing_applies_immediately_and_saves_without_restart() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    presenter.preferences_loaded(Ok(UserPreferences::default()));
    presenter.archive_formats = vec![ArchiveFormat::Zip];
    let entry = Entry::new(EntryId(1), "document.DOCX".into(), EntryKind::File);
    assert!(!presenter.can_browse_archive(&entry));
    let saved = RefCell::new(Vec::new());
    for enabled in [true, false] {
        presenter.set_archive_association_enabled("docx", enabled, &|command| {
            let Command::SavePreferences { preferences, .. } = command else {
                panic!()
            };
            saved.borrow_mut().push(*preferences);
            true
        });
        assert_eq!(presenter.can_browse_archive(&entry), enabled);
        assert_eq!(
            saved
                .borrow()
                .last()
                .unwrap()
                .archive_association_enabled("docx"),
            enabled
        );
    }
    assert_eq!(saved.borrow().len(), 2);
    let revision = presenter.preferences.revision;
    presenter.set_archive_association_enabled("DOCX", true, &|_| panic!("unknown choice"));
    assert_eq!(presenter.preferences.revision, revision);
}
