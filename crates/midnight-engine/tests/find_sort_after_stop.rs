use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc};

/// Enumerates real children, then blocks before the child after `pause_after`
/// until the search is cancelled. `slow_child` makes that child arrive only
/// after the given real delay, while the search is still running.
struct PausedEnumeration {
    native: Arc<LocalFileSystem>,
    pause_after: usize,
    slow_child: Option<(usize, Duration)>,
    paused: Arc<Notify>,
}
struct Source {
    inner: Box<dyn FindSource>,
    count: usize,
    pause_after: usize,
    slow_child: Option<(usize, Duration)>,
    paused: Arc<Notify>,
}
impl FindService for PausedEnumeration {
    fn open_find_source(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn FindSource>, FsError> {
        Ok(Box::new(Source {
            inner: self.native.open_find_source(path, cancel)?,
            count: 0,
            pause_after: self.pause_after,
            slow_child: self.slow_child,
            paused: self.paused.clone(),
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
        if self.count == self.pause_after {
            self.paused.notify_one();
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            return Ok(None);
        }
        if let Some((index, delay)) = self.slow_child
            && index == self.count
        {
            std::thread::sleep(delay);
        }
        let child = self.inner.next_child(cancel)?;
        if child.is_some() {
            self.count += 1;
        }
        Ok(child)
    }
}

struct Search {
    send: midnight_engine::CommandSender,
    receive: mpsc::Receiver<Event>,
    engine: tokio::task::JoinHandle<()>,
}

fn files(directory: &Path, count: usize) {
    for index in 0..count {
        std::fs::write(
            directory.join(format!("file-{index:03}")),
            vec![42; index + 1],
        )
        .unwrap();
    }
}

async fn start(directory: &Path, service: PausedEnumeration) -> Search {
    let (events, receive) = mpsc::channel(16);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        service.native.clone(),
        events,
    );
    let (send, commands) = engine.command_channel(16).unwrap();
    engine.configure_find().unwrap().install(Arc::new(service));
    let engine = tokio::spawn(engine.run(commands));
    send.send(Command::StartFind(Box::new(FindRequest::new(
        FindSessionId(1),
        FindGeneration(1),
        vec![directory.to_owned()],
    ))))
    .await
    .unwrap();
    Search {
        send,
        receive,
        engine,
    }
}

impl Search {
    async fn next_find(&mut self) -> FindUpdate {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), self.receive.recv())
                .await
                .expect("Find event")
                .expect("engine output");
            if let Event::Find(event) = event {
                return event.update;
            }
        }
    }

    async fn stop(&self) {
        self.send
            .send(Command::CancelFind {
                session: FindSessionId(1),
                generation: FindGeneration(1),
            })
            .await
            .unwrap();
    }

    async fn close(self) {
        drop(self.send);
        tokio::time::timeout(Duration::from_secs(5), self.engine)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn stopped_search_retains_partial_results_and_headers_can_still_sort_them() {
    let temporary = tempfile::tempdir().unwrap();
    files(temporary.path(), FIND_BATCH_SIZE);
    let native = Arc::new(LocalFileSystem::new(None));
    let mut search = start(
        temporary.path(),
        PausedEnumeration {
            native,
            pause_after: FIND_BATCH_SIZE,
            slow_child: None,
            paused: Arc::default(),
        },
    )
    .await;
    let results = loop {
        if let FindUpdate::Batch(batch) = search.next_find().await {
            break batch;
        }
    };
    assert_eq!(results.len(), FIND_BATCH_SIZE);
    search.stop().await;
    loop {
        if let FindUpdate::Finished(summary) = search.next_find().await {
            assert_eq!(summary.completion, FindCompletion::Cancelled);
            break;
        }
    }
    search
        .send
        .send(Command::SortFind(FindSortRequest {
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
    let sorted = loop {
        if let FindUpdate::Sorted(order) = search.next_find().await
            && order.revision == 1
        {
            break order;
        }
    };
    search.close().await;
    let first = &results[(sorted.ids[0].0 - 1) as usize];
    assert_eq!(first.entry.metadata.size, Some(FIND_BATCH_SIZE as u64));
}

#[tokio::test]
async fn stop_publishes_matches_below_one_batch_and_orders_them_automatically() {
    let temporary = tempfile::tempdir().unwrap();
    let count = FIND_BATCH_SIZE - 1;
    files(temporary.path(), count);
    let native = Arc::new(LocalFileSystem::new(None));
    let paused = Arc::new(Notify::new());
    let mut search = start(
        temporary.path(),
        PausedEnumeration {
            native,
            pause_after: count,
            slow_child: None,
            paused: paused.clone(),
        },
    )
    .await;
    // Every match is found, revalidated and counted before Stop.
    tokio::time::timeout(Duration::from_secs(5), paused.notified())
        .await
        .expect("enumeration reaches the pause");
    search.stop().await;
    let mut retained = Vec::new();
    let mut finished = false;
    // The default Name order follows Stop without another header click; it can
    // arrive before or after the terminal event.
    let mut order = None;
    while !finished || order.is_none() {
        match search.next_find().await {
            FindUpdate::Batch(batch) => retained.extend(batch.iter().cloned()),
            FindUpdate::Finished(summary) => {
                assert_eq!(summary.completion, FindCompletion::Cancelled);
                assert_eq!(summary.progress.results, count as u64);
                finished = true;
            }
            FindUpdate::Sorted(sorted) if sorted.ids.len() == count => order = Some(sorted),
            _ => {}
        }
    }
    assert_eq!(
        retained.len(),
        count,
        "Stop keeps every found match, not only full batches"
    );
    let order = order.unwrap();
    search.close().await;
    assert_eq!(order.revision, 0);
    let names: Vec<_> = order
        .ids
        .iter()
        .map(|id| retained[(id.0 - 1) as usize].entry.path.clone())
        .collect();
    let mut expected = names.clone();
    expected.sort();
    assert_eq!(names, expected);
}

#[tokio::test]
async fn a_running_search_publishes_a_partial_batch_on_the_progress_cadence() {
    let temporary = tempfile::tempdir().unwrap();
    files(temporary.path(), 4);
    let native = Arc::new(LocalFileSystem::new(None));
    let started = Instant::now();
    // The fourth child arrives after one progress interval; nothing else ever
    // completes a 64-row batch before the search is stopped.
    let mut search = start(
        temporary.path(),
        PausedEnumeration {
            native,
            pause_after: 4,
            slow_child: Some((3, Duration::from_millis(260))),
            paused: Arc::default(),
        },
    )
    .await;
    let batch = loop {
        match search.next_find().await {
            FindUpdate::Batch(batch) => break batch,
            FindUpdate::Finished(_) => panic!("partial matches must stream before the search ends"),
            _ => {}
        }
    };
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert_eq!(batch.len(), 4);
    search.stop().await;
    loop {
        match search.next_find().await {
            FindUpdate::Batch(batch) => panic!("no match remained unpublished: {}", batch.len()),
            FindUpdate::Finished(summary) => {
                assert_eq!(summary.completion, FindCompletion::Cancelled);
                assert_eq!(summary.progress.results, 4);
                break;
            }
            _ => {}
        }
    }
    search.close().await;
}
