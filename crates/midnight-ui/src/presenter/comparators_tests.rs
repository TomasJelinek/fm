use super::*;
use std::{cell::RefCell, io};

// A backend test double for the deliberately simple '*' fixtures. Production
// presentation never constructs a compiled policy or a publication ID.
struct TestPolicy(Arc<FileComparatorPreferences>);
impl CompiledFileComparatorPolicy for TestPolicy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.0
    }
    fn select(&self, paths: &[PathBuf; 2]) -> Option<usize> {
        if paths
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()).is_none())
        {
            return None;
        }
        self.0
            .rules
            .iter()
            .position(|rule| rule.enabled && rule.expression == "*")
    }
    fn matcher_budget_bytes(&self) -> usize {
        0
    }
}

fn preferences(program: &str) -> FileComparatorPreferences {
    FileComparatorPreferences {
        rules: vec![FileComparatorRule {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*".into(),
            case_sensitive: false,
            target: FileComparatorTarget::External(FileComparatorProgram {
                executable: Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("_fixture")
                    .join(program),
                arguments: vec!["{left}".into(), "{right}".into()],
                working_directory: None,
            }),
        }],
    }
}

fn publish(
    store: &FileComparatorPolicyStore,
    preferences: FileComparatorPreferences,
) -> FileComparatorSnapshot {
    preferences.validate().unwrap();
    let id = store.prepare_id().unwrap();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(TestPolicy(Arc::new(preferences)))).with_id(id),
    )
}

struct Fixture {
    p: Presenter,
    store: FileComparatorPolicyStore,
}
fn ready() -> Fixture {
    ready_with(preferences("initial-comparator"))
}
fn ready_with(preferences: FileComparatorPreferences) -> Fixture {
    let store = FileComparatorPolicyStore::default();
    let snapshot = publish(&store, preferences.clone());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).to_owned();
    let mut p = Presenter::new(root.clone(), root);
    p.accepted_preferences_loaded(Ok(AcceptedPreferences {
        preferences: UserPreferences {
            file_comparators: preferences,
            ..Default::default()
        },
        comparators: snapshot,
    }));
    Fixture { p, store }
}

fn edit_program(p: &mut Presenter, program: &str) {
    p.select_comparator_rule(0);
    let mut form = p
        .comparators
        .draft
        .as_ref()
        .unwrap()
        .selected_rule()
        .unwrap()
        .form();
    form.executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("_fixture")
        .join(program)
        .to_string_lossy()
        .into_owned()
        .into();
    p.edit_comparator_form(form);
}

fn accept(p: &mut Presenter) -> (PreferencesRevision, UserPreferences) {
    let saved = RefCell::new(None);
    p.accept_options(&|command| {
        if let Command::SavePreferences {
            revision,
            preferences,
        } = command
        {
            *saved.borrow_mut() = Some((revision, *preferences));
        }
        true
    });
    saved
        .into_inner()
        .expect("OK did not submit the normal preferences transaction")
}

fn write_error() -> PreferencesApplyError {
    FsError::from_io(
        "save",
        Path::new("preferences"),
        io::Error::other("write refused"),
    )
    .into()
}

fn assert_launch(p: &Presenter, snapshot: &FileComparatorSnapshot) {
    let launch = p.file_compare_launch(PaneId::Left).unwrap();
    assert_eq!(launch.comparator_policy, snapshot.id);
    assert!(Arc::ptr_eq(&launch.comparators, &snapshot.preferences));
}

#[test]
fn incomplete_draft_reset_and_cancel_never_replace_accepted_launch_metadata() {
    let Fixture { mut p, store } = ready();
    let accepted = store.snapshot();
    p.begin_options();
    edit_program(&mut p, "draft-comparator");
    assert_launch(&p, &accepted);
    assert_eq!(p.preferences.values.file_comparators, *accepted.preferences);
    p.reset_options(&|_| panic!("Reset is staged, not persisted"));
    assert!(p.preferences.values.file_comparators.rules.is_empty());
    assert_launch(&p, &accepted);
    p.cancel_options(&|_| true);
    assert_eq!(p.preferences.values.file_comparators, *accepted.preferences);
    assert_launch(&p, &accepted);
    assert!(p.comparators.draft.is_none());
}

