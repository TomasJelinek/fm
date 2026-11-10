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

#[test]
fn custom_associations_capture_exact_decoder_and_keep_capability_gating() {
    let mut presenter = Presenter::new("left".into(), "right".into());
    let mut saved = UserPreferences::default();
    saved
        .archive_custom_extensions
        .insert("existing".into(), ArchiveFormat::Tar);
    assert!(
        presenter
            .add_archive_extension("early", ArchiveFormat::Zip, &|_| panic!())
            .is_err()
    );
    presenter.preferences_loaded(Ok(saved));
    let saves = RefCell::new(Vec::new());
    let send = |command| {
        if let Command::SavePreferences { preferences, .. } = command {
            saves.borrow_mut().push(*preferences);
        }
        true
    };
    for (index, format) in midnight_core::ARCHIVE_FORMATS.iter().copied().enumerate() {
        let extension = format!("custom{index}");
        presenter
            .add_archive_extension(&extension, format, &send)
            .unwrap();
        let entry = Entry::new(
            EntryId(1),
            format!("文档.{}", extension.to_uppercase()).into(),
            EntryKind::File,
        );
        presenter.archive_formats.clear();
        assert!(!presenter.can_browse_archive(&entry));
        presenter.archive_formats = vec![format];
        assert_eq!(presenter.browsable_archive_format(&entry), Some(format));
        for kind in [
            EntryKind::Directory,
            EntryKind::DirectoryLink,
            EntryKind::Symlink,
            EntryKind::Other,
        ] {
            assert!(!presenter.can_browse_archive(&Entry::new(
                EntryId(2),
                entry.path.clone(),
                kind
            )));
        }
    }
    for invalid in [".DOCX", "CUSTOM0", "*.zip", "../zip"] {
        let before = presenter.preferences.values.clone();
        assert!(
            presenter
                .add_archive_extension(invalid, ArchiveFormat::Zstd, &send)
                .is_err()
        );
        assert_eq!(presenter.preferences.values, before);
    }
    presenter.remove_archive_extension("custom0", ArchiveFormat::Tar, &send);
    assert!(
        presenter
            .preferences
            .values
            .archive_custom_extensions
            .contains_key("custom0")
    );
    presenter.remove_archive_extension("custom0", ArchiveFormat::Zip, &send);
    assert!(
        !presenter
            .preferences
            .values
            .archive_custom_extensions
            .contains_key("custom0")
    );
    assert_eq!(
        presenter.preferences.values.archive_custom_extensions["existing"],
        ArchiveFormat::Tar
    );
    assert_eq!(
        saves.borrow().last().unwrap(),
        &presenter.preferences.values
    );
}
