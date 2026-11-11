//! Ordered, bounded selection work. Neither bitsets nor raw paths are walked by UI.
use crate::Outbox;
use midnight_core::*;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinSet,
};

const QUEUE_CAPACITY: usize = 32;

enum Message {
    Request(Box<SelectionRequest>),
    Wake,
}

pub(crate) struct Controller {
    pub(crate) streaming: crate::streaming_selection::StreamingSelections,
    senders: [Option<mpsc::Sender<Message>>; 2],
    failures: Arc<Mutex<[Option<Event>; 2]>>,
    stop: Arc<StopSignal>,
}

#[derive(Default)]
struct StopSignal {
    cancel: Cancellation,
    wake: Notify,
}

impl StopSignal {
    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let streaming = crate::streaming_selection::StreamingSelections::default();
        let stop = Arc::new(StopSignal::default());
        let failures: Arc<Mutex<[Option<Event>; 2]>> = Arc::new(Mutex::new([None, None]));
        let senders = std::array::from_fn(|slot| {
            let (sender, input) = mpsc::channel::<Message>(QUEUE_CAPACITY);
            let outbox = outbox.clone();
            let stop = stop.clone();
            let failures = failures.clone();
            tasks.spawn(run(slot, input, outbox, failures, stop, streaming.clone()));
            Some(sender)
        });
        Self {
            streaming,
            senders,
            failures,
            stop,
        }
    }

    pub(crate) fn submit(&self, request: SelectionRequest) {
        let slot = request.pane.index();
        let Some(sender) = &self.senders[slot] else {
            return;
        };
        if let Err(error) = sender.try_send(Message::Request(Box::new(request))) {
            let Message::Request(request) = error.into_inner() else {
                return;
            };
            self.failures.lock().unwrap_or_else(|e| e.into_inner())[slot] =
                Some(Event::SelectionFailed {
                    pane: request.pane,
                    navigation: request.navigation,
                    revision: request.revision,
                    error: FsError::from_io(
                        "update selection",
                        &request.location,
                        std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "Selection queue is busy. Try again.",
                        ),
                    ),
                });
            // If the FIFO drained between try_send and mailbox publication,
            // wake its sleeping worker. If full, its eventual drain sees the
            // already-published refusal before it can sleep.
            if let Some(sender) = &self.senders[slot] {
                let _ = sender.try_send(Message::Wake);
            }
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop.cancel.cancel();
        self.stop.wake.notify_waiters();
        self.senders = [None, None];
    }
}

async fn run(
    slot: usize,
    mut input: mpsc::Receiver<Message>,
    outbox: Outbox,
    failures: Arc<Mutex<[Option<Event>; 2]>>,
    stop: Arc<StopSignal>,
    streaming: crate::streaming_selection::StreamingSelections,
) {
    let mut state = State::default();
    // Receiving and backpressure are async. Idle panes own no blocking task,
    // and each pane awaits its single actual CPU job before taking another.
    while let Some(message) = input.recv().await {
        if stop.is_cancelled() {
            break;
        }
        if let Message::Request(request) = message {
            let failure = (
                request.pane,
                request.navigation,
                request.revision,
                request.location.clone(),
            );
            let stream = streaming.clone();
            let computed = tokio::task::spawn_blocking(move || {
                let selected = if request.view.is_none()
                    && matches!(request.action, SelectionAction::Range { .. })
                {
                    stream
                        .snapshot(&request)
                        .and_then(|view| state.apply_with_stream(&request, Some(view)))
                } else {
                    state.apply(&request)
                };
                let event = match selected {
                    Ok(Some((selection, deferred))) => Some(Event::SelectionChanged {
                        pane: request.pane,
                        navigation: request.navigation,
                        revision: request.revision,
                        selection,
                        deferred,
                    }),
                    Ok(None) => None,
                    Err(message) => Some(Event::SelectionFailed {
                        pane: request.pane,
                        navigation: request.navigation,
                        revision: request.revision,
                        error: FsError::from_io(
                            "update selection",
                            &request.location,
                            std::io::Error::new(std::io::ErrorKind::InvalidData, message),
                        ),
                    }),
                };
                (state, event)
            })
            .await;
            let event = match computed {
                Ok((updated, event)) => {
                    state = updated;
                    event
                }
                Err(error) => {
                    state = State::default();
                    Some(Event::SelectionFailed {
                        pane: failure.0,
                        navigation: failure.1,
                        revision: failure.2,
                        error: FsError::from_io(
                            "selection worker",
                            &failure.3,
                            std::io::Error::other(error.to_string()),
                        ),
                    })
                }
            };
            if let Some(event) = event
                && !deliver(&outbox, event, &stop).await
            {
                break;
            }
        }
        // A refusal cannot overtake any earlier accepted intent. In particular
        // an old Wake cannot deliver a newer refusal ahead of queued requests.
        let failure = {
            let mut failures = failures.lock().unwrap_or_else(|e| e.into_inner());
            let can_flush = input.is_empty()
                || failures[slot].as_ref().is_some_and(|event| {
                    matches!(event, Event::SelectionFailed { navigation, revision, .. }
                    if (*navigation, *revision) < (state.navigation, state.revision))
                });
            can_flush.then(|| failures[slot].take()).flatten()
        };
        if let Some(event) = failure
            && !deliver(&outbox, event, &stop).await
        {
            break;
        }
    }
}

async fn deliver(outbox: &Outbox, event: Event, stop: &StopSignal) -> bool {
    let cancelled = stop.wake.notified();
    tokio::pin!(cancelled);
    cancelled.as_mut().enable();
    if stop.is_cancelled() {
        return false;
    }
    tokio::select! {
        _ = cancelled => false,
        sent = outbox.send(event) => sent,
    }
}