#[test]
fn failed_write_keeps_exact_draft_and_old_accepted_policy_for_retry() {
    let Fixture { mut p, store } = ready();
    let accepted = store.snapshot();
    p.begin_options();
    edit_program(&mut p, "draft-comparator");
    p.edit_comparator_argument(1, 0, "one two");
    let (revision, submitted) = accept(&mut p);
    p.accepted_preferences_saved(revision, Err(write_error()));
    assert!(p.preferences.options.open);
    assert!(p.comparators.dirty);
    assert_eq!(
        p.comparators.draft.as_ref().unwrap().candidate().unwrap(),
        submitted.file_comparators
    );
    assert_launch(&p, &accepted);
    let (retry, retried) = accept(&mut p);
    assert!(retry > revision);
    assert_eq!(retried.file_comparators, submitted.file_comparators);
}

#[test]
fn committed_older_success_is_used_even_when_newer_local_save_fails() {
    let Fixture { mut p, store } = ready();
    p.begin_options();
    edit_program(&mut p, "committed-comparator");
    let (committed_revision, submitted) = accept(&mut p);
    let later = RefCell::new(None);
    p.save_viewer_preferences(
        ViewerPreferenceChange::ComparisonFont("monospace".into()),
        &|command| {
            if let Command::SavePreferences { revision, .. } = command {
                *later.borrow_mut() = Some(revision);
            }
            true
        },
    );
    let later = later
        .into_inner()
        .expect("independent viewer setting did not enqueue");
    assert!(later > committed_revision);
    let accepted = publish(&store, submitted.file_comparators);
    p.accepted_preferences_saved(committed_revision, Ok(accepted.clone()));
    assert_launch(&p, &accepted);
    assert!(store.pin(&accepted).is_ok());
    p.accepted_preferences_saved(later, Err(write_error()));
    assert_launch(&p, &accepted);
}

#[test]
fn backend_disabled_regex_diagnostic_reveals_comparators_and_exact_field() {
    let Fixture {
        mut p,
        store: _store,
    } = ready();
    p.begin_options();
    p.select_comparator_rule(0);
    let mut form = p
        .comparators
        .draft
        .as_ref()
        .unwrap()
        .selected_rule()
        .unwrap()
        .form();
    form.enabled = false;
    form.mode = 1;
    form.expression = "(".into();
    p.edit_comparator_form(form);
    let (revision, _) = accept(&mut p); // Raw Core validation does not compile regex.
    let diagnostic = FileComparatorDiagnostic {
        rule: Some(0),
        field: FileComparatorField::Expression,
        message: "Unclosed regular expression group".into(),
    };
    p.accepted_preferences_saved(revision, Err(diagnostic.into()));
    assert!(p.preferences.options.open);
    assert_eq!(p.preferences.options.error_topic, Some(10));
    assert_eq!(p.comparators.error_field, 2);
    assert_eq!(
        p.comparators.draft.as_ref().unwrap().selected_index(),
        Some(0)
    );
    assert!(
        !p.comparators
            .draft
            .as_ref()
            .unwrap()
            .selected_rule()
            .unwrap()
            .enabled
    );
    assert_eq!(
        p.comparators
            .draft
            .as_ref()
            .unwrap()
            .selected_rule()
            .unwrap()
            .expression,
        "("
    );
}

#[test]
fn stale_diagnostic_cannot_focus_a_reordered_newer_draft() {
    let Fixture {
        mut p,
        store: _store,
    } = ready();
    p.begin_options();
    edit_program(&mut p, "draft");
    p.comparator_rule_action(0);
    let (revision, _) = accept(&mut p);
    p.accepted_preferences_saved(revision, Err(write_error()));
    p.comparator_rule_action(2);
    let selected = p.comparators.draft.as_ref().unwrap().selected;
    let diagnostic = FileComparatorDiagnostic {
        rule: Some(0),
        field: FileComparatorField::Executable,
        message: "Old error".into(),
    };
    p.accepted_preferences_saved(revision, Err(diagnostic.into()));
    assert_eq!(p.comparators.draft.as_ref().unwrap().selected, selected);
    assert!(p.comparators.error.is_empty());
    assert_eq!(p.comparators.error_field, -1);
}

