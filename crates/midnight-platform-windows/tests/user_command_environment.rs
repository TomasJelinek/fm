#![cfg(windows)]
#[allow(
    dead_code,
    reason = "Standalone block encoder harness excludes the native process caller."
)]
#[path = "../src/user_command_environment.rs"]
mod environment;
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
