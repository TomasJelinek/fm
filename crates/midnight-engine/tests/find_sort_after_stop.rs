use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{collections::VecDeque, path::Path, sync::Arc, time::Duration};
use tokio::sync::{Notify, mpsc};

/// Enumerates real children, then blocks before the child after `pause_after`
/// until the search is cancelled. The worker's partial-batch cadence is
/// covered without real waits by `find_backend::partial_batches_follow_*`.
struct PausedEnumeration {
    native: Arc<LocalFileSystem>,
    pause_after: usize,
    paused: Arc<Notify>,
}
struct Source {
    inner: Box<dyn FindSource>,
    count: usize,
    pause_after: usize,
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
    /// Find updates read while the test waited for something else, oldest first.
    read: VecDeque<FindUpdate>,
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
        read: VecDeque::new(),
        engine,
    }
}

impl Search {
    async fn next_find(&mut self) -> FindUpdate {
        if let Some(update) = self.read.pop_front() {
            return update;
        }
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

    /// Waits until the enumeration reaches its pause, keeping the Find updates
    /// sent meanwhile for `next_find`.
    ///
    /// The worker publishes its start, and progress with any partial batch every
    /// 250 ms, before it reaches the pause, waiting for room in the bounded event
    /// channel; the Engine's own startup events share that channel. Without
    /// reading it here, a slow enumeration could fill it and never pause.
    async fn until_paused(&mut self, paused: &Notify) {
        let paused = paused.notified();
        tokio::pin!(paused);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    () = &mut paused => return,
                    event = self.receive.recv() => {
                        if let Event::Find(event) = event.expect("engine output") {
                            self.read.push_back(event.update);
                        }
                    }
                }
            }
        })
        .await
        .expect("enumeration reaches the pause");
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
            paused: paused.clone(),
        },
    )
    .await;
    // Every match is found, revalidated and counted before Stop.
    search.until_paused(&paused).await;
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
