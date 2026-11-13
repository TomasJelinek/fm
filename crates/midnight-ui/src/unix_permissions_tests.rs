use super::*;

#[test]
fn matrix_edits_reject_unknown_bits_negative_values_and_values_outside_the_mask() {
    assert_eq!(matrix_edit(0o020, 0o020).unwrap(), (0o020, 0o020));
    assert_eq!(matrix_edit(0, 0).unwrap(), (0, 0));
    for (mask, value) in [(-1, 0), (0o7777, -1), (0o10000, 0), (0o020, 0o040)] {
        assert!(matrix_edit(mask, value).is_err());
    }
}

#[test]
fn octal_input_preserves_special_bits_unless_explicitly_supplied() {
    assert_eq!(parse_octal("600").unwrap(), (0o777, 0o600));
    assert_eq!(parse_octal(" 4755 ").unwrap(), (0o7777, 0o4755));
    for text in ["", "75", "888", "10000", "-755", "7.55e2"] {
        assert!(parse_octal(text).is_err(), "{text}");
    }
}

#[test]
fn aggregation_preserves_only_known_common_permission_bits() {
    assert_eq!(
        common_modes([Some(0o100640), Some(0o100600)], 2),
        (0o7737, 0o600)
    );
    assert_eq!(common_modes([Some(0o600), None], 2), (0, 0));
    assert_eq!(common_modes([Some(0o600)], 2), (0, 0));
    assert_eq!(common_modes([], 0), (0, 0));
}

#[test]
fn a_single_permission_edit_preserves_other_mixed_and_special_bits() {
    let view = project(0o600, 0o7737, 0o020, 0o020);
    assert_eq!((view.known, view.mode), (0o7737, 0o620));
    assert_eq!(view.octal, "");
    assert_eq!(view.text, "rw-?w----");
    let view = project(0o2600, 0o7777, 0o040, 0o040);
    assert_eq!(view.mode, 0o2640);
    assert_eq!(view.octal, "2640");
    assert_eq!(view.text, "rw-r-S---");
}

#[test]
fn permission_text_distinguishes_special_bits_and_unknown_values() {
    for (mode, octal, text) in [
        (0o600, "600", "rw-------"),
        (0o4755, "4755", "rwsr-xr-x"),
        (0o4644, "4644", "rwSr--r--"),
        (0o2640, "2640", "rw-r-S---"),
        (0o1777, "1777", "rwxrwxrwt"),
        (0o1700, "1700", "rwx-----T"),
    ] {
        let view = project(mode, 0o7777, 0, 0);
        assert_eq!((view.octal.as_str(), view.text.as_str()), (octal, text));
    }
    assert_eq!(project(0, 0, 0, 0).text, "?????????");
    assert_eq!(project(0o4000, 0o7776, 0, 0).text, "--S-----?");
    assert_eq!(project(0o4000, 0o7677, 0, 0).text, "--?------");
}
