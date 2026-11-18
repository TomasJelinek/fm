//! Comparator configuration and accepted-policy boundaries; no filesystem or matcher compiler.
use midnight_core::*;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Weak, mpsc},
    time::Duration,
};

fn absolute(name: &str) -> PathBuf {
    if cfg!(windows) {
        Path::new("C:\\comparator-tests").join(name)
    } else {
        Path::new("/comparator-tests").join(name)
    }
}

fn program() -> FileComparatorProgram {
    FileComparatorProgram {
        executable: absolute("tool.exe"),
        arguments: vec!["{left}".into(), "{right}".into()],
        working_directory: None,
    }
}

fn preferences(target: FileComparatorTarget) -> FileComparatorPreferences {
    FileComparatorPreferences {
        rules: vec![FileComparatorRule {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.txt".into(),
            case_sensitive: false,
            target,
        }],
    }
}

#[test]
fn literal_whole_argv_expansion_preserves_order_repetition_whitespace_and_empty_items() {
    let paths = [
        absolute("before 文档 & $x.txt"),
        absolute("after e\u{301}.txt"),
    ];
    let mut value = program();
    value.arguments = vec![
        "{right}".into(),
        "--left={left}".into(),
        "{left}".into(),
        " \"quoted\";$HOME ".into(),
        String::new(),
        "{left}".into(),
    ];
    assert_eq!(
        value.arguments_for(&paths).unwrap(),
        vec![
            paths[1].as_os_str().to_owned(),
            OsString::from("--left={left}"),
            paths[0].as_os_str().to_owned(),
            OsString::from(" \"quoted\";$HOME "),
            OsString::new(),
            paths[0].as_os_str().to_owned(),
        ]
    );
}

#[test]
fn process_is_program_only_and_uses_executable_parent_or_explicit_native_cwd() {
    let paths = [absolute("a.txt"), absolute("b.txt")];
    let mut value = program();
    let process = value.process_for(&paths).unwrap();
    assert_eq!(process.as_process().working_directory, absolute(""));
    match &process.as_process().invocation {
        UserCommandProcessInvocation::Program {
            executable,
            arguments,
        } => {
            assert_eq!(executable, &absolute("tool.exe"));
            assert_eq!(arguments, &paths.map(PathBuf::into_os_string));
        }
        UserCommandProcessInvocation::Terminal { .. } => panic!("comparator selected a shell"),
    }
    value.working_directory = Some(absolute("explicit 文档"));
    assert_eq!(
        value
            .process_for(&[absolute("a"), absolute("b")])
            .unwrap()
            .as_process()
            .working_directory,
        absolute("explicit 文档")
    );
    let debug = format!("{process:?}");
    assert!(!debug.contains("comparator-tests"));
    assert!(!debug.contains("tool.exe"));
    let raw_debug = format!("{value:?}");
    assert!(!raw_debug.contains("comparator-tests"));
    assert!(!raw_debug.contains("tool.exe"));
    assert!(!raw_debug.contains("{left}"));
}

#[test]
fn both_whole_placeholders_and_absolute_nul_free_paths_are_required() {
    let paths = [absolute("a"), absolute("b")];
    for arguments in [vec!["{left}"], vec!["{right}"], vec!["x{left}", "{right}"]] {
        let mut value = program();
        value.arguments = arguments.into_iter().map(str::to_owned).collect();
        assert!(value.arguments_for(&paths).is_err());
    }
    for executable in [PathBuf::from("tool.exe"), absolute("bad\0.exe")] {
        let mut value = program();
        value.executable = executable;
        assert!(value.arguments_for(&paths).is_err());
    }
    for directory in [PathBuf::from("relative"), absolute("bad\0cwd")] {
        let mut value = program();
        value.working_directory = Some(directory);
        assert!(value.arguments_for(&paths).is_err());
    }
    assert!(
        program()
            .arguments_for(&[PathBuf::from("a"), absolute("b")])
            .is_err()
    );
    assert!(
        program()
            .arguments_for(&[absolute("a"), absolute("bad\0b")])
            .is_err()
    );
    let mut value = program();
    value.arguments.push("literal\0value".into());
    assert!(value.arguments_for(&paths).is_err());
}

#[test]
fn argument_byte_and_count_limits_are_template_limits_not_expanded_path_limits() {
    let mut value = program();
    value.arguments.extend(vec![String::new(); 62]);
    let paths = [absolute(&"a".repeat(40_000)), absolute("b")];
    assert!(value.arguments_for(&paths).is_ok());
    value.arguments.push(String::new());
    assert!(value.arguments_for(&paths).is_err());
    value = program();
    value.arguments.push("x".repeat(32_768 - 13)); // 6-byte left + 7-byte right.
    assert!(value.arguments_for(&paths).is_ok());
    value.arguments[2].push('x');
    assert!(value.arguments_for(&paths).is_err());
    let huge = [absolute(&"x".repeat(8 * 1024 * 1024)), absolute("b")];
    assert!(program().process_for(&huge).is_err());
}

