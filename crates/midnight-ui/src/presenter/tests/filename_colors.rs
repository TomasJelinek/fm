use super::*;

fn colored_preferences() -> UserPreferences {
    UserPreferences {
        filename_colors: vec![FilenameColorRule {
            style: FilenameStyle {
                foreground: Some(RgbColor(220, 30, 40)),
                background: None,
            },
            ..Default::default()
        }]
        .into(),
        ..Default::default()
    }
}

fn request(p: &mut Presenter) -> FilenameColorsRequest {
    let sent = RefCell::new(None);
    p.flush_filename_colors(&|command| {
        if let Command::MatchFilenameColors(request) = command
            && request.pane == PaneId::Left
        {
            *sent.borrow_mut() = Some(request);
        }
        true
    });
    sent.into_inner().unwrap()
}

fn deliver(p: &mut Presenter, request: FilenameColorsRequest) {
    let result = FilenameColorsResult {
        styles: request
            .source
            .iter()
            .map(|entry| (entry.id, request.rules[0].style))
            .collect(),
        request: request.clone(),
    };
    p.apply(Event::FilenameColorsReady {
        request,
        result: Ok(Arc::new(result)),
    });
}

#[test]
fn stale_color_work_cannot_color_recycled_entry_ids_after_refresh() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "old.rs")]);
    let old = request(&mut p);
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(1, "replacement.txt")],
    );
    let new = request(&mut p);
    deliver(&mut p, old);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    deliver(&mut p, new);
    assert_eq!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .unwrap()
            .foreground,
        Some(RgbColor(220, 30, 40))
    );
}

#[test]
fn selection_changes_reuse_styles_but_reordered_source_gets_fresh_matching() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(
        &mut p,
        PaneId::Left,
        generation,
        vec![entry(1, "a.rs"), entry(2, "b.rs")],
    );
    let initial = request(&mut p);
    deliver(&mut p, initial);
    p.panes[0].cursor = Some(EntryId(2));
    p.panes[0].selected =
        Arc::new(SelectionSnapshot::from_bits(p.panes[0].index.clone(), vec![2]).unwrap());
    p.flush_filename_colors(&|_| panic!("caret-only change must reuse styles"));
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[1])
            .is_some()
    );
    let reversed: Vec<_> = p.panes[0].entries.iter().rev().cloned().collect();
    p.panes[0].entries = reversed.into();
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    let reordered = request(&mut p);
    assert_eq!(reordered.source[0].id, EntryId(2));
    deliver(&mut p, reordered);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_some()
    );
}

#[test]
fn local_color_edits_survive_late_preferences_without_overwriting_other_saved_options() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.update_preferences(colored_preferences(), &|_| {
        panic!("save must await disk preferences")
    });
    p.preferences_loaded(Ok(UserPreferences {
        theme: PreferenceTheme::Nord,
        font_family: "Saved 文档".into(),
        ..Default::default()
    }));
    assert_eq!(
        p.preferences.values.filename_colors,
        colored_preferences().filename_colors
    );
    assert_eq!(p.preferences.values.theme, PreferenceTheme::Nord);
    assert_eq!(p.preferences.values.font_family, "Saved 文档");
}

#[test]
fn removing_last_rule_cancels_backend_work_and_releases_the_previous_snapshot() {
    let mut p = Presenter::new("left".into(), "right".into());
    p.preferences_loaded(Ok(colored_preferences()));
    let generation = begin(&mut p, PaneId::Left, "left");
    finish(&mut p, PaneId::Left, generation, vec![entry(1, "a.rs")]);
    let current = request(&mut p);
    deliver(&mut p, current);
    Arc::make_mut(&mut p.preferences.values.filename_colors).clear();
    let cleared = RefCell::new(Vec::new());
    p.flush_filename_colors(&|command| {
        let Command::ClearFilenameColors { pane, .. } = command else {
            panic!("expected color cancellation")
        };
        cleared.borrow_mut().push(pane);
        true
    });
    assert_eq!(*cleared.borrow(), [PaneId::Left, PaneId::Right]);
    assert!(
        p.filename_style(PaneId::Left, &p.panes[0].entries[0])
            .is_none()
    );
    p.flush_filename_colors(&|_| panic!("cancel must not repeat after it is accepted"));
}