#[derive(Default)]
struct State {
    location: PathBuf,
    navigation: NavigationId,
    revision: SelectionRevision,
    context: NavigationId,
    current: Arc<SelectionSnapshot>,
    deferred: Option<Deferred>,
    protected: Option<(SelectionOrigin, Arc<SelectionSnapshot>)>,
    authoritative: u64,
}

#[derive(Clone, Copy, Default)]
enum Transform {
    #[default]
    Identity,
    All,
    None,
    Invert,
}

impl Transform {
    fn apply(self, selected: bool) -> bool {
        match self {
            Self::Identity => selected,
            Self::All => true,
            Self::None => false,
            Self::Invert => !selected,
        }
    }

    fn inverted(self) -> Self {
        match self {
            Self::Identity => Self::Invert,
            Self::Invert => Self::Identity,
            Self::All => Self::None,
            Self::None => Self::All,
        }
    }
}

struct Deferred {
    base: Arc<SelectionSnapshot>,
    files: Transform,
    folders: Transform,
    toggles: HashSet<PathBuf>,
    directories: HashSet<PathBuf>,
}

impl State {
    fn apply(
        &mut self,
        request: &SelectionRequest,
    ) -> Result<Option<(Arc<SelectionSnapshot>, SelectionDefaults)>, &'static str> {
        self.apply_with_stream(request, None)
    }

    fn apply_with_stream(
        &mut self,
        request: &SelectionRequest,
        stream: Option<Arc<DirectoryIndex>>,
    ) -> Result<Option<(Arc<SelectionSnapshot>, SelectionDefaults)>, &'static str> {
        if request.navigation < self.navigation
            || (request.navigation == self.navigation && request.revision <= self.revision)
        {
            return Ok(None);
        }
        let authority = request
            .authoritative
            .as_ref()
            .filter(|seed| seed.id > self.authoritative);
        if let Some(seed) = authority {
            if seed.origin.navigation == self.context && seed.origin.location == self.location {
                self.current = seed.snapshot.clone();
                self.deferred = None;
            }
            // The worker may never have received a selection command in the old
            // source. Preserve the exact captured comparison for a failed move away.
            if request.preserve.as_ref() == Some(&seed.origin) {
                self.protected = Some((seed.origin.clone(), seed.snapshot.clone()));
            }
            self.authoritative = seed.id;
        }
        if let Some(origin) = &request.preserve {
            // A same-path refresh may already use a newer navigation token;
            // the still-live source selection is identified by its raw location.
            if self.location == origin.location && self.context == origin.navigation {
                self.protected = Some((origin.clone(), self.current.clone()));
            }
        }
        let desired = SelectionOrigin {
            navigation: request.context,
            location: request.location.clone(),
        };
        let restored = authority
            .filter(|seed| seed.origin == desired)
            .map(|seed| seed.snapshot.clone())
            .or_else(|| {
                self.protected
                    .as_ref()
                    .filter(|(saved, _)| *saved == desired)
                    .map(|(_, value)| value.clone())
            });
        if request.context != self.context || request.location != self.location {
            self.current = restored.unwrap_or_else(|| request.seed.clone());
            self.deferred = None;
        } else if request.seed_revision > self.revision {
            if request.seed_revision >= request.revision {
                return Err("An authoritative seed must precede its selection action");
            }
            self.current = request.seed.clone();
            self.deferred = None;
        }
        if request.preserve.is_none() {
            self.protected = None;
        }
        self.location = request.location.clone();
        self.navigation = request.navigation;
        self.context = request.context;
        self.revision = request.revision;
        if let Some(view) = &request.view {
            self.current = if let Some(deferred) = self.deferred.take() {
                deferred.rebase(view.clone())?
            } else {
                rebase(&self.current, view.clone())?
            };
            let mut bits = self.current.bits().to_vec();
            match &request.action {
                SelectionAction::Bulk(operation) => match operation {
                    SelectionOperation::All => bits.fill(u64::MAX),
                    SelectionOperation::FilesOnly => {
                        for (word, directories) in bits.iter_mut().zip(&view.directory_bits) {
                            *word = !directories;
                        }
                    }
                    SelectionOperation::ClearFilesOnly => {
                        for (word, directories) in bits.iter_mut().zip(&view.directory_bits) {
                            *word &= directories;
                        }
                    }
                    SelectionOperation::Clear => bits.fill(0),
                    SelectionOperation::Invert => {
                        for (word, directories) in bits.iter_mut().zip(&view.directory_bits) {
                            *word ^= !directories;
                        }
                    }
                    SelectionOperation::InvertAll => {
                        for word in &mut bits {
                            *word = !*word;
                        }
                    }
                },
                SelectionAction::Toggle(path) => {
                    if let Some(position) = view.by_path.get(path).and_then(|id| view.by_id.get(id))
                    {
                        bits[position / 64] ^= 1u64 << (position % 64);
                    }
                }
                SelectionAction::Remove(paths) => {
                    for path in paths.iter() {
                        if let Some(position) =
                            view.by_path.get(path).and_then(|id| view.by_id.get(id))
                        {
                            bits[position / 64] &= !(1u64 << (position % 64));
                        }
                    }
                }
                SelectionAction::Range {
                    anchor,
                    target,
                    additive,
                } => {
                    let position = |path| {
                        view.by_path
                            .get(path)
                            .and_then(|id| view.by_id.get(id))
                            .copied()
                    };
                    let (Some(anchor), Some(target)) = (position(anchor), position(target)) else {
                        return Err("Range endpoints no longer belong to the captured view");
                    };
                    if !additive {
                        bits.fill(0);
                    }
                    for position in anchor.min(target)..=anchor.max(target) {
                        bits[position / 64] |= 1 << (position % 64);
                    }
                }
                SelectionAction::Ranges { ranges, mode } => {
                    if ranges.len() != view.by_id.len() {
                        return Err("Selection rectangle no longer matches its captured view");
                    }
                    if *mode == SelectionMode::Replace {
                        bits.fill(0);
                    }
                    for span in ranges.spans() {
                        for position in span.first..=span.last {
                            let bit = 1 << (position % 64);
                            let word = &mut bits[position / 64];
                            if *mode == SelectionMode::Toggle {
                                *word ^= bit;
                            } else {
                                *word |= bit;
                            }
                        }
                    }
                }
                SelectionAction::Rebase => {}
            }
            let count = self.current.basis().by_id.len();
            if !count.is_multiple_of(64)
                && let Some(last) = bits.last_mut()
            {
                *last &= (1u64 << (count % 64)) - 1;
            }
            self.current = Arc::new(self.current.with_bits(bits)?);
            Ok(Some((self.current.clone(), SelectionDefaults::NONE)))
        } else {
            let deferred = self.deferred.get_or_insert_with(|| Deferred {
                base: self.current.clone(),
                files: Transform::Identity,
                folders: Transform::Identity,
                toggles: HashSet::new(),
                directories: HashSet::new(),
            });
            match &request.action {
                SelectionAction::Bulk(operation) => match operation {
                    SelectionOperation::All => {
                        deferred.files = Transform::All;
                        deferred.folders = Transform::All;
                        deferred.toggles.clear();
                    }
                    SelectionOperation::FilesOnly => {
                        deferred.files = Transform::All;
                        deferred.folders = Transform::None;
                        deferred.toggles.clear();
                    }
                    SelectionOperation::ClearFilesOnly => {
                        deferred.files = Transform::None;
                        // Directory exceptions and their inherited defaults must survive;
                        // file toggles must not resurrect marks after the final rebase.
                        let directories = &deferred.directories;
                        let basis = deferred.base.basis();
                        deferred.toggles.retain(|path| {
                            directories.contains(path)
                                || basis
                                    .by_path
                                    .get(path)
                                    .is_some_and(|id| basis.is_directory(*id))
                        });
                    }
                    SelectionOperation::Clear => {
                        deferred.files = Transform::None;
                        deferred.folders = Transform::None;
                        deferred.toggles.clear();
                    }
                    SelectionOperation::Invert => {
                        deferred.files = deferred.files.inverted();
                    }
                    SelectionOperation::InvertAll => {
                        deferred.files = deferred.files.inverted();
                        deferred.folders = deferred.folders.inverted();
                    }
                },
                SelectionAction::Toggle(path) => {
                    if request.toggle_directory {
                        deferred.directories.insert(path.clone());
                    } else {
                        deferred.directories.remove(path);
                    }
                    if !deferred.toggles.remove(path) {
                        deferred.toggles.insert(path.clone());
                    }
                }
                SelectionAction::Range {
                    anchor,
                    target,
                    additive,
                } => {
                    let view = stream
                        .as_ref()
                        .ok_or("The streamed range index is unavailable")?;
                    let position = |path| {
                        view.by_path
                            .get(path)
                            .and_then(|id| view.by_id.get(id))
                            .copied()
                    };
                    let (Some(anchor), Some(target)) = (position(anchor), position(target)) else {
                        return Err("Range endpoints no longer belong to the streamed view");
                    };
                    if !additive {
                        deferred.files = Transform::None;
                        deferred.folders = Transform::None;
                        deferred.toggles.clear();
                    }
                    for (path, id) in &view.by_path {
                        if (anchor.min(target)..=anchor.max(target)).contains(&view.by_id[id]) {
                            if view.is_directory(*id) {
                                deferred.directories.insert(path.clone());
                            }
                            if !deferred.contains(path) && !deferred.toggles.remove(path) {
                                deferred.toggles.insert(path.clone());
                            }
                        }
                    }
                }
                SelectionAction::Ranges { .. } => {
                    return Err("Selection rectangles require a complete immutable view");
                }
                SelectionAction::Remove(paths) => {
                    for path in paths.iter() {
                        if deferred.contains(path) && !deferred.toggles.remove(path) {
                            deferred.toggles.insert(path.clone());
                        }
                    }
                }
                SelectionAction::Rebase => {}
            }
            self.current = deferred.rebase(stream.unwrap_or_else(|| deferred.subset()))?;
            let pending = SelectionDefaults {
                files: deferred.files.apply(false),
                directories: deferred.folders.apply(false),
            };
            Ok(Some((self.current.clone(), pending)))
        }
    }
}

