//! Compile Select/Deselect by mask patterns on selection workers, never in the UI.
use midnight_core::MaskSelection;
use midnight_platform_common::filter::CompiledFilter;

/// The panel filter's Masks grammar, ignoring case; folders are decided by the caller.
pub(crate) fn compile(mask: &MaskSelection) -> Result<CompiledFilter, String> {
    CompiledFilter::new(&mask.filter())
}

/// Native selection failures carry fixed texts. The mask grammar reports these.
pub(crate) fn fixed_error(message: &str) -> &'static str {
    const KNOWN: [&str; 5] = [
        "Use at most one | before excluded masks.",
        "Add at least one excluded mask after |.",
        "A panel filter may contain at most 64 masks.",
        "A panel filter may contain at most 4096 UTF-8 bytes.",
        "A panel filter cannot contain NUL.",
    ];
    KNOWN
        .into_iter()
        .find(|known| *known == message)
        .unwrap_or("The selection mask is not valid. Separate masks with ; and put at most one | before excluded masks.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn mask(pattern: &str) -> MaskSelection {
        MaskSelection {
            pattern: pattern.into(),
            select: true,
            folders: false,
        }
    }

    #[test]
    fn masks_match_like_a_case_insensitive_panel_filter() {
        let matcher = compile(&mask("*.txt;*.MD|draft*")).unwrap();
        for (name, matched) in [
            ("a.txt", true),
            ("A.TXT", true),
            ("notes.md", true),
            ("draft.txt", false),
            ("Draft-2.md", false),
            ("a.txt.bak", false),
            ("文档.TXT", true),
        ] {
            assert_eq!(matcher.matches(OsStr::new(name)), matched, "{name}");
        }
        let every = compile(&mask("*")).unwrap();
        assert!(every.matches(OsStr::new(".hidden")));
        assert!(every.matches(OsStr::new("no-extension")));
        let single = compile(&mask("?.log")).unwrap();
        assert!(single.matches(OsStr::new("b.log")));
        assert!(!single.matches(OsStr::new("bb.log")));
    }

    #[test]
    fn invalid_masks_report_the_grammar_error_as_a_fixed_text() {
        for (pattern, expected) in [
            ("*.a|*.b|*.c", "Use at most one | before excluded masks."),
            ("*.txt|", "Add at least one excluded mask after |."),
        ] {
            let error = compile(&mask(pattern)).err().unwrap();
            assert_eq!(fixed_error(&error), expected);
        }
        let many = vec!["*.x"; 65].join(";");
        let error = compile(&mask(&many)).err().unwrap();
        assert_eq!(
            fixed_error(&error),
            "A panel filter may contain at most 64 masks."
        );
        assert!(fixed_error("unexpected engine text").starts_with("The selection mask"));
    }
}
