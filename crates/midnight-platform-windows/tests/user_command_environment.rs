#![cfg(windows)]
#[allow(
    dead_code,
    reason = "Standalone block encoder harness excludes the native process caller."
)]
#[path = "../src/user_command_environment.rs"]
mod environment;
#[path = "../src/windows_argv.rs"]
mod windows_argv;
use std::{ffi::OsString, os::windows::ffi::OsStringExt};

#[test]
fn child_block_replaces_case_insensitive_keys_removes_undeclared_context_and_preserves_native_data()
{
    let raw = OsString::from_wide(&[0xd800, b'&' as u16, b'=' as u16, 0xdcff]);
    let inherited = vec![
        ("z".into(), "last".into()),
        ("=C:".into(), "C:\\cwd".into()),
        ("me_file_path".into(), "stale".into()),
        ("ME_OTHER_PATH".into(), "undeclared".into()),
        ("ME_FILE_PATH_ARG".into(), "stale encoded argument".into()),
        (
            "me_other_path_arg".into(),
            "undeclared encoded argument".into(),
        ),
        ("A".into(), "value".into()),
    ];
    let block = environment::build(inherited, &[("ME_FILE_PATH".into(), raw.clone())]).unwrap();
    assert!(block.ends_with(&[0, 0]));
    let records = block[..block.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|record| !record.is_empty())
        .map(OsString::from_wide)
        .collect::<Vec<_>>();
    let mut expected = vec![OsString::from("=C:=C:\\cwd"), "A=value".into()];
    let mut data = OsString::from("ME_FILE_PATH=");
    data.push(raw);
    expected.push(data);
    expected.push("z=last".into());
    assert_eq!(records, expected);
}

#[test]
fn unknown_duplicate_or_nul_custom_environment_fields_are_rejected_before_spawn() {
    for entries in [
        vec![("PATH".into(), OsString::from("untrusted"))],
        vec![("ME_FILE_PATH".into(), "x\0y".into())],
        vec![
            ("ME_FILE_PATH".into(), "x".into()),
            ("ME_FILE_PATH".into(), "y".into()),
        ],
    ] {
        assert!(environment::build(vec![], &entries).is_err());
    }
}

#[test]
fn native_shell_record_is_omitted_only_during_capture_and_cannot_be_overridden() {
    let shell = (OsString::from("=::"), OsString::from("opaque fixture"));
    let block = environment::snapshot_with_inherited(
        vec![
            shell.clone(),
            ("=C:".into(), r"C:\captured".into()),
            ("NORMAL".into(), "kept".into()),
        ],
        &[("ME_FILE_PATH".into(), "literal & $()".into())],
    )
    .unwrap();
    let records = block[..block.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|record| !record.is_empty())
        .map(OsString::from_wide)
        .collect::<Vec<_>>();
    assert!(
        !records
            .iter()
            .any(|record| record.to_string_lossy().starts_with("=::="))
    );
    assert!(records.contains(&OsString::from(r"=C:=C:\captured")));
    assert!(records.contains(&OsString::from("NORMAL=kept")));
    assert!(records.contains(&OsString::from("ME_FILE_PATH=literal & $()")));
    assert!(
        environment::build(vec![shell.clone()], &[]).is_err(),
        "encoder stays strict"
    );
    assert!(
        environment::snapshot_with_inherited(vec![shell], &[("=::".into(), "override".into())])
            .is_err(),
        "private inherited names cannot enter through fixed caller overrides"
    );
}

#[test]
fn shell_capture_keeps_nul_name_and_ordinal_duplicate_rejection() {
    for (name, value) in [
        ("", "value"),
        ("ordinary=name", "value"),
        ("bad\0name", "value"),
        ("=C:extra", "value"),
        ("=1:", "value"),
        ("=ExitCode", "value"),
        ("=::", "value\0tail"),
        ("NORMAL", "value\0tail"),
        ("=C:", "value\0tail"),
    ] {
        assert!(
            environment::snapshot_with_inherited(vec![(name.into(), value.into())], &[]).is_err()
        );
    }
    assert!(
        environment::snapshot_with_inherited(
            vec![("Path".into(), "one".into()), ("PATH".into(), "two".into())],
            &[],
        )
        .is_err()
    );
}
#[test]
fn identical_case_aliases_are_coalesced_without_changing_native_values() {
    let value = OsString::from_wide(&[0xd800, b'&' as u16, 0xdcff]);
    let block = environment::build(
        vec![
            ("Path".into(), value.clone()),
            ("PATH".into(), value.clone()),
            ("path".into(), value.clone()),
            ("Öppna".into(), "Unicode case alias".into()),
            ("öPPNA".into(), "Unicode case alias".into()),
        ],
        &[],
    )
    .unwrap();
    let records = block[..block.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|record| !record.is_empty())
        .map(OsString::from_wide)
        .collect::<Vec<_>>();
    let mut native_record = OsString::from("Path=");
    native_record.push(value);
    assert_eq!(records, [native_record, "Öppna=Unicode case alias".into()]);
    for values in [("same", "SAME"), ("first", "second")] {
        assert!(
            environment::build(
                vec![
                    ("Path".into(), values.0.into()),
                    ("PATH".into(), values.1.into())
                ],
                &[],
            )
            .is_err(),
            "conflicting values must never be selected by order"
        );
    }
}

#[test]
fn inherited_lowercase_drive_cwd_is_not_synthesized_again() {
    let block =
        environment::snapshot_with_inherited(vec![("=c:".into(), r"C:\captured 文档".into())], &[])
            .unwrap();
    let drive_records = block[..block.len() - 1]
        .split(|unit| *unit == 0)
        .filter(|record| {
            record.len() >= 4
                && record[0] == b'=' as u16
                && (record[1] == b'C' as u16 || record[1] == b'c' as u16)
                && record[2] == b':' as u16
                && record[3] == b'=' as u16
        })
        .map(OsString::from_wide)
        .collect::<Vec<_>>();
    assert_eq!(drive_records, [OsString::from(r"=c:=C:\captured 文档")]);
}
#[test]
fn conflicting_alias_error_names_the_keys_without_disclosing_values() {
    let error = environment::build(
        vec![
            ("Path".into(), "private first value".into()),
            ("PATH".into(), "private second value".into()),
        ],
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("\"Path\"") && error.contains("\"PATH\""));
    assert!(error.contains("conflicting values"));
    assert!(!error.contains("private"));
    let long = "N".repeat(4096);
    let error = environment::build(
        vec![
            (long.clone().into(), "first".into()),
            (long.to_ascii_lowercase().into(), "second".into()),
        ],
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(error.len() < 400, "diagnostics must remain bounded");
    let error = environment::build(
        vec![
            ("name\n".into(), "first".into()),
            ("NAME\n".into(), "second".into()),
        ],
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(
        !error.contains('\n'),
        "names must not inject diagnostic lines"
    );
}