#[test]
fn disabled_rows_are_bounded_with_row_and_field_diagnostics_without_compiling_regex() {
    let mut value = preferences(FileComparatorTarget::Automatic);
    value.rules[0].enabled = false;
    value.rules[0].mode = FilterMode::Regex;
    value.rules[0].expression = "[".into();
    assert!(value.validate().is_ok(), "Core must not compile regexes");
    value.rules[0].mode = FilterMode::Expression;
    let error = value.validate().unwrap_err();
    assert_eq!(error.rule, Some(0));
    assert_eq!(error.field, FileComparatorField::PatternMode);
    value.rules[0].mode = FilterMode::Masks;
    for expression in [
        String::new(),
        " \t ".into(),
        "a\0b".into(),
        "x".repeat(4097),
    ] {
        value.rules[0].expression = expression;
        assert_eq!(
            value.validate().unwrap_err().field,
            FileComparatorField::Expression
        );
    }
    value.rules[0].expression = "x".repeat(4096);
    assert!(value.validate().is_ok());
    value.rules = vec![value.rules[0].clone(); 64];
    assert!(value.validate().is_ok());
    value.rules.push(value.rules[0].clone());
    let error = value.validate().unwrap_err();
    assert_eq!(error.rule, None);
    assert_eq!(error.field, FileComparatorField::Rules);
}

#[test]
fn external_row_diagnostics_and_user_preferences_reject_invalid_disabled_configuration() {
    let mut value = preferences(FileComparatorTarget::External(program()));
    value.rules[0].enabled = false;
    let FileComparatorTarget::External(program) = &mut value.rules[0].target else {
        unreachable!()
    };
    program.working_directory = Some("relative".into());
    let diagnostic = value.validate().unwrap_err();
    assert_eq!(diagnostic.rule, Some(0));
    assert_eq!(diagnostic.field, FileComparatorField::WorkingDirectory);
    let settings = UserPreferences {
        file_comparators: value,
        ..Default::default()
    };
    assert!(settings.validate().is_err());
    assert!(UserPreferences::default().validate().is_ok());
}

struct Policy {
    raw: Arc<FileComparatorPreferences>,
    on_drop: Option<(Weak<FileComparatorPolicyStore>, mpsc::Sender<String>)>,
}

impl CompiledFileComparatorPolicy for Policy {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.raw
    }
    fn select(&self, _: &[PathBuf; 2]) -> Option<usize> {
        Some(0)
    }
    fn matcher_budget_bytes(&self) -> usize {
        1024
    }
}

impl Drop for Policy {
    fn drop(&mut self) {
        if let Some((store, sender)) = &self.on_drop
            && let Some(store) = store.upgrade()
        {
            let _ = sender.send(store.snapshot().preferences.rules[0].expression.clone());
        }
    }
}

fn prepared(
    store: &FileComparatorPolicyStore,
    target: FileComparatorTarget,
) -> PreparedFileComparatorPolicy {
    PreparedFileComparatorPolicy::new(Arc::new(Policy {
        raw: Arc::new(preferences(target)),
        on_drop: None,
    }))
    .with_id(store.prepare_id().unwrap())
}

#[test]
fn default_launch_snapshot_pins_default_store_without_compiled_history() {
    let store = FileComparatorPolicyStore::default();
    let snapshot = FileComparatorSnapshot::default();
    let accepted = store.pin(&snapshot).unwrap();
    assert!(accepted.preferences().rules.is_empty());
    assert_eq!(accepted.select(&[absolute("a"), absolute("b")]), None);
    assert!(
        accepted
            .validation_proof()
            .matches(&FileComparatorPreferences::default())
    );
}

#[test]
fn pin_requires_coherent_raw_identity_and_key_and_existing_handle_stays_frozen() {
    let store = FileComparatorPolicyStore::default();
    let first = store.publish(prepared(&store, FileComparatorTarget::BuiltInText));
    let accepted = store.pin(&first).unwrap();
    let reconstructed = FileComparatorSnapshot {
        id: first.id,
        preferences: Arc::new((*first.preferences).clone()),
    };
    assert!(store.pin(&reconstructed).is_err());
    let second = store.publish(prepared(&store, FileComparatorTarget::BuiltInImages));
    assert!(store.pin(&first).is_err());
    assert!(
        store
            .pin(&FileComparatorSnapshot {
                id: first.id,
                preferences: second.preferences.clone()
            })
            .is_err()
    );
    assert_eq!(
        accepted.preferences().rules[0].target,
        FileComparatorTarget::BuiltInText
    );
    assert_eq!(
        store.pin(&second).unwrap().preferences().rules[0].target,
        FileComparatorTarget::BuiltInImages
    );
}

#[test]
fn abandoned_replacement_does_not_publish_and_writer_reuse_preserves_id_and_raw_arc() {
    let store = FileComparatorPolicyStore::default();
    let first = store.publish(prepared(&store, FileComparatorTarget::BuiltInText));
    drop(prepared(&store, FileComparatorTarget::BuiltInImages)); // compile/save rejection
    assert_eq!(store.snapshot().id, first.id);
    let reused = store.reuse_for_save(&first.preferences).unwrap();
    assert_eq!(reused.snapshot().id, first.id);
    assert!(Arc::ptr_eq(reused.preferences(), &first.preferences));
    assert!(
        reused
            .validation_proof()
            .matches(&(*first.preferences).clone())
    );
    assert!(
        !reused
            .validation_proof()
            .matches(&preferences(FileComparatorTarget::Automatic))
    );
    assert!(
        store
            .reuse_for_save(&preferences(FileComparatorTarget::BuiltInImages))
            .is_none()
    );
}

