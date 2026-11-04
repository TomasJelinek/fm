use fm_core::{ShellFlavor, quote_shell_path};
use std::path::Path;

#[test]
fn simple_unicode_paths_are_inserted_without_surrounding_quotes() {
    for shell in [
        ShellFlavor::Cmd,
        ShellFlavor::PowerShell,
        ShellFlavor::Posix,
    ] {
        for input in [
            "document.txt",
            "/文档/Žluťoučký/👩🏽‍💻/e\u{301}",
            "relative/path",
        ] {
            assert_eq!(quote_shell_path(Path::new(input), shell).unwrap(), input);
        }
    }
    for shell in [ShellFlavor::Cmd, ShellFlavor::PowerShell] {
        let input = "C:\\文档\\Žluťoučký\\";
        assert_eq!(quote_shell_path(Path::new(input), shell).unwrap(), input);
    }
}

#[test]
fn powershell_special_characters_use_literal_double_quoted_tokens() {
    assert_eq!(
        quote_shell_path(
            Path::new("文档's ‘ ’ \" “ ” „ `$x;$(throw 1)&"),
            ShellFlavor::PowerShell
        )
        .unwrap(),
        "\"文档's ‘ ’ `\" `“ `” `„ ```$x;`$(throw 1)&\""
    );
}

#[test]
fn posix_special_characters_use_literal_double_quoted_tokens() {
    assert_eq!(
        quote_shell_path(Path::new("文档's \" \\ `$x;$(exit 7)&"), ShellFlavor::Posix).unwrap(),
        "\"文档's \\\" \\\\ \\`\\$x;\\$(exit 7)&\""
    );
}

#[test]
fn empty_and_shell_syntax_are_never_bare_tokens() {
    for shell in [
        ShellFlavor::Cmd,
        ShellFlavor::PowerShell,
        ShellFlavor::Posix,
    ] {
        for input in [
            "",
            "two words",
            "a&b",
            "a|b",
            "(a)",
            "a^b",
            "a'b",
            "~",
            "#x",
            "a=b",
        ] {
            let token = quote_shell_path(Path::new(input), shell).unwrap();
            assert!(
                token.starts_with('"') && token.ends_with('"'),
                "{shell:?}: {input:?}"
            );
        }
        assert!(quote_shell_path(Path::new("nul\0path"), shell).is_err());
    }
}

#[test]
fn powershell_numeric_names_and_unicode_syntax_are_quoted_without_changing_spelling() {
    for input in [
        "001",
        "0x10",
        "1e3",
        "1kb",
        ".5",
        "-name",
        "–name",
        "—name",
        "―name",
        "one‘two",
        "one’two",
        "one‚two",
        "one‛two",
        "one“two",
        "one”two",
        "one„two",
    ] {
        let token = quote_shell_path(Path::new(input), ShellFlavor::PowerShell).unwrap();
        assert!(token.starts_with('"') && token.ends_with('"'), "{input:?}");
    }
}

#[test]
fn cmd_rejects_expansion_and_line_breaks_even_when_quoting_would_look_safe() {
    for input in ["%PATH%", "a\"b", "a\nb", "a\rb", "C:\\Folder Space\\"] {
        assert!(quote_shell_path(Path::new(input), ShellFlavor::Cmd).is_err());
    }
}