#[test]
fn coverage_requests_are_once_per_startup_snapshot_and_reject_stale_echoes() {
    let Fixture {
        mut p,
        store: _store,
    } = ready();
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    p.request_comparator_capabilities(false, &send);
    p.request_comparator_capabilities(false, &send);
    p.begin_options();
    p.request_comparator_capabilities(false, &send);
    assert_eq!(sent.borrow().len(), 1);
    {
        let commands = sent.borrow();
        let Command::ListComparatorCapabilities { disabled_plugins } = &commands[0] else {
            panic!()
        };
        assert!(Arc::ptr_eq(
            disabled_plugins,
            p.viewer_plugins.startup_disabled.as_ref().unwrap()
        ));
    }
    p.comparator_capabilities_listed(ComparatorCapabilities {
        revision: 1,
        disabled_plugins: Arc::from(["wrong".into()]),
        images: Ok(Arc::from([])),
    });
    assert!(p.comparators.capabilities.pending);
    assert!(p.comparators.capabilities.snapshot.is_none());
    p.comparator_capabilities_listed(ComparatorCapabilities {
        revision: 2,
        disabled_plugins: Arc::from([]),
        images: Ok(Arc::from([])),
    });
    assert!(!p.comparators.capabilities.pending);
    assert!(
        p.comparators
            .capabilities
            .snapshot
            .as_ref()
            .unwrap()
            .images
            .as_ref()
            .unwrap()
            .is_empty()
    );
    p.comparator_capabilities_listed(ComparatorCapabilities {
        revision: 1,
        disabled_plugins: Arc::from([]),
        images: Err("stale".into()),
    });
    assert_eq!(
        p.comparators
            .capabilities
            .snapshot
            .as_ref()
            .unwrap()
            .revision,
        2
    );
}

#[test]
fn picker_reply_after_topic_cancel_does_not_replace_external_program() {
    let Fixture {
        mut p,
        store: _store,
    } = ready();
    p.begin_options();
    edit_program(&mut p, "draft");
    let before = p.comparators.draft.as_ref().unwrap().candidate().unwrap();
    let request = RefCell::new(None);
    p.request_settings_picker(
        settings_picker::Target::ComparatorProgram,
        PathPickerOwner::None,
        &|command| {
            if let Command::PickPath { request: id, .. } = command {
                *request.borrow_mut() = Some(id);
            }
            true
        },
    );
    let request = request.into_inner().unwrap();
    p.settings_picker_context(9, &|_| true);
    p.settings_path_picked(
        request,
        Ok(Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("other"))),
    );
    assert_eq!(
        p.comparators.draft.as_ref().unwrap().candidate().unwrap(),
        before
    );
}