impl Deferred {
    fn is_directory(&self, path: &std::path::Path) -> bool {
        self.directories.contains(path)
            || self
                .base
                .basis()
                .by_path
                .get(path)
                .is_some_and(|id| self.base.basis().is_directory(*id))
    }

    fn selected(&self, path: &std::path::Path, directory: bool) -> bool {
        let transform = if directory { self.folders } else { self.files };
        transform.apply(self.base.contains_path(path)) ^ self.toggles.contains(path)
    }

    fn contains(&self, path: &std::path::Path) -> bool {
        self.selected(path, self.is_directory(path))
    }
    fn subset(&self) -> Arc<DirectoryIndex> {
        let mut paths: HashSet<_> = self.base.basis().by_path.keys().cloned().collect();
        paths.extend(self.toggles.iter().cloned());
        let mut by_id = HashMap::with_capacity(paths.len());
        let mut by_path = HashMap::with_capacity(paths.len());
        let mut directory_bits = vec![0; paths.len().div_ceil(64)];
        for (position, path) in paths.into_iter().enumerate() {
            let id = EntryId(position as u64);
            if self.is_directory(&path) {
                directory_bits[position / 64] |= 1 << (position % 64);
            }
            by_id.insert(id, position);
            by_path.insert(path, id);
        }
        Arc::new(DirectoryIndex {
            column_samples: Default::default(),
            file_sizes: vec![None; by_id.len()],
            by_id,
            by_path,
            directory_bits,
        })
    }

