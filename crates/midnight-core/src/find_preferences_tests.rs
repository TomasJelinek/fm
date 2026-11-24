use super::*;

#[test]
fn accepted_masks_form_an_exact_bounded_mru() {
    let mut preferences = FindPreferences::default();
    for index in 0..35 {
        preferences
            .remember_mask(&format!("{index}-文档*.txt"))
            .unwrap();
    }
    assert_eq!(preferences.mask_history.len(), 30);
    assert_eq!(preferences.mask_history[0], "34-文档*.txt");
    assert_eq!(preferences.mask_history[29], "5-文档*.txt");
    preferences.remember_mask("10-文档*.txt").unwrap();
    assert_eq!(preferences.mask_history[0], "10-文档*.txt");
    assert_eq!(preferences.mask_history.len(), 30);
    preferences.remember_mask("*.TXT").unwrap();
    preferences.remember_mask("*.txt").unwrap();
    assert_eq!(&preferences.mask_history[..2], ["*.txt", "*.TXT"]);
    preferences.remember_mask("").unwrap();
    assert_eq!(preferences.mask_history[0], "*.*");
}

#[test]
fn malformed_masks_and_placements_are_refused_without_changing_history() {
    let mut preferences = FindPreferences::default();
    preferences.remember_mask("*.rs").unwrap();
    let accepted = preferences.clone();
    for invalid in ["x".repeat(4097), "bad\0mask".into()] {
        assert!(preferences.remember_mask(&invalid).is_err());
        assert_eq!(preferences, accepted);
    }
    for history in [
        vec![String::new()],
        vec!["*.rs".into(), "*.rs".into()],
        (0..31).map(|index| index.to_string()).collect(),
    ] {
        assert!(
            FindPreferences {
                mask_history: history,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    preferences.window_geometry = Some(WindowGeometry {
        normal: crate::WindowRect {
            x: 0,
            y: 0,
            width: 0,
            height: 500,
        },
        monitor: None,
        maximized: false,
        fullscreen: false,
    });
    assert!(preferences.validate().is_err());
}