#[test]
fn capability_display_memo_preserves_form_and_request_state_until_new_metadata() {
    use slint::{
        ComponentHandle, Model, SharedString, VecModel,
        platform::{
            Platform, PlatformError, WindowAdapter,
            software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
        },
    };
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    struct TestPlatform;
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
        }
    }
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let window = crate::MainWindow::new().unwrap();
    let Fixture { mut p, store: _ } = ready();
    p.begin_options();
    p.select_comparator_rule(0);
    let p = Rc::new(RefCell::new(p));
    let admitted = Rc::new(Cell::new(true));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let (accept, commands) = (admitted.clone(), sent.clone());
    crate::comparator_options_presentation::install(
        &window,
        p.clone(),
        Rc::new(move |command| {
            commands.borrow_mut().push(command);
            accept.get()
        }),
    );
    let present = || crate::comparator_options_presentation::refresh(&window, &p.borrow());
    let get_memo = || p.borrow().comparators.capabilities.display.clone().unwrap();
    let echo = p.borrow().viewer_plugins.startup_disabled.clone().unwrap();
    let view = window.global::<crate::ComparatorOptionsView>();
    let models = || (view.get_labels(), view.get_argument_rows());
    let display = || {
        (
            view.get_image_coverage(),
            view.get_coverage_state(),
            view.get_coverage_error(),
        )
    };
    let check_display = |expected: (&str, &str, &str)| {
        let (coverage, status, error) = display();
        assert_eq!(
            (coverage.as_str(), status.as_str(), error.as_str()),
            expected
        );
    };
    let record = |revision, images| ComparatorCapabilities {
        revision,
        disabled_plugins: Arc::from([]),
        images,
    };
    let images = |extensions: &[&str]| -> Arc<[ComparatorImageCapability]> {
        Arc::from([ComparatorImageCapability {
            provider_id: "test.image-decoder".into(),
            extensions: extensions
                .iter()
                .map(|extension| (*extension).into())
                .collect(),
        }])
    };
    let state = || {
        let p = p.borrow();
        let caps = &p.comparators.capabilities;
        (
            p.comparators.revision,
            caps.pending,
            caps.requested,
            caps.error.clone(),
            caps.snapshot.as_ref().map(|snapshot| snapshot.revision),
        )
    };
    present();
    check_display(("", "Image support metadata has not been received.", ""));
    view.invoke_refresh_coverage();
    assert!(view.get_coverage_busy());
    assert!(p.borrow().comparators.capabilities.display.is_none());
    check_display(("", "Checking enabled image support…", ""));
    p.borrow_mut()
        .comparator_capabilities_listed(record(10, Ok(images(&["zeta", ".png", "avif", "..png"]))));
    present();
    let memo = get_memo();
    let accepted = display();
    assert_eq!(accepted.0, "*.avif; *.png; *.zeta");
    let (labels, arguments) = models();
    assert!(labels.as_any().is::<VecModel<SharedString>>());
    assert!(arguments.as_any().is::<VecModel<SharedString>>());
    let same_identity = || {
        assert!(Arc::ptr_eq(&get_memo(), &memo));
        assert_eq!(models(), (labels.clone(), arguments.clone()));
    };
    let same_display = || {
        same_identity();
        assert_eq!(display(), accepted);
        assert_eq!(
            view.get_image_coverage().as_str().as_ptr(),
            memo.coverage.as_str().as_ptr()
        );
    };
    same_display();
    view.invoke_argument(0, 0, " --literal {left} 世界 ".into());
    assert_eq!(arguments.row_data(0).unwrap(), " --literal {left} 世界 ");
    same_display();
    let mut form = view.get_draft();
    form.working_directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("memo cwd 文档")
        .to_string_lossy()
        .into_owned()
        .into();
    view.invoke_edit(form.clone());
    assert_eq!(view.get_draft().working_directory, form.working_directory);
    same_display();
    for row in [1, 0] {
        view.invoke_select(row);
        assert_eq!(view.get_selected(), row);
        same_display();
    }
    assert_eq!(arguments.row_data(0).unwrap(), " --literal {left} 世界 ");

    // Refusal retains the memo under an overlay; an admitted retry reveals it.
    for refused in [true, false] {
        admitted.set(!refused);
        view.invoke_refresh_coverage();
        same_identity();
        assert_eq!(view.get_coverage_busy(), !refused);
        if refused {
            check_display((
                "",
                "Image support is unavailable.",
                "Image support metadata is busy. Try Refresh again.",
            ));
        } else {
            same_display();
        }
        let before = state();
        let shown = display();
        let mut wrong_echo = record(11, Err("wrong echo".into()));
        wrong_echo.disabled_plugins = Arc::from(["wrong.startup.echo".into()]);
        for rejected in [
            record(9, Err("stale".into())),
            record(10, Err("equal".into())),
            wrong_echo,
        ] {
            p.borrow_mut().comparator_capabilities_listed(rejected);
            present();
            assert_eq!(state(), before);
            same_identity();
            assert_eq!(display(), shown);
            assert_eq!(view.get_coverage_busy(), !refused);
        }
    }
    let count = sent.borrow().len();
    admitted.set(false);
    view.invoke_refresh_coverage();
    assert_eq!(sent.borrow().len(), count, "pending requests must coalesce");
    assert!(view.get_coverage_busy());
    same_display();

    let available = "Coverage comes from image decoders enabled for this session. Some file pairs may still be unsupported. Plugin changes take effect after restart.";
    let no_providers = "No image decoder is enabled for this session.";
    let no_extensions = "Enabled image decoders advertise no filename extensions.";
    let unavailable = "Image support is unavailable.";
    let failure = "decoder metadata failed 文档";
    let cases = [
        (11, Ok(images(&["jpeg"])), "*.jpeg", available, ""),
        (12, Ok(Arc::from([])), "", no_providers, ""),
        (13, Ok(images(&[])), "", no_extensions, ""),
        (14, Err(failure.into()), "", unavailable, failure),
    ];
    for (revision, metadata, coverage, status, error) in cases {
        let previous = Arc::downgrade(&get_memo());
        let previous_images = p
            .borrow()
            .comparators
            .capabilities
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.images.as_ref().ok())
            .map(Arc::downgrade);
        let next = record(revision, metadata);
        assert_eq!(echo.as_ref(), next.disabled_plugins.as_ref());
        assert!(!Arc::ptr_eq(&echo, &next.disabled_plugins));
        p.borrow_mut().comparator_capabilities_listed(next);
        present();
        assert!(!Arc::ptr_eq(&previous.upgrade().unwrap(), &get_memo()));
        assert_eq!(state().4, Some(revision));
        assert!(!view.get_coverage_busy());
        check_display((coverage, status, error));
        assert_eq!(models(), (labels.clone(), arguments.clone()));
        if revision == 12 {
            // The preceding JPEG record has no other strong memo/metadata owners.
            let previous_images = previous_images.unwrap();
            assert!(previous.upgrade().is_some() && previous_images.upgrade().is_some());
            let retired = p.borrow_mut().take_retired();
            std::thread::spawn(move || drop(retired)).join().unwrap();
            assert!(previous.upgrade().is_none() && previous_images.upgrade().is_none());
        }
    }
    let memo = get_memo();
    admitted.set(true);
    view.invoke_refresh_coverage();
    assert!(view.get_coverage_busy());
    assert!(Arc::ptr_eq(&get_memo(), &memo));
    check_display(("", unavailable, failure));
}