    fn rebase(&self, view: Arc<DirectoryIndex>) -> Result<Arc<SelectionSnapshot>, &'static str> {
        let blank =
            SelectionSnapshot::from_bits(view.clone(), vec![0; view.by_id.len().div_ceil(64)])?;
        let mut bits = blank.bits().to_vec();
        for (path, id) in &view.by_path {
            let selected = self.selected(path, view.is_directory(*id));
            if selected {
                let position = view.by_id[id];
                bits[position / 64] |= 1u64 << (position % 64);
            }
        }
        Ok(Arc::new(blank.with_bits(bits)?))
    }
}

fn rebase(
    snapshot: &Arc<SelectionSnapshot>,
    view: Arc<DirectoryIndex>,
) -> Result<Arc<SelectionSnapshot>, &'static str> {
    if Arc::ptr_eq(snapshot.basis(), &view) {
        return Ok(snapshot.clone());
    }
    Deferred {
        base: snapshot.clone(),
        files: Transform::Identity,
        folders: Transform::Identity,
        toggles: HashSet::new(),
        directories: HashSet::new(),
    }
    .rebase(view)
}

pub(crate) struct PreparedProperties {
    pub(crate) request: DesktopRequestId,
    pub(crate) paths: Vec<PathBuf>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) struct PropertiesInput {
    pub(crate) request: DesktopRequestId,
    pub(crate) selection: Arc<SelectionSnapshot>,
    pub(crate) view: Arc<DirectoryIndex>,
    pub(crate) fallback: Option<PathBuf>,
}

