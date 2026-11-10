use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::mpsc;

struct PausedEnumeration(Arc<LocalFileSystem>);
struct Source {
    inner: Box<dyn FindSource>,
    count: usize,
}
impl FindService for PausedEnumeration {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        Ok(Box::new(Source {
            inner: self.0.open_find_source(path, cancel)?,
            count: 0,
        }))
    }
}
impl FindSource for Source {
    fn entry(&self) -> &FindEntry {
        self.inner.entry()
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        self.inner.revalidate(cancel)
    }
    fn read_chunk(&mut self, buffer: &mut [u8], cancel: &Cancellation) -> Result<usize, FsError> {
        self.inner.read_chunk(buffer, cancel)
    }
    fn next_child(&mut self, cancel: &Cancellation) -> Result<Option<FindChild>, FsError> {
        if self.count == FIND_BATCH_SIZE {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            return Ok(None);
        }
        let child = self.inner.next_child(cancel)?;
        if child.is_some() {
            self.count += 1;
        }
        Ok(child)
    }
}
#[tokio::test]
async fn stopped_search_retains_partial_results_and_headers_can_still_sort_them() {
    let temporary = tempfile::tempdir().unwrap();
    for index in 0..FIND_BATCH_SIZE {
        std::fs::write(
            temporary.path().join(format!("file-{index:03}")),
            vec![42; index + 1],
        )
        .unwrap();
    }
    let native = Arc::new(LocalFileSystem::new(None));
    let service = Arc::new(PausedEnumeration(native.clone()));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(Engine::new(native, events).with_find(service).run(commands));
    send.send(Command::StartFind(Box::new(FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![temporary.path().to_owned()],
    ))))
    .await
    .unwrap();
    let results = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::Find(event)) = receive.recv().await
                && let FindUpdate::Batch(batch) = event.update
            {
                break batch;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(results.len(), FIND_BATCH_SIZE);
    send.send(Command::CancelFind {
        session: FindSessionId(1),
        generation: FindGeneration(1),
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Event::Find(event)) = receive.recv().await
                && let FindUpdate::Finished(summary) = event.update
            {
                assert_eq!(summary.completion, FindCompletion::Cancelled);
                break;
            }
        }
    })
    .await
    .unwrap();
    send.send(Command::SortFind(FindSortRequest {
        session: FindSessionId(1),
        generation: FindGeneration(1),
        revision: 1,
        sort: FindSortSpec {
            column: FindSortColumn::Size,
            descending: true,
        },
    }))
    .await
    .unwrap();
    let sorted = tokio::time::timeout(Duration::from_millis(1500), async {
        loop {
            if let Some(Event::Find(event)) = receive.recv().await
                && let FindUpdate::Sorted(order) = event.update
                && order.revision == 1
            {
                break order;
            }
        }
    })
    .await
    .ok();
    drop(send);
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .unwrap()
        .unwrap();
    let sorted = sorted.expect("Stop must not permanently cancel ordering of retained results");
    let first = &results[(sorted.ids[0].0 - 1) as usize];
    assert_eq!(first.entry.metadata.size, Some(FIND_BATCH_SIZE as u64));
}
