//! Browser-owned live-focus arbitration. New wire tags require an explicit launch opt-in.
use midnight_web_viewer_api as api;

// CEF EVENTFLAG_IS_REPEAT; the platform adapters preserve Key.repeat in this flag.
pub(crate) const REPEAT: u32 = 8192;
const KEY_MODIFIERS: u32 = 2 | 4 | 8 | 128;

pub(crate) fn enabled(value: Option<&str>) -> bool {
    value == Some(api::HOST_SHORTCUTS_PHYSICAL_V1)
}

pub(crate) fn shortcut(
    key: i32,
    flags: u32,
    editable: bool,
    extended: bool,
) -> Option<api::HostShortcut> {
    if editable {
        return None;
    }
    let modifiers = flags & KEY_MODIFIERS;
    match (key, modifiers) {
        (32, 0) => Some(api::HostShortcut::CycleNext),
        (32, 2) => Some(api::HostShortcut::CyclePrevious),
        (8, 0) if extended => Some(api::HostShortcut::Backspace),
        (37, 8) if extended => Some(api::HostShortcut::AltLeft),
        (39, 8) if extended => Some(api::HostShortcut::AltRight),
        _ => None,
    }
}

pub(crate) fn emit(shortcut: api::HostShortcut, flags: u32, key_down: bool) -> bool {
    key_down
        && (matches!(
            shortcut,
            api::HostShortcut::CycleNext | api::HostShortcut::CyclePrevious
        ) || flags & REPEAT == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_hosts_never_receive_extended_tags_and_opt_in_is_exact() {
        for value in [
            None,
            Some(""),
            Some("1"),
            Some("physical-v2"),
            Some("physical-v1 "),
        ] {
            assert!(!enabled(value));
            for (key, flags) in [(8, 0), (37, 8), (39, 8)] {
                assert_eq!(shortcut(key, flags, false, enabled(value)), None);
            }
        }
        assert!(enabled(Some(api::HOST_SHORTCUTS_PHYSICAL_V1)));
    }
    #[test]
    fn editable_fields_keep_every_navigation_key_and_modifier_variant() {
        for (key, flags) in [(8, 0), (32, 0), (32, 2), (37, 8), (39, 8)] {
            assert_eq!(shortcut(key, flags, true, true), None);
        }
        for (key, flags) in [(8, 2), (8, 4), (37, 0), (37, 10), (39, 12), (39, 136)] {
            assert_eq!(shortcut(key, flags, false, true), None);
        }
    }
    #[test]
    fn exact_physical_keys_have_distinct_events_and_do_not_repeat_file_changes() {
        for (key, flags, expected) in [
            (8, 0, api::HostShortcut::Backspace),
            (37, 8, api::HostShortcut::AltLeft),
            (39, 8, api::HostShortcut::AltRight),
        ] {
            assert_eq!(shortcut(key, flags, false, true), Some(expected));
            assert!(emit(expected, flags, true));
            assert!(!emit(expected, flags | REPEAT, true));
            assert!(!emit(expected, flags, false));
        }
    }
    #[test]
    fn baseline_space_and_shift_space_keep_their_v1_meaning() {
        assert_eq!(
            shortcut(32, 0, false, false),
            Some(api::HostShortcut::CycleNext)
        );
        assert_eq!(
            shortcut(32, 2, false, false),
            Some(api::HostShortcut::CyclePrevious)
        );
        assert!(emit(api::HostShortcut::CycleNext, REPEAT, true));
    }
}