#[test]
fn metadata_snapshot_and_proof_do_not_create_a_compiled_revision_history() {
    let store = FileComparatorPolicyStore::default();
    let compiled = Arc::new(Policy {
        raw: Arc::new(preferences(FileComparatorTarget::Automatic)),
        on_drop: None,
    });
    let weak = Arc::downgrade(&compiled);
    let first = store
        .publish(PreparedFileComparatorPolicy::new(compiled).with_id(store.prepare_id().unwrap()));
    let accepted = store.pin(&first).unwrap();
    assert!(accepted.validation_proof().matches(&first.preferences));
    store.publish(prepared(&store, FileComparatorTarget::BuiltInText));
    assert!(weak.upgrade().is_some());
    drop(accepted);
    assert!(
        weak.upgrade().is_none(),
        "raw snapshot must not pin the compiler"
    );
    assert_eq!(
        first.preferences.rules[0].target,
        FileComparatorTarget::Automatic
    );
}

#[test]
fn displaced_compiler_destructor_runs_after_unlock_and_observes_new_publication() {
    let store = Arc::new(FileComparatorPolicyStore::default());
    let (sender, receiver) = mpsc::channel();
    let compiled = Arc::new(Policy {
        raw: Arc::new(preferences(FileComparatorTarget::Automatic)),
        on_drop: Some((Arc::downgrade(&store), sender)),
    });
    store.publish(PreparedFileComparatorPolicy::new(compiled).with_id(store.prepare_id().unwrap()));
    let mut replacement = preferences(FileComparatorTarget::BuiltInText);
    replacement.rules[0].expression = "new-publication".into();
    let candidate = PreparedFileComparatorPolicy::new(Arc::new(Policy {
        raw: Arc::new(replacement),
        on_drop: None,
    }))
    .with_id(store.prepare_id().unwrap());
    let worker = std::thread::spawn(move || store.publish(candidate));
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(3)).unwrap(),
        "new-publication"
    );
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn unix_raw_native_paths_survive_expansion_and_cannot_authorize_custom_selection() {
    use std::os::unix::ffi::OsStringExt;
    let paths = [
        absolute("a"),
        absolute("").join(OsString::from_vec(vec![b'b', 0xff])),
    ];
    assert_eq!(
        program().arguments_for(&paths).unwrap()[1],
        paths[1].as_os_str()
    );
    let store = FileComparatorPolicyStore::default();
    let snapshot = store.publish(prepared(&store, FileComparatorTarget::External(program())));
    assert_eq!(store.pin(&snapshot).unwrap().select(&paths), None);
}

#[cfg(windows)]
#[test]
fn windows_raw_native_paths_survive_expansion_and_cannot_authorize_custom_selection() {
    use std::os::windows::ffi::OsStringExt;
    let paths = [
        absolute("a"),
        absolute("").join(OsString::from_wide(&[b'b' as u16, 0xd800])),
    ];
    assert_eq!(
        program().arguments_for(&paths).unwrap()[1],
        paths[1].as_os_str()
    );
    let store = FileComparatorPolicyStore::default();
    let snapshot = store.publish(prepared(&store, FileComparatorTarget::External(program())));
    assert_eq!(store.pin(&snapshot).unwrap().select(&paths), None);
}

struct ProtectedOwner {
    drops: Arc<std::sync::atomic::AtomicUsize>,
}

impl ExternalCompareOwnedPayload for ProtectedOwner {
    fn as_any(&self) -> &(dyn std::any::Any + Send) {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }
}

impl Drop for ProtectedOwner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn opaque_request_preserves_owner_for_type_inspection_reboxing_and_command_drop() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let drops = Arc::new(AtomicUsize::new(0));
    let request = ExternalCompareRequest::from_backend_owner(Box::new(ProtectedOwner {
        drops: drops.clone(),
    }));
    assert_eq!(format!("{request:?}"), "ExternalCompareRequest { .. }");
    let owner = request.into_backend_owner();
    assert!(owner.as_any().is::<ProtectedOwner>());
    assert!(!owner.as_any().is::<String>());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let command =
        Command::LaunchExternalComparison(ExternalCompareRequest::from_backend_owner(owner));
    drop(command);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn known_opaque_request_converts_without_implicit_trait_upcasting() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = ExternalCompareRequest::from_backend_owner(Box::new(ProtectedOwner {
        drops: drops.clone(),
    }))
    .into_backend_owner();
    assert!(owner.as_any().is::<ProtectedOwner>());
    let concrete = owner.into_any().downcast::<ProtectedOwner>().ok().unwrap();
    assert!(Arc::ptr_eq(&concrete.drops, &drops));
    drop(concrete);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
