//! Select, deselect and invert by filename mask (the keypad +, − and * keys).
//! Masks use the panel filter's Masks grammar (`FilterMasks`), which the dialog
//! checks; a backend worker then compiles the mask (`MaskValidationRequest`) before
//! the dialog applies and remembers it, and selection workers compile it to match.
use crate::{FilterMasks, FilterMode, MAX_FILTER_BYTES, MaskSyntaxError, PanelFilter};

/// The pattern a new Select/Deselect by mask dialog starts with.
pub const DEFAULT_SELECTION_MASK: &str = "*";

/// One mask selection gesture against the displayed rows of a panel. Matching
/// ignores case like a default panel filter; the parent row is never selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskSelection {
    /// Panel filter masks: `;` separates masks, one `|` starts exclusions.
    pub pattern: String,
    /// Select matching rows; false deselects them.
    pub select: bool,
    /// Folders and directory links take part too; files always do.
    pub folders: bool,
}

impl MaskSelection {
    /// The equivalent enabled Masks filter the backend compiles for matching.
    pub fn filter(&self) -> PanelFilter {
        PanelFilter {
            enabled: true,
            mode: FilterMode::Masks,
            expression: self.pattern.clone(),
            case_sensitive: false,
        }
    }

    /// Whether a row of this kind takes part; matching its name decides the rest.
    pub fn includes(&self, directory: bool) -> bool {
        !directory || self.folders
    }
}

/// OK in the Select or Deselect by mask dialog asks a backend worker whether
/// `pattern` compiles into a matcher within the selection workers' budget, as the
/// panel filter dialog has its filter checked. The answer
/// (`Event::SelectionMaskValidated`) carries the request back; only the latest of
/// increasing `id`s is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskValidationRequest {
    pub id: u64,
    pub pattern: String,
}

/// The last mask and folder choice, shared by Select and Deselect by mask and by
/// the keypad's invert (which inverts folders too when `folders` is set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskSelectionPreferences {
    pub pattern: String,
    pub folders: bool,
}

impl Default for MaskSelectionPreferences {
    fn default() -> Self {
        Self {
            pattern: DEFAULT_SELECTION_MASK.into(),
            folders: false,
        }
    }
}

impl MaskSelectionPreferences {
    /// Storage bounds only, so a remembered mask never makes saved settings
    /// unreadable; `validate_mask` adds the grammar.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.pattern.len() > MAX_FILTER_BYTES {
            Err("A selection mask may contain at most 4096 UTF-8 bytes.")
        } else if self.pattern.chars().any(char::is_control) {
            Err("A selection mask cannot contain control characters.")
        } else {
            Ok(())
        }
    }

    /// The dialog's own check of a mask: the storage bounds and the panel filter's
    /// Masks grammar. A mask that passes is then compiled by a backend worker
    /// (`MaskValidationRequest`), which also refuses a matcher over its size budget;
    /// only then is it applied and remembered.
    pub fn validate_mask(&self) -> Result<(), &'static str> {
        self.validate()?;
        match FilterMasks::parse(&self.pattern) {
            Ok(_) => Ok(()),
            Err(MaskSyntaxError::TooManyMasks) => {
                Err("A selection mask may contain at most 64 masks.")
            }
            Err(error) => Err(error.message()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_selection_uses_a_case_insensitive_masks_filter_and_kind_policy() {
        let selection = MaskSelection {
            pattern: "*.txt;*.md|draft*".into(),
            select: true,
            folders: false,
        };
        let filter = selection.filter();
        assert!(filter.enabled);
        assert_eq!(filter.mode, FilterMode::Masks);
        assert_eq!(filter.expression, "*.txt;*.md|draft*");
        assert!(!filter.case_sensitive);
        assert!(selection.includes(false));
        assert!(!selection.includes(true));
        assert!(
            MaskSelection {
                folders: true,
                ..selection
            }
            .includes(true)
        );
    }

    #[test]
    fn preferences_default_to_every_file_and_bound_the_stored_pattern() {
        let defaults = MaskSelectionPreferences::default();
        assert_eq!(defaults.pattern, "*");
        assert!(!defaults.folders);
        assert!(defaults.validate().is_ok());
        let long = MaskSelectionPreferences {
            pattern: "a".repeat(MAX_FILTER_BYTES + 1),
            folders: true,
        };
        assert!(long.validate().is_err());
        let control = MaskSelectionPreferences {
            pattern: "*.txt\n".into(),
            folders: false,
        };
        assert!(control.validate().is_err());
        let unicode = MaskSelectionPreferences {
            pattern: "文档*.txt;é?".into(),
            folders: true,
        };
        assert!(unicode.validate().is_ok());
    }

    #[test]
    fn the_dialog_accepts_only_masks_of_the_panel_filter_grammar() {
        let mask = |pattern: &str| MaskSelectionPreferences {
            pattern: pattern.into(),
            folders: false,
        };
        for pattern in [
            "*",
            "*.txt;*.md|draft*",
            "|*.tmp",
            "semi;;colon",
            "文档*.txt;é?",
        ] {
            assert_eq!(mask(pattern).validate_mask(), Ok(()), "{pattern:?}");
        }
        let most = vec!["*.x"; 64].join(";");
        assert_eq!(mask(&most).validate_mask(), Ok(()));
        let too_many = vec!["*.x"; 65].join(";");
        let too_long = "a".repeat(MAX_FILTER_BYTES + 1);
        for (pattern, error) in [
            ("a|b|c", "Use at most one | before excluded masks."),
            ("*.txt|*.md|", "Use at most one | before excluded masks."),
            ("*.txt|", "Add at least one excluded mask after |."),
            ("*.txt| ;", "Add at least one excluded mask after |."),
            (
                too_many.as_str(),
                "A selection mask may contain at most 64 masks.",
            ),
            (
                too_long.as_str(),
                "A selection mask may contain at most 4096 UTF-8 bytes.",
            ),
            (
                "*.txt\0",
                "A selection mask cannot contain control characters.",
            ),
        ] {
            assert_eq!(mask(pattern).validate_mask(), Err(error), "{pattern:?}");
        }
        // Loading saved settings checks only the storage bounds: a mask an earlier
        // build remembered still loads, and the dialog then says what is wrong.
        assert_eq!(mask("a|b|c").validate(), Ok(()));
    }
}
