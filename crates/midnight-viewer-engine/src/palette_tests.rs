use super::*;

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "document family".into(),
        font_size: 23.0,
        foreground: [11, 22, 33, 255],
        background: [230, 231, 232, 255],
        selection: [40, 50, 60, 255],
        accent: [70, 80, 90, 255],
    }
}

#[test]
fn acceptance_compares_latest_requested_palette_so_skipped_b_then_a_advances() {
    let a = appearance();
    let palette = State::new(&a);
    let mut b = a.clone();
    b.accent = [90, 80, 70, 255];
    assert_eq!(palette.restore(&b, || Ok(())).unwrap(), 2);
    assert_eq!(palette.restore(&a, || Ok(())).unwrap(), 3);
    assert_eq!(palette.snapshot(), (3, Colors::from(&a)));
    assert_eq!(palette.restore(&a, || Ok(())).unwrap(), 3);
}

#[test]
fn typography_is_not_palette_authority_and_full_wakes_keep_latest_acceptance() {
    let a = appearance();
    let palette = State::new(&a);
    let mut value = a.clone();
    value.family = "UI family".into();
    value.font_size = 77.0;
    assert_eq!(palette.restore(&value, || Ok(())).unwrap(), 1);
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    assert!(sender.try_send(crate::worker::Message::Ready).is_ok());
    value.background = [1, 2, 3, 255];
    assert_eq!(
        palette
            .restore(&value, || sender.try_send(crate::worker::Message::Ready))
            .unwrap(),
        2
    );
    let mut document = a;
    palette.snapshot().1.apply(&mut document);
    assert_eq!(document.family, "document family");
    assert_eq!(document.font_size, 23.0);
    assert_eq!(document.background, value.background);
}

#[test]
fn exhaustion_closed_and_disconnected_acceptance_leave_prior_slot_intact() {
    let a = appearance();
    let palette = State::new(&a);
    palette.value.lock().unwrap().revision = u64::MAX;
    let original = palette.snapshot();
    let mut b = a.clone();
    b.foreground = [9; 4];
    assert!(
        palette
            .restore(&b, || panic!("exhaustion must not submit work"))
            .is_err()
    );
    assert_eq!(palette.snapshot(), original);
    let palette = State::new(&a);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    drop(receiver);
    assert!(
        palette
            .restore(&b, || sender.try_send(crate::worker::Message::Ready))
            .is_err()
    );
    assert_eq!(palette.snapshot(), (1, Colors::from(&a)));
    palette.close();
    assert!(
        palette
            .restore(&a, || panic!("closed acceptance must not wake"))
            .is_err()
    );
    assert_eq!(palette.snapshot(), (1, Colors::from(&a)));
}

#[test]
fn unstarted_worker_lifetime_drop_rejects_later_palette_acceptance() {
    let a = appearance();
    let palette = Arc::new(State::new(&a));
    drop(Lifetime::new(palette.clone()));
    assert!(
        palette
            .restore(&a, || panic!("stopped worker must not wake"))
            .is_err()
    );
}