#[cfg(any(unix, windows))]
use crate::raw_picker_paths;

#[cfg(any(unix, windows))]
#[test]
fn native_picker_replies_adopt_raw_aliases_and_ignore_identical_values() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("_fixture");
    for directory in [false, true] {
        let paths = raw_picker_paths::same_display(&root);
        let [original, different] = if directory {
            paths
        } else {
            paths.map(|path| path.join("compare.exe"))
        };
        assert_ne!(original.as_os_str(), different.as_os_str());
        assert_eq!(original.to_string_lossy(), different.to_string_lossy());
        let mut preferences = preferences("initial-comparator");
        let FileComparatorTarget::External(program) = &mut preferences.rules[0].target else {
            unreachable!();
        };
        if directory {
            program.working_directory = Some(original.clone());
        } else {
            program.executable = original.clone();
        }
        let Fixture { mut p, store } = ready_with(preferences);
        let accepted = store.snapshot();
        p.begin_options();
        p.select_comparator_rule(0);
        let mut expected_dirty = false;
        for (chosen, changed) in [(&original, false), (&different, true), (&different, false)] {
            let before = (p.comparators.revision, p.comparators.edit_revision);
            let request = RefCell::new(None);
            p.request_settings_picker(
                if directory {
                    settings_picker::Target::ComparatorWorkingDirectory
                } else {
                    settings_picker::Target::ComparatorProgram
                },
                PathPickerOwner::None,
                &|command| {
                    let Command::PickPath { request: id, input } = command else {
                        panic!("Browse must publish a typed PickPath request");
                    };
                    assert_eq!(
                        input.kind,
                        if directory {
                            PathPickerKind::Directory
                        } else {
                            PathPickerKind::Program
                        }
                    );
                    assert!(request.borrow_mut().replace(id).is_none());
                    true
                },
            );
            assert!(p.settings_picker.busy());
            p.settings_path_picked(request.into_inner().unwrap(), Ok(Some(chosen.clone())));
            assert!(!p.settings_picker.busy());
            let increment = u64::from(changed);
            assert_eq!(
                (p.comparators.revision, p.comparators.edit_revision),
                (before.0 + increment, before.1 + increment)
            );
            expected_dirty |= changed;
            assert_eq!(p.comparators.dirty, expected_dirty);
            let draft = p.comparators.draft.as_ref().unwrap();
            let external = &draft.selected_rule().unwrap().external;
            let (display, native) = if directory {
                (
                    &external.working_directory,
                    external.native_working_directory.as_deref(),
                )
            } else {
                (&external.executable, external.native_executable.as_deref())
            };
            assert_eq!(display.as_str(), chosen.to_string_lossy().as_ref());
            assert_eq!(native.unwrap().as_os_str(), chosen.as_os_str());
            assert_launch(&p, &accepted);
        }
    }
}