pub(crate) fn prepare_properties(
    input: PropertiesInput,
    budget: Arc<tokio::sync::Semaphore>,
    outbox: Outbox,
    ready: mpsc::Sender<PreparedProperties>,
    tasks: &mut JoinSet<()>,
) {
    let PropertiesInput {
        request,
        selection,
        view,
        fallback,
    } = input;
    let Ok(permit) = budget.try_acquire_owned() else {
        tasks.spawn(async move {
            outbox
                .send(Event::DesktopActionFinished {
                    request,
                    result: Err(FsError::from_io(
                        "prepare Properties",
                        std::path::Path::new(""),
                        std::io::Error::new(
                            std::io::ErrorKind::WouldBlock,
                            "Properties preparation is busy. Try again.",
                        ),
                    )),
                })
                .await;
        });
        return;
    };
    tasks.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            let paths: Vec<PathBuf> = if selection.is_empty() {
                fallback.into_iter().collect()
            } else {
                selection
                    .selected_paths()
                    .filter(|path| view.by_path.contains_key(*path))
                    .cloned()
                    .collect()
            };
            if paths.is_empty() {
                Err(FsError::from_io(
                    "prepare Properties",
                    std::path::Path::new(""),
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "No selected entries remain in the captured folder.",
                    ),
                ))
            } else {
                Ok(PreparedProperties {
                    request,
                    paths,
                    _permit: permit,
                })
            }
        })
        .await;
        match result {
            Ok(Ok(prepared)) => {
                let _ = ready.send(prepared).await;
            }
            Ok(Err(error)) => {
                outbox
                    .send(Event::DesktopActionFinished {
                        request,
                        result: Err(error),
                    })
                    .await;
            }
            Err(error) => {
                outbox
                    .send(Event::DesktopActionFinished {
                        request,
                        result: Err(FsError::from_io(
                            "prepare Properties",
                            std::path::Path::new(""),
                            std::io::Error::other(error.to_string()),
                        )),
                    })
                    .await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, time::Duration};

    fn view(names: &[&str]) -> Arc<DirectoryIndex> {
        Arc::new(DirectoryIndex::from_entries(
            &names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    Entry::new(
                        EntryId(100 + i as u64),
                        PathBuf::from(name),
                        EntryKind::File,
                    )
                })
                .collect::<Vec<_>>(),
        ))
    }

    pub(super) fn request(
        revision: u64,
        view: Option<Arc<DirectoryIndex>>,
        action: SelectionAction,
    ) -> SelectionRequest {
        SelectionRequest {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(revision),
            context: NavigationId(1),
            location: "root".into(),
            view,
            seed: Arc::default(),
            seed_revision: SelectionRevision(0),
            authoritative: None,
            reset: false,
            preserve: None,
            restore: None,
            action,
            toggle_directory: false,
        }
    }

    fn apply(state: &mut State, request: SelectionRequest) -> Arc<SelectionSnapshot> {
        state.apply(&request).unwrap().unwrap().0
    }

    #[test]
    fn completion_removal_is_idempotent_and_survives_stream_rebase() {
        for streaming in [false, true] {
            let index = view(&["a", "文档", "c"]);
            let mut state = State::default();
            let initial = (!streaming).then(|| index.clone());
            apply(
                &mut state,
                request(
                    1,
                    initial.clone(),
                    SelectionAction::Bulk(SelectionOperation::All),
                ),
            );
            let removed: Arc<HashSet<PathBuf>> =
                Arc::new([PathBuf::from("文档")].into_iter().collect());
            for revision in [2, 3] {
                apply(
                    &mut state,
                    request(
                        revision,
                        initial.clone(),
                        SelectionAction::Remove(removed.clone()),
                    ),
                );
            }
            let result = apply(&mut state, request(4, Some(index), SelectionAction::Rebase));
            assert!(result.contains_path(Path::new("a")));
            assert!(!result.contains_path(Path::new("文档")));
            assert!(result.contains_path(Path::new("c")));
            assert_eq!(result.len(), 2);
        }
    }

    #[test]
    fn rectangle_disjoint_ranges_keep_raw_identities_and_modifier_semantics() {
        let index = view(&["é", "b", "文档", "d", "e", "f"]);
        let mut state = State::default();
        apply(
            &mut state,
            request(1, Some(index.clone()), SelectionAction::Toggle("e".into())),
        );
        let ranges = SelectionRanges::new(
            6,
            vec![
                SelectionSpan { first: 0, last: 0 },
                SelectionSpan { first: 2, last: 3 },
            ],
        )
        .unwrap();
        for (revision, mode, expected) in [
            (2, SelectionMode::Add, 0b011101),
            (3, SelectionMode::Toggle, 0b010000),
            (4, SelectionMode::Replace, 0b001101),
        ] {
            let result = apply(
                &mut state,
                request(
                    revision,
                    Some(index.clone()),
                    SelectionAction::Ranges {
                        ranges: ranges.clone(),
                        mode,
                    },
                ),
            );
            assert_eq!(result.bits(), &[expected]);
            assert!(Arc::ptr_eq(result.basis(), &index));
        }
        let stale = request(
            3,
            Some(index),
            SelectionAction::Bulk(SelectionOperation::Clear),
        );
        assert!(state.apply(&stale).unwrap().is_none());
    }

    #[test]
    fn rectangle_requires_complete_view_and_exact_source_extent() {
        let ranges = SelectionRanges::new(1, vec![SelectionSpan { first: 0, last: 0 }]).unwrap();
        let action = SelectionAction::Ranges {
            ranges,
            mode: SelectionMode::Replace,
        };
        assert!(
            State::default()
                .apply(&request(1, None, action.clone()))
                .is_err()
        );
        assert!(
            State::default()
                .apply(&request(1, Some(view(&["a", "b"])), action))
                .is_err()
        );
    }

    #[test]
    fn authoritative_comparison_seed_is_adopted_once_before_queued_toggles() {
        let mut state = State::default();
        let index = view(&["a", "b", "c"]);
        apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::Clear),
            ),
        );
        let seed = Arc::new(SelectionSnapshot::from_bits(index.clone(), vec![1]).unwrap());
        for (revision, name, expected) in [(3, "b", 0b011), (4, "c", 0b111)] {
            let mut next = request(
                revision,
                Some(index.clone()),
                SelectionAction::Toggle(name.into()),
            );
            next.seed = seed.clone();
            next.seed_revision = SelectionRevision(2);
            assert_eq!(apply(&mut state, next).bits(), &[expected]);
        }
    }

    #[test]
    fn comparison_selection_survives_a_refresh_that_resets_request_revisions() {
        let mut state = State::default();
        let index = view(&["a", "b", "c"]);
        apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::Clear),
            ),
        );
        let snapshot = Arc::new(SelectionSnapshot::from_bits(index.clone(), vec![1]).unwrap());
        let mut refresh = request(1, Some(index), SelectionAction::Rebase);
        refresh.navigation = NavigationId(2);
        refresh.seed = snapshot.clone();
        refresh.authoritative = Some(Arc::new(AuthoritativeSelection {
            id: 1,
            origin: SelectionOrigin {
                navigation: NavigationId(1),
                location: "root".into(),
            },
            snapshot,
        }));
        // The presenter starts request revisions over on each navigation.
        refresh.seed_revision = SelectionRevision(0);
        assert_eq!(apply(&mut state, refresh).bits(), &[1]);
    }

    #[test]
    fn comparison_authority_survives_failed_navigation_and_cannot_replay_over_edits() {
        for initialized in [false, true] {
            let mut state = State::default();
            let index = view(&["a", "b", "c"]);
            if initialized {
                apply(
                    &mut state,
                    request(
                        1,
                        Some(index.clone()),
                        SelectionAction::Bulk(SelectionOperation::Clear),
                    ),
                );
            }
            let origin = SelectionOrigin {
                navigation: NavigationId(1),
                location: "root".into(),
            };
            let authority = Arc::new(AuthoritativeSelection {
                id: 1,
                origin: origin.clone(),
                snapshot: Arc::new(SelectionSnapshot::from_bits(index.clone(), vec![1]).unwrap()),
            });
            let mut away = request(1, None, SelectionAction::Toggle("other/x".into()));
            away.navigation = NavigationId(2);
            away.context = NavigationId(2);
            away.location = "other".into();
            away.preserve = Some(origin.clone());
            away.authoritative = Some(authority.clone());
            apply(&mut state, away);
            let mut restore = request(2, Some(index.clone()), SelectionAction::Rebase);
            restore.navigation = NavigationId(2);
            restore.restore = Some(origin.clone());
            restore.authoritative = Some(authority.clone());
            assert_eq!(apply(&mut state, restore).bits(), &[1]);
            for (revision, path, expected) in [(3, "b", 0b011), (4, "a", 0b010)] {
                let mut edit = request(
                    revision,
                    Some(index.clone()),
                    SelectionAction::Toggle(path.into()),
                );
                edit.navigation = NavigationId(2);
                edit.authoritative = Some(authority.clone());
                assert_eq!(apply(&mut state, edit).bits(), &[expected]);
            }
            let mut next = request(5, Some(index.clone()), SelectionAction::Toggle("b".into()));
            next.navigation = NavigationId(2);
            next.authoritative = Some(Arc::new(AuthoritativeSelection {
                id: 2,
                origin,
                snapshot: Arc::new(
                    SelectionSnapshot::from_bits(index.clone(), vec![0b100]).unwrap(),
                ),
            }));
            assert_eq!(apply(&mut state, next).bits(), &[0b110]);
            let mut stale = request(6, Some(index), SelectionAction::Toggle("a".into()));
            stale.navigation = NavigationId(2);
            stale.authoritative = Some(authority);
            assert_eq!(apply(&mut state, stale).bits(), &[0b111]);
        }
    }

    #[test]
    fn all_invert_toggle_clear_are_ordered_and_ignore_numeric_entry_ids() {
        let mut state = State::default();
        let index = view(&["文档/e\u{301}", "文档/é", "👩🏽‍💻"]);
        let all = apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        assert_eq!(all.len(), 3);
        assert!(!all.contains_path(std::path::Path::new("..")));
        assert_eq!(
            apply(
                &mut state,
                request(
                    2,
                    Some(index.clone()),
                    SelectionAction::Bulk(SelectionOperation::Invert)
                )
            )
            .len(),
            0
        );
        let one = apply(
            &mut state,
            request(
                3,
                Some(index.clone()),
                SelectionAction::Toggle("文档/é".into()),
            ),
        );
        assert_eq!(one.len(), 1);
        assert!(one.contains_path(std::path::Path::new("文档/é")));
        assert!(!one.contains_path(std::path::Path::new("文档/e\u{301}")));
        assert!(
            apply(
                &mut state,
                request(
                    4,
                    Some(index),
                    SelectionAction::Bulk(SelectionOperation::Clear)
                )
            )
            .is_empty()
        );
    }

    #[test]
    fn streaming_bulk_and_toggle_order_survive_final_rebase() {
        let mut state = State::default();
        apply(
            &mut state,
            request(1, None, SelectionAction::Bulk(SelectionOperation::All)),
        );
        apply(
            &mut state,
            request(2, None, SelectionAction::Toggle("a".into())),
        );
        let selected = apply(
            &mut state,
            request(3, Some(view(&["a", "b"])), SelectionAction::Rebase),
        );
        assert!(!selected.contains_path(std::path::Path::new("a")));
        assert!(selected.contains_path(std::path::Path::new("b")));
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn refresh_preserves_paths_and_does_not_select_new_recycled_ids() {
        let mut state = State::default();
        apply(
            &mut state,
            request(
                1,
                Some(view(&["a", "b"])),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        let mut rebase = request(2, Some(view(&["new", "b"])), SelectionAction::Rebase);
        rebase.navigation = NavigationId(2);
        let selected = apply(&mut state, rebase);
        assert!(!selected.contains_path(std::path::Path::new("new")));
        assert!(selected.contains_path(std::path::Path::new("b")));
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn protected_working_origin_survives_failed_and_superseded_destinations() {
        let mut state = State::default();
        let original = view(&["a", "b"]);
        apply(
            &mut state,
            request(
                1,
                Some(original.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        // A same-directory refresh changes navigation, not the selection context.
        let mut refresh = request(
            1,
            Some(original.clone()),
            SelectionAction::Toggle("a".into()),
        );
        refresh.navigation = NavigationId(2);
        apply(&mut state, refresh);
        let origin = SelectionOrigin {
            navigation: NavigationId(1),
            location: "root".into(),
        };
        for generation in [3, 4] {
            let mut elsewhere = request(1, None, SelectionAction::Toggle("temporary".into()));
            elsewhere.navigation = NavigationId(generation);
            elsewhere.context = NavigationId(generation);
            elsewhere.location = format!("destination-{generation}").into();
            elsewhere.preserve = Some(origin.clone());
            apply(&mut state, elsewhere);
        }
        let mut restored = request(1, Some(original), SelectionAction::Rebase);
        restored.navigation = NavigationId(5);
        // A retry can have neither boolean reset nor restore metadata: the
        // persistent context barrier remains authoritative after queue refusal.
        restored.reset = false;
        restored.restore = None;
        let selected = apply(&mut state, restored);
        assert!(!selected.contains_path(std::path::Path::new("a")));
        assert!(selected.contains_path(std::path::Path::new("b")));
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn same_path_new_tab_context_resets_once_even_if_first_barrier_was_rejected() {
        let mut state = State::default();
        let index = view(&["a", "b"]);
        apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        let mut retried = request(2, Some(index.clone()), SelectionAction::Rebase);
        retried.navigation = NavigationId(2);
        retried.context = NavigationId(2);
        retried.reset = false;
        assert!(apply(&mut state, retried).is_empty());
        let mut next = request(
            3,
            Some(index),
            SelectionAction::Bulk(SelectionOperation::Invert),
        );
        next.navigation = NavigationId(2);
        next.context = NavigationId(2);
        assert_eq!(apply(&mut state, next).len(), 2);
    }

    #[test]
    fn complete_word_boundaries_never_select_padding_bits() {
        let mut state = State::default();
        let entries: Vec<_> = (0..65)
            .map(|id| {
                Entry::new(
                    EntryId(u64::MAX - id),
                    format!("row-{id}").into(),
                    EntryKind::File,
                )
            })
            .collect();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let selected = apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        assert_eq!(selected.len(), 65);
        assert_eq!(selected.bits(), [u64::MAX, 1]);
        assert!(
            apply(
                &mut state,
                request(
                    2,
                    Some(index),
                    SelectionAction::Bulk(SelectionOperation::Invert)
                )
            )
            .is_empty()
        );
    }

    #[tokio::test]
    async fn refused_burst_completes_after_earlier_accepted_snapshots_and_idle_wake_works() {
        let (sender, mut events) = mpsc::channel(1);
        let outbox = Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        };
        let mut tasks = JoinSet::new();
        let controller = Controller::new(outbox, &mut tasks);
        let index = view(&["a", "b"]);
        // Prevent event delivery from silently draining the complete input burst.
        for revision in 1..=100 {
            controller.submit(request(
                revision,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::Invert),
            ));
        }
        let mut latest = SelectionRevision(0);
        let refused = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match events.recv().await.unwrap() {
                    Event::SelectionChanged { revision, .. } => {
                        assert!(revision > latest);
                        latest = revision;
                    }
                    Event::SelectionFailed { revision, .. } => break revision,
                    _ => unreachable!(),
                }
            }
        })
        .await
        .unwrap();
        assert!(refused > latest);
        // Explicitly wake an idle worker after publication, reproducing the
        // publication-after-drain half of the refusal race without extra input.
        controller.failures.lock().unwrap()[0] = Some(Event::SelectionFailed {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(101),
            error: FsError::from_io(
                "selection",
                std::path::Path::new(""),
                std::io::ErrorKind::WouldBlock.into(),
            ),
        });
        controller.senders[0]
            .as_ref()
            .unwrap()
            .send(Message::Wake)
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(Event::SelectionFailed {
                revision: SelectionRevision(101),
                ..
            })
        ));
        drop(controller);
        drop(events);
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn removed_properties_selection_completes_with_error_instead_of_leaking_request() {
        let index = view(&["removed"]);
        let selection = Arc::new(SelectionSnapshot::from_bits(index, vec![1]).unwrap());
        let (sender, mut events) = mpsc::channel(2);
        let (ready, mut prepared) = mpsc::channel(2);
        let mut tasks = JoinSet::new();
        prepare_properties(
            PropertiesInput {
                request: DesktopRequestId(42),
                selection,
                view: view(&["new"]),
                fallback: None,
            },
            Arc::new(tokio::sync::Semaphore::new(2)),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            ready,
            &mut tasks,
        );
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .unwrap(),
            Some(Event::DesktopActionFinished {
                request: DesktopRequestId(42),
                result: Err(_)
            })
        ));
        assert!(prepared.try_recv().is_err());
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn old_wake_cannot_overtake_later_accepted_requests_after_refusal_advances() {
        let (sender, mut events) = mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Outbox {
                sender: sender.clone(),
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let index = view(&["a", "b"]);
        controller.submit(request(
            1,
            Some(index.clone()),
            SelectionAction::Bulk(SelectionOperation::All),
        ));
        assert!(matches!(
            events.recv().await,
            Some(Event::SelectionChanged {
                revision: SelectionRevision(1),
                ..
            })
        ));
        sender
            .send(Event::FontFamiliesLoaded(Ok(Vec::new())))
            .await
            .unwrap();
        controller.submit(request(
            2,
            Some(index.clone()),
            SelectionAction::Bulk(SelectionOperation::Invert),
        ));
        // Once dequeued, request2 cannot deliver its output until the dummy is
        // drained. This leaves a deterministic interval to queue an old Wake.
        tokio::time::timeout(Duration::from_secs(1), async {
            while controller.senders[0].as_ref().unwrap().capacity() != QUEUE_CAPACITY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        controller.senders[0]
            .as_ref()
            .unwrap()
            .send(Message::Wake)
            .await
            .unwrap();
        for revision in 3..=5 {
            controller.submit(request(
                revision,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::Invert),
            ));
        }
        controller.failures.lock().unwrap()[0] = Some(Event::SelectionFailed {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            revision: SelectionRevision(6),
            error: FsError::from_io(
                "selection",
                std::path::Path::new(""),
                std::io::ErrorKind::WouldBlock.into(),
            ),
        });
        assert!(matches!(
            events.recv().await,
            Some(Event::FontFamiliesLoaded(_))
        ));
        for expected in 2..=5 {
            assert!(
                matches!(tokio::time::timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
                Some(Event::SelectionChanged { revision, .. }) if revision == SelectionRevision(expected))
            );
        }
        assert!(matches!(
            events.recv().await,
            Some(Event::SelectionFailed {
                revision: SelectionRevision(6),
                ..
            })
        ));
        drop(controller);
        drop(events);
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test(start_paused = true)]
    async fn idle_controllers_do_not_keep_blocking_jobs_alive_or_stop_virtual_time() {
        let (sender, _events) = mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let (expired, watchdog) = tokio::sync::oneshot::channel();
        let (cancel, cancellation) = std::sync::mpsc::channel();
        let guard = std::thread::spawn(move || {
            if cancellation.recv_timeout(Duration::from_secs(1)).is_err() {
                let _ = expired.send(());
            }
        });
        let before = tokio::time::Instant::now();
        let advanced = tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(86_400)) => true,
            _ = watchdog => false,
        };
        let _ = cancel.send(());
        guard.join().unwrap();
        drop(controller);
        while tasks.join_next().await.is_some() {}
        assert!(
            advanced,
            "idle selection tasks must allow paused Tokio timers to auto-advance"
        );
        let elapsed = tokio::time::Instant::now() - before;
        assert!(elapsed >= Duration::from_secs(86_400));
        assert!(elapsed <= Duration::from_secs(86_400) + Duration::from_millis(1));
    }

    #[tokio::test]
    async fn shutdown_cancels_a_full_output_channel_without_more_input() {
        let (sender, _events) = mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        for revision in 1..=3 {
            controller.submit(request(
                revision,
                Some(view(&["a"])),
                SelectionAction::Bulk(SelectionOperation::All),
            ));
        }
        // Allow both queued output and asynchronous backpressure to become live.
        tokio::time::sleep(Duration::from_millis(10)).await;
        drop(controller);
        tokio::time::timeout(Duration::from_secs(1), async {
            while tasks.join_next().await.is_some() {}
        })
        .await
        .unwrap();
    }
    #[test]
    #[ignore = "explicit debug selection CPU benchmark"]
    fn interaction_selection_cpu() {
        use std::time::Instant;
        for count in [10_000, 100_000] {
            let entries: Vec<_> = (0..count)
                .map(|id| {
                    Entry::new(
                        EntryId(id),
                        format!("root/entry-{id:06}-文档.txt").into(),
                        EntryKind::File,
                    )
                })
                .collect();
            let index = Arc::new(DirectoryIndex::from_entries(&entries));
            let mut state = State::default();
            apply(
                &mut state,
                request(1, Some(index.clone()), SelectionAction::Rebase),
            );
            let mut values = Vec::new();
            for round in 0..100 {
                let req = request(
                    round + 2,
                    Some(index.clone()),
                    SelectionAction::Toggle(entries[round as usize].path.clone()),
                );
                let start = Instant::now();
                std::hint::black_box(apply(&mut state, req));
                values.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            values.sort_by(f64::total_cmp);
            println!(
                "selection_state_apply: rows={count} median_ms={:.4} p95_ms={:.4} n=100 (debug, actual backend CPU, no injected delay)",
                values[50], values[95]
            );
        }
    }
    #[test]
    fn range_selection_uses_inclusive_captured_order_and_native_paths() {
        let index = view(&["文档/a", "文档/b", "文档/c", "文档/d"]);
        let mut state = State::default();
        apply(
            &mut state,
            request(
                1,
                Some(index.clone()),
                SelectionAction::Bulk(SelectionOperation::All),
            ),
        );
        let range = |anchor: &str, target: &str, additive| SelectionAction::Range {
            anchor: anchor.into(),
            target: target.into(),
            additive,
        };
        let replaced = apply(
            &mut state,
            request(2, Some(index.clone()), range("文档/c", "文档/b", false)),
        );
        assert_eq!(replaced.bits(), [0b0110]);
        let added = apply(
            &mut state,
            request(3, Some(index.clone()), range("文档/c", "文档/d", true)),
        );
        assert_eq!(added.bits(), [0b1110]);
        assert!(
            state
                .apply(&request(4, Some(index), range("missing", "文档/b", false)))
                .is_err()
        );
        assert_eq!(
            state.current.bits(),
            [0b1110],
            "invalid anchor cannot clear committed selection"
        );
    }

    #[test]
    fn streamed_directory_toggle_reports_folder_count_before_listing_commit() {
        let mut state = State::default();
        let mut toggle = request(1, None, SelectionAction::Toggle("folder".into()));
        toggle.toggle_directory = true;
        let selected = apply(&mut state, toggle);
        assert_eq!((selected.file_count(), selected.folder_count()), (0, 1));
        let selected = apply(
            &mut state,
            request(2, None, SelectionAction::Toggle("file".into())),
        );
        assert_eq!((selected.file_count(), selected.folder_count()), (1, 1));
    }
    #[tokio::test]
    async fn streamed_range_is_resolved_by_worker_and_survives_final_sort() {
        let (sender, mut events) = mpsc::channel(8);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let entries = vec![
            Entry::new(EntryId(900), "root/z".into(), EntryKind::DirectoryLink),
            Entry::new(EntryId(7), "root/a".into(), EntryKind::File),
            Entry::new(EntryId(99), "root/m".into(), EntryKind::File),
        ];
        controller.streaming.append(
            PaneId::Left,
            NavigationId(1),
            std::path::Path::new("root"),
            &entries,
        );
        controller.submit(request(
            1,
            None,
            SelectionAction::Range {
                anchor: "root/z".into(),
                target: "root/a".into(),
                additive: false,
            },
        ));
        let Event::SelectionChanged {
            selection,
            deferred,
            ..
        } = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("streamed selection result")
        };
        assert!(!deferred.any());
        assert_eq!((selection.file_count(), selection.folder_count()), (1, 1));
        assert!(!selection.contains_path(std::path::Path::new("root/m")));
        let sorted = vec![entries[1].clone(), entries[2].clone(), entries[0].clone()];
        controller.submit(request(
            2,
            Some(Arc::new(DirectoryIndex::from_entries(&sorted))),
            SelectionAction::Rebase,
        ));
        let Event::SelectionChanged { selection, .. } =
            tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("committed selection")
        };
        assert_eq!(selection.bits(), [0b101]);
        drop(controller);
        drop(events);
        while tasks.join_next().await.is_some() {}
    }

    #[test]
    fn additive_streamed_range_keeps_pending_select_all_for_future_rows() {
        let mut state = State::default();
        apply(
            &mut state,
            request(1, None, SelectionAction::Bulk(SelectionOperation::All)),
        );
        let range = request(
            2,
            None,
            SelectionAction::Range {
                anchor: "a".into(),
                target: "b".into(),
                additive: true,
            },
        );
        let (partial, pending) = state
            .apply_with_stream(&range, Some(view(&["a", "b"])))
            .unwrap()
            .unwrap();
        assert_eq!(partial.len(), 2);
        assert!(pending.any());
        let complete = apply(
            &mut state,
            request(3, Some(view(&["b", "later", "a"])), SelectionAction::Rebase),
        );
        assert_eq!(complete.len(), 3);
    }
}

#[cfg(test)]
#[path = "selection_invert_tests.rs"]
mod invert_tests;
