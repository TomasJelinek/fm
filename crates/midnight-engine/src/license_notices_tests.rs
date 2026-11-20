use super::*;
use midnight_core::LicenseNoticeId;
use std::{path::Path, time::Duration};
use tokio::sync::mpsc;

fn setup(root: &Path) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(4);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        TaskLifetime::testing(()),
        NoticeSources {
            installed: Some(root.join("licenses")),
            inventory: Some(root.join("THIRD-PARTY.json")),
            registries: Vec::new(),
        },
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    (controller, tasks, events)
}

fn request(id: u64, name: &str) -> LicenseNoticeRequest {
    LicenseNoticeRequest {
        id: LicenseNoticeId(id),
        name: name.into(),
        version: "1.0.0".into(),
        folder: None,
    }
}

fn inventory(root: &Path, author: &str) {
    std::fs::write(
        root.join("THIRD-PARTY.json"),
        format!(r#"[{{"name": "demo", "version": "1.0.0", "authors": ["{author}"]}}]"#),
    )
    .unwrap();
}

async fn next(events: &mut mpsc::Receiver<Event>) -> LicenseNotice {
    let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::LicenseNoticeReady(notice) = event else {
        panic!("expected a license notice")
    };
    *notice
}

#[tokio::test]
async fn the_newest_request_is_answered_from_the_package_and_the_inventory_is_read_once() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("licenses/demo-1.0.0");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("LICENSE"), "Demo license text").unwrap();
    inventory(root.path(), "Ada");
    let (controller, mut tasks, mut events) = setup(root.path());
    // Both requests are queued before the worker runs: only the newer one is read.
    controller.request(request(1, "missing"));
    controller.request(request(2, "demo"));
    let notice = next(&mut events).await;
    assert_eq!(notice.id, LicenseNoticeId(2));
    assert!(notice.text.contains("Demo license text"), "{}", notice.text);
    assert_eq!(notice.authors, ["Ada"]);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "the replaced request is never answered"
    );
    // The inventory was parsed with the first read and is not read again.
    inventory(root.path(), "Mallory");
    controller.request(request(3, "demo"));
    let again = next(&mut events).await;
    assert_eq!(
        (again.id, again.authors),
        (LicenseNoticeId(3), vec![String::from("Ada")])
    );
    // A missing library still gets an (empty) answer.
    controller.request(request(4, "missing"));
    let missing = next(&mut events).await;
    assert_eq!(missing.id, LicenseNoticeId(4));
    assert!(missing.text.is_empty() && missing.authors.is_empty());
    controller.shutdown();
    controller.request(request(5, "demo"));
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(events.try_recv().is_err(), "no answer after shutdown");
}
