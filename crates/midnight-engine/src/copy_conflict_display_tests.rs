//! A conflict prompt formats the modification times of its two displayed entries
//! through the job's reviewed locale. The native transfer adapters report raw
//! times, so enumerated, inspected, published and automatically resolved entries
//! are never formatted (review P1-T1-2).
use super::*;
use crate::transfer_composition::{
    QualifiedLocaleInstallation, QualifiedTransferInstallation, tests::CountingLocale,
};

/// The production composition: an audited native transfer service carrying the
/// audited locale installation.
fn localized(locale: QualifiedLocaleInstallation) -> Harness {
    Harness::configured(
        Some(
            QualifiedTransferInstallation::audited(Arc::new(
                midnight_platform::LocalFileSystem::new(None),
            ))
            .with_locale(Some(locale)),
        ),
        1,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_conflict_prompt_formats_only_its_two_displayed_entries() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("tree/nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    let mut roots = vec![source.join("tree")];
    for index in 0..16 {
        fs::write(source.join(format!("tree/a{index}")), b"a").unwrap();
        fs::write(source.join(format!("tree/nested/b{index}")), b"b").unwrap();
        let root = source.join(format!("c{index}"));
        fs::write(&root, b"c").unwrap();
        roots.push(root);
    }
    // The clashing root is last: every other entry has been enumerated,
    // inspected and published without a prompt before it is reached.
    let clash = source.join("clash.txt");
    fs::write(&clash, b"new").unwrap();
    fs::write(destination.join("clash.txt"), b"existing").unwrap();
    roots.push(clash.clone());
    let locale = Arc::new(CountingLocale::default());
    let mut h = localized(QualifiedLocaleInstallation::audited(locale.clone()));
    h.controller.enqueue(request(1, &roots, &destination)).await;
    let conflict = h.conflict(1).await;
    assert_eq!(conflict.source.path, clash);
    assert_eq!(conflict.destination, destination.join("clash.txt"));
    for item in [&conflict.source, &conflict.target] {
        assert!(item.metadata.modified.is_some(), "{item:?}");
        assert_eq!(
            item.metadata.localized_modified,
            Some(CountingLocale::text())
        );
    }
    assert_eq!(
        locale.calls(),
        2,
        "only the prompt's two entries are formatted"
    );
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Skip,
        })
        .await;
    let done = h.terminal(1).await;
    assert_eq!(done.skipped, 1, "{done:?}");
    assert_eq!(done.files_completed, 48, "{done:?}");
    assert_eq!(
        fs::read(destination.join("clash.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(locale.calls(), 2, "no other entry was ever formatted");
    h.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_conflict_prompt_shows_the_native_localized_time_of_both_entries() {
    let temp = fixture();
    let source = temp.path().join("report.txt");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    fs::write(&source, b"new").unwrap();
    fs::write(destination.join("report.txt"), b"existing").unwrap();
    // As installed by the application: the native object is also the locale.
    let native = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let mut h = localized(QualifiedLocaleInstallation::audited(native.clone()));
    h.controller
        .enqueue(request(1, std::slice::from_ref(&source), &destination))
        .await;
    let conflict = h.conflict(1).await;
    for item in [&conflict.source, &conflict.target] {
        let modified = item
            .metadata
            .modified
            .expect("the adapter reports raw times");
        // The very text the adapter used to attach to every transfer entry.
        let native_text = native.format_datetime(modified);
        assert!(native_text.is_some(), "{item:?}");
        assert_eq!(item.metadata.localized_modified, native_text);
    }
    h.controller
        .decide(CopyDecision {
            id: conflict.id,
            version: conflict.version,
            choice: CopyChoice::Cancel,
        })
        .await;
    assert_eq!(h.terminal(1).await.state, CopyOperationState::Cancelled);
    assert_eq!(
        fs::read(destination.join("report.txt")).unwrap(),
        b"existing"
    );
    h.close().await;
}

#[test]
fn replace_all_resolves_later_conflicts_without_formatting_their_entries() {
    let temp = fixture();
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let names = ["first", "second", "third"];
    let sources: Vec<_> = names
        .iter()
        .map(|name| {
            let path = temp.path().join(name);
            fs::write(&path, name.as_bytes()).unwrap();
            fs::write(destination.join(name), b"old").unwrap();
            path
        })
        .collect();
    let request = request(1, &sources, &destination);
    let status = Arc::new(Mutex::new(initial(&request.clone().into())));
    let service = midnight_platform::LocalFileSystem::new(None);
    let mut job = worker::Job::prepare(request, &service, Cancellation::default(), status).unwrap();
    let locale = Arc::new(CountingLocale::default());
    job.set_locale(QualifiedLocaleInstallation::audited(locale.clone()));
    let mut prompts = 0;
    for step in 0..1_000 {
        match job.step() {
            worker::Step::Conflict(conflict) => {
                prompts += 1;
                assert!(conflict.can_replace, "{conflict:?}");
                assert_eq!(
                    conflict.target.metadata.localized_modified,
                    Some(CountingLocale::text())
                );
                assert!(job.accept_decision(CopyDecision {
                    id: conflict.id,
                    version: conflict.version,
                    choice: CopyChoice::ReplaceRecoverable { all: true },
                }));
            }
            worker::Step::Failure(failure) => panic!("unexpected failure: {failure:?}"),
            worker::Step::Done => break,
            worker::Step::Continue => {}
        }
        assert!(step < 999, "the copy did not finish");
    }
    assert_eq!(
        prompts, 1,
        "Replace all resolves the later conflicts itself"
    );
    assert_eq!(
        locale.calls(),
        2,
        "automatically resolved conflicts are never formatted"
    );
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    for name in names {
        assert_eq!(fs::read(destination.join(name)).unwrap(), name.as_bytes());
    }
}
