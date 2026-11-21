use super::*;
use midnight_core::{FilterDiagnosticKind, FilterMode, PaneId, PanelFilter};
use std::time::Duration;
use tokio::sync::mpsc;

fn request(id: u64, source: &str) -> FilterValidationRequest {
    FilterValidationRequest {
        id,
        pane: PaneId::Left,
        filter: PanelFilter {
            enabled: true,
            mode: FilterMode::Expression,
            expression: source.into(),
            case_sensitive: false,
        },
    }
}

#[tokio::test]
async fn expression_validation_reports_typed_utf8_span_without_touching_a_filesystem() {
    let (sender, mut events) = mpsc::channel(2);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller.validate(request(1, "name = \"é\" trailing"));
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::PanelFilterValidated {
        request,
        result,
        diagnostic,
    } = event
    else {
        panic!("Expected filter validation")
    };
    assert_eq!(result.unwrap_err().kind, io::ErrorKind::InvalidInput);
    let diagnostic = diagnostic.unwrap();
    assert_eq!(diagnostic.kind, FilterDiagnosticKind::Syntax);
    assert_eq!(&request.filter.expression[diagnostic.span], "trailing");
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn latest_filter_intent_supersedes_old_validation_behind_a_full_outbox() {
    let (sender, mut events) = mpsc::channel(1);
    sender.send(Event::VolumesUpdated(vec![])).await.unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    controller.validate(request(1, "kind > file"));
    tokio::task::yield_now().await;
    controller.validate(request(2, "kind = file"));
    assert!(matches!(
        events.recv().await,
        Some(Event::VolumesUpdated(_))
    ));
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::PanelFilterValidated {
        request,
        result,
        diagnostic,
    } = event
    else {
        panic!("Expected latest filter validation")
    };
    assert_eq!(request.id, 2);
    assert!(result.is_ok());
    assert!(diagnostic.is_none());
    controller.validate(super::tests::request(1, "size = \"bad\""));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(
        events.try_recv().is_err(),
        "Obsolete IDs must not revive an old draft"
    );
}

/// Ask `controller` to check a Select by mask pattern and wait for its answer.
async fn check_mask(
    controller: &Controller,
    events: &mut mpsc::Receiver<Event>,
    id: u64,
    pattern: String,
) -> Result<(), String> {
    controller.validate_mask(MaskValidationRequest { id, pattern });
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::SelectionMaskValidated { request, result } = event else {
        panic!("Expected a mask check")
    };
    assert_eq!(request.id, id);
    result
}

#[tokio::test]
async fn select_masks_are_compiled_on_a_worker_with_their_own_identities() {
    let (sender, mut events) = mpsc::channel(4);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let valid = check_mask(&controller, &mut events, 1, "*.txt;*.MD|draft*".into()).await;
    assert_eq!(valid, Ok(()));
    // Thousands of ? pass the dialog's grammar check but exceed the matcher budget.
    let large = "?".repeat(midnight_core::MAX_FILTER_BYTES);
    assert_eq!(
        check_mask(&controller, &mut events, 2, large).await,
        Err(midnight_platform_common::filter::MASK_TOO_LARGE.to_owned())
    );
    assert_eq!(
        check_mask(&controller, &mut events, 3, "a|b|c".into()).await,
        Err("Use at most one | before excluded masks.".to_owned())
    );
    // Mask checks never supersede a pane's filter draft, whose identities are its own.
    controller.validate(request(1, "kind = file"));
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        Event::PanelFilterValidated { request, result: Ok(()), .. } if request.id == 1
    ));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}
