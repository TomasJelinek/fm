use super::*;

fn key(pressed: bool) -> api::CommandKind {
    api::CommandKind::Key(api::Key {
        key: "Shift".into(),
        code: "ShiftLeft".into(),
        pressed,
        repeat: false,
        modifiers: Default::default(),
    })
}

fn pointer(kind: api::PointerKind) -> api::CommandKind {
    api::CommandKind::Pointer(api::Pointer {
        kind,
        button: api::PointerButton::Left,
        x: 10.0,
        y: 20.0,
        modifiers: Default::default(),
        click_count: 1,
    })
}

#[test]
fn resize_keeps_keyboard_and_button_release_but_rejects_stale_coordinate_input() {
    let mut frames = crate::interaction::Frames::default();
    let first = frames.painted().unwrap();
    frames.invalidate();
    assert!(accepts_input(&frames, first, first, &key(false)));
    assert!(accepts_input(&frames, first, first, &key(true)));
    assert!(accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Up)
    ));
    assert!(accepts_input(
        &frames,
        first,
        first,
        &api::CommandKind::Zoom(1.25)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Down)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Move)
    ));
    let fresh = frames.painted().unwrap();
    assert!(accepts_input(
        &frames,
        first,
        fresh,
        &pointer(api::PointerKind::Down)
    ));
}

#[test]
fn replaced_document_rejects_old_keys_releases_and_permission_increases() {
    let mut frames = crate::interaction::Frames::default();
    let old = frames.painted().unwrap();
    frames.invalidate();
    let first = frames.painted().unwrap();
    assert!(!accepts_input(&frames, first, old, &key(false)));
    assert!(!accepts_input(
        &frames,
        first,
        old,
        &pointer(api::PointerKind::Up)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        old,
        &api::CommandKind::Policy(api::Policy {
            network: true,
            scripts: false
        })
    ));
    assert!(accepts_input(&frames, first, first, &key(true)));
    assert!(accepts_input(
        &frames,
        first,
        old,
        &api::CommandKind::Policy(api::Policy::default())
    ));
    assert!(!accepts_input(&frames, first, first + 1, &key(true)));
}

#[test]
fn policy_controls_keep_file_scope_across_fresh_browser_contexts() {
    let mut frames = crate::interaction::Frames::default();
    let file = frames.painted().unwrap();
    frames.invalidate();
    let context = frames.painted().unwrap();
    let policy = api::CommandKind::Policy(api::Policy {
        network: true,
        scripts: true,
    });
    assert!(accepts_input(&frames, file, file, &policy));
    assert!(!accepts_input(&frames, context, file, &key(true)));
    assert!(accepts_input(&frames, context, context, &key(true)));
}
