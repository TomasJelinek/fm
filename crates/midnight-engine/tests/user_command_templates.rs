use midnight_core::{UserCommandTemplate as Template, UserCommandVariable as Variable};
use std::ffi::{OsStr, OsString};

#[test]
fn native_values_are_literal_and_never_recursively_expanded_or_shell_quoted() {
    let parsed = Template::parse("prefix:${file.name}:${file.extension}:suffix").unwrap();
    let name = OsStr::new("č文 $(touch nope); '&\" ${other.path}");
    let empty = OsStr::new("");
    let result = parsed
        .expand(|variable| match variable {
            Variable::FileName => Some(name),
            Variable::FileExtension => Some(empty),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        result,
        OsString::from("prefix:č文 $(touch nope); '&\" ${other.path}::suffix")
    );
    assert!(
        Template::parse("${file.path}")
            .unwrap()
            .expand(|_| None)
            .is_err()
    );
}

#[test]
fn group_tokens_are_whole_arguments_and_unknown_or_malformed_templates_are_refused() {
    assert!(Template::parse("${selected.paths}").unwrap().is_group());
    for text in [
        "x${selected.paths}",
        "${selected.paths}x",
        "${unknown}",
        "${",
        "${file.path",
        "${}",
        "a\0b",
    ] {
        assert!(Template::parse(text).is_err(), "{text:?}");
    }
    assert!(
        Template::parse("${selected.paths}")
            .unwrap()
            .expand(|_| Some(OsStr::new("data")))
            .is_err()
    );
}

#[test]
fn escaped_dollars_and_every_scalar_variable_have_explicit_semantics() {
    let parsed = Template::parse("$${file.path}/$$/${active.path}/${other.path}/${file.path}/${file.name}/${file.stem}/${file.extension}").unwrap();
    let value = OsStr::new("literal");
    assert_eq!(
        parsed.expand(|_| Some(value)).unwrap(),
        OsStr::new("${file.path}/$/literal/literal/literal/literal/literal/literal")
    );
    assert!(!parsed.is_group());
    assert_eq!(
        Template::parse("").unwrap().expand(|_| None).unwrap(),
        OsStr::new("")
    );
}

#[cfg(unix)]
#[path = "support/user_command_template_unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "support/user_command_template_windows.rs"]
mod windows;

#[test]
fn native_long_values_are_preserved_and_workload_budgets_fail_without_truncation() {
    let long = OsString::from("文".repeat(20_000));
    let expanded = Template::parse("${file.path}")
        .unwrap()
        .expand(|_| Some(long.as_os_str()))
        .unwrap();
    assert_eq!(expanded, long);
    assert!(
        Template::parse("${file.path}")
            .unwrap()
            .expand(|_| Some(OsStr::new("a\0b")))
            .is_err()
    );
    assert!(Template::parse(&"x".repeat(1024 * 1024 + 1)).is_err());
    assert!(Template::parse(&"${file.name}".repeat(4097)).is_err());
    let oversized = OsString::from("x".repeat(8 * 1024 * 1024 + 1));
    assert!(
        Template::parse("${file.path}")
            .unwrap()
            .expand(|_| Some(oversized.as_os_str()))
            .is_err()
    );
}
