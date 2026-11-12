use super::*;
use midnight_core::{
    Entry, EntryId, EntryKind, FilenameColorRule, FilenameStyle, PaneId, RgbColor,
};

fn request() -> FilenameColorsRequest {
    FilenameColorsRequest {
        id: 1,
        pane: PaneId::Left,
        source: vec![
            Entry::new(EntryId(1), "doc.rs".into(), EntryKind::File),
            Entry::new(EntryId(2), "doc.txt".into(), EntryKind::File),
        ]
        .into(),
        rules: vec![FilenameColorRule {
            expression: "*.rs".into(),
            style: FilenameStyle {
                foreground: Some(RgbColor(10, 20, 30)),
                background: None,
            },
            ..Default::default()
        }]
        .into(),
    }
}

#[test]
fn computation_preserves_exact_source_and_matches_only_requested_names() {
    let request = request();
    let event = compute(&Work::Match(request.clone()), &|| false).unwrap();
    let Event::FilenameColorsReady {
        result: Ok(result), ..
    } = event
    else {
        panic!("expected matched result")
    };
    assert!(Arc::ptr_eq(&result.request.source, &request.source));
    assert_eq!(result.styles.len(), 1);
    assert_eq!(
        result.styles[&EntryId(1)].foreground,
        Some(RgbColor(10, 20, 30))
    );
}

#[test]
fn cancellation_and_ambiguous_ids_cannot_publish_styles() {
    let mut request = request();
    assert!(compute(&Work::Match(request.clone()), &|| true).is_none());
    request.source = vec![request.source[0].clone(), request.source[0].clone()].into();
    assert!(matches!(
        compute(&Work::Match(request), &|| false),
        Some(Event::FilenameColorsReady { result: Err(_), .. })
    ));
}

#[tokio::test]
async fn coalesced_validation_publishes_only_latest_revision() {
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let controller = Controller::new(outbox, &mut tasks);
    controller.validate(FilenameColorsValidation {
        id: 1,
        rules: vec![],
    });
    controller.validate(FilenameColorsValidation {
        id: 2,
        rules: vec![FilenameColorRule {
            expression: "[".into(),
            mode: midnight_core::FilterMode::Regex,
            ..Default::default()
        }],
    });
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        Event::FilenameColorsValidated {
            request: FilenameColorsValidation { id: 2, .. },
            result: Err(_)
        }
    ));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn disabling_colors_releases_captured_source_and_cancels_pending_publication() {
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let controller = Controller::new(outbox, &mut tasks);
    let request = request();
    let source = Arc::downgrade(&request.source);
    controller.matches(request);
    controller.clear(PaneId::Left, 2);
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(events.recv().await.is_none());
    assert!(source.upgrade().is_none());
}
