use super::*;
use crate::adapter::Record;
use std::{collections::HashSet, sync::atomic::AtomicU64};

struct PageRequest {
    page: u64,
    maximum: u32,
    deadline: Instant,
}

impl<A: Adapter> NetworkFilesystem<A> {
    fn directory(&self, node: &NodeRecord) -> Result<Arc<Directory>, Failure> {
        let mut state = lock(&self.state);
        let now = Instant::now();
        let retired: Vec<_> = state
            .directories
            .extract_if(|_, directory| {
                Arc::strong_count(directory) == 1
                    && lock(&directory.cache).as_ref().is_none_or(|cache| {
                        !cache.fresh(now, directory.epoch.load(Ordering::Acquire))
                    })
            })
            .collect();
        if !retired.is_empty() {
            state.directories.shrink_to_fit();
        }
        drop(state);
        drop(retired);
        let mut state = lock(&self.state);
        let result = if let Some(directory) = state.directories.get(&node.key) {
            Ok(directory.clone())
        } else if state.directories.len() >= 4096 {
            Err(limit())
        } else {
            let key = node.key.clone();
            let size = accounting::directory_bytes(&key)?;
            let directory = Arc::new(Directory {
                epoch: AtomicU64::new(0),
                cache: Mutex::new(None),
                _charge: self.budget.reserve(size, 0, 0)?,
            });
            state.directories.try_reserve(1).map_err(|_| limit())?;
            state.directories.insert(key, directory.clone());
            Ok(directory)
        };
        drop(state);
        result
    }
    pub(super) fn list_begin(
        &self,
        mount: u64,
        node: u64,
        maximum: u32,
        call: &CallContext<'_>,
    ) -> Result<Reply, Failure> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let (mount, node) = self.node(mount, node)?;
        if !(1..=MAX_BATCH).contains(&maximum) {
            return Err(failure(ErrorCode::Invalid));
        }
        if node
            .location
            .trail
            .last()
            .is_some_and(|record| record.target.kind == NodeKind::Share)
        {
            return Err(failure(ErrorCode::MountRequired));
        }
        let directory = self.directory(&node)?;
        let epoch = directory.epoch.load(Ordering::Acquire);
        let cached = lock(&directory.cache)
            .as_ref()
            .filter(|cache| cache.fresh(Instant::now(), epoch))
            .map(|cache| cache.snapshot.clone());
        let permit = if cached.is_none() {
            Some(DiscoveryPermit::acquire()?)
        } else {
            None
        };
        let live = permit.is_some();
        let source = if let Some(snapshot) = cached {
            Source::Cached { snapshot, index: 0 }
        } else {
            Source::Finished
        };
        let id = allocate_handle()?;
        let cursor = Arc::new(Cursor {
            mount,
            node,
            directory,
            epoch,
            closed: AtomicBool::new(false),
            busy: AtomicBool::new(true),
            data: Mutex::new(CursorData {
                source,
                next: 0,
                done: false,
                pending: None,
                seen: HashSet::new(),
                accumulated: Vec::new(),
                charge: if live {
                    Some(self.budget.reserve(0, 0, 0)?)
                } else {
                    None
                },
            }),
            _charge: self
                .budget
                .reserve(accounting::cursor_bytes::<A>()?, 0, 0)?,
        });
        {
            let mut state = lock(&self.state);
            if cursor.stale() {
                return Err(stale());
            }
            if state.cursors.len() >= 64 {
                return Err(limit());
            }
            state.cursors.try_reserve(1).map_err(|_| limit())?;
            state.cursors.insert(id, cursor.clone());
        }
        let busy = Busy(cursor.clone());
        if let Some(permit) = permit {
            let cancelled = || call.is_cancelled() || cursor.stale();
            let context = Context {
                cancelled: &cancelled,
                deadline,
            };
            let result = context.check().and_then(|()| {
                A::begin(
                    &cursor.node.location.trail.last().ok_or_else(stale)?.target,
                    &context,
                )
            });
            match result {
                Ok(os) => {
                    lock(&cursor.data).source = Source::Live(Live {
                        cursor: os,
                        _permit: permit,
                    })
                }
                Err(error) => {
                    let invalid = cursor.stale();
                    self.retire(id, &cursor);
                    drop(permit);
                    return Err(if invalid { stale() } else { error });
                }
            }
        }
        self.page(
            id,
            cursor,
            PageRequest {
                page: 0,
                maximum,
                deadline,
            },
            call,
            busy,
        )
    }
    pub(super) fn list_next(
        &self,
        id: u64,
        page: u64,
        maximum: u32,
        call: &CallContext<'_>,
    ) -> Result<Reply, Failure> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let cursor = lock(&self.state)
            .cursors
            .get(&id)
            .cloned()
            .ok_or_else(stale)?;
        if cursor
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(failure(ErrorCode::Busy));
        }
        let busy = Busy(cursor.clone());
        self.page(
            id,
            cursor,
            PageRequest {
                page,
                maximum,
                deadline,
            },
            call,
            busy,
        )
    }
    fn page(
        &self,
        id: u64,
        cursor: Arc<Cursor<A>>,
        request: PageRequest,
        call: &CallContext<'_>,
        busy: Busy<A>,
    ) -> Result<Reply, Failure> {
        let PageRequest {
            page,
            maximum,
            deadline,
        } = request;
        let mut data = lock(&cursor.data);
        if cursor.stale() || data.next != page || data.done || !(1..=MAX_BATCH).contains(&maximum) {
            return Err(stale());
        }
        let cancelled = || call.is_cancelled() || cursor.stale();
        let context = Context {
            cancelled: &cancelled,
            deadline,
        };
        let mut transaction = NodeTransaction::new(cursor.mount.clone());
        let mut entries = Vec::new();
        let mut bytes = 64usize;
        let result = (|| {
            entries
                .try_reserve_exact(maximum as usize)
                .map_err(|_| limit())?;
            while entries.len() < maximum as usize && !data.done {
                context.check()?;
                let record = if let Some(pending) = data.pending.take() {
                    Some(pending.record)
                } else {
                    Self::read_record(&mut data, &context)?
                };
                let Some(record) = record else {
                    data.done = true;
                    drop(std::mem::replace(&mut data.source, Source::Finished));
                    break;
                };
                let mut location = cursor.node.location.clone();
                location.trail.push(record.clone());
                let address = location.encode()?;
                let size = 256usize
                    .checked_add(address.bytes.len())
                    .and_then(|size| size.checked_add(record.target.key.len()))
                    .and_then(|size| size.checked_add(record.display.len()))
                    .ok_or_else(limit)?;
                if bytes
                    .checked_add(size)
                    .is_none_or(|bytes| bytes > call.config().reply_bytes as usize)
                {
                    if entries.is_empty() {
                        return Err(limit());
                    }
                    data.pending = Some(Pending::new(record, &self.budget)?);
                    break;
                }
                let node = self.bind(&cursor.mount, location, &mut transaction)?;
                entries.push(node.model()?);
                bytes += size;
            }
            context.check()?;
            Ok(())
        })();
        if let Err(error) = result {
            let invalid = cursor.stale();
            drop(data);
            self.retire(id, &cursor);
            drop(busy);
            return Err(if invalid { stale() } else { error });
        }
        let done = data.done;
        if done {
            // The snapshot owns the retained-record charge after completion. Do not
            // leave a second identity set alive after its cache charge can retire.
            data.seen.clear();
            data.seen.shrink_to_fit();
        }
        let snapshot = if done && !data.accumulated.is_empty() {
            Some(Arc::new(Snapshot {
                records: std::mem::take(&mut data.accumulated),
                _charge: data.charge.take().ok_or_else(limit)?,
            }))
        } else if done && data.charge.is_some() && data.seen.is_empty() {
            // Only a live empty enumeration seeds an empty cache. Cache replay
            // never refreshes the original expiry deadline.
            Some(Arc::new(Snapshot {
                records: Vec::new(),
                _charge: data.charge.take().ok_or_else(limit)?,
            }))
        } else {
            None
        };
        drop(data);
        let response = Response::Page {
            cursor: id,
            page,
            done,
            entries,
        };
        let state = self.state.clone();
        Ok(Reply::owned(response, move |publish| {
            let (retired, old_cache, result) = {
                let mut state = lock(&state);
                if !publish || cursor.stale() {
                    cursor.closed.store(true, Ordering::Release);
                    let retired = state.cursors.remove(&id);
                    state.cursors.shrink_to_fit();
                    (retired, None, if publish { Err(stale()) } else { Ok(()) })
                } else {
                    transaction.commit();
                    lock(&cursor.data).next = page.checked_add(1).ok_or_else(limit)?;
                    let old_cache = if let Some(snapshot) = snapshot {
                        lock(&cursor.directory.cache).replace(Cached {
                            snapshot,
                            created: Instant::now(),
                            epoch: cursor.epoch,
                        })
                    } else {
                        None
                    };
                    (None, old_cache, Ok(()))
                }
            };
            drop(old_cache);
            drop(retired);
            drop(busy);
            result
        }))
    }
    fn read_record(
        data: &mut CursorData<A>,
        context: &Context<'_>,
    ) -> Result<Option<Record>, Failure> {
        let record = match &mut data.source {
            Source::Live(live) => A::next(&mut live.cursor, context)?,
            Source::Cached { snapshot, index } => {
                let record = snapshot.records.get(*index).cloned();
                if record.is_some() {
                    *index += 1;
                }
                return Ok(record);
            }
            Source::Finished => return Ok(None),
        };
        if let Some(record) = &record {
            A::validate(&record.target)?;
            if data.seen.contains(&record.target) {
                return Err(failure(ErrorCode::SourceChanged));
            }
            if data.accumulated.len() >= 4096 {
                return Err(limit());
            }
            let bytes = record
                .target
                .key
                .len()
                .checked_mul(2)
                .and_then(|size| size.checked_add(record.display.len()))
                .and_then(|size| size.checked_add(256))
                .ok_or_else(limit)?;
            data.charge.as_mut().ok_or_else(limit)?.grow(bytes)?;
            data.seen.try_reserve(1).map_err(|_| limit())?;
            data.accumulated.try_reserve(1).map_err(|_| limit())?;
            data.seen.insert(record.target.clone());
            data.accumulated.push(record.clone());
        }
        Ok(record)
    }
    fn retire(&self, id: u64, cursor: &Cursor<A>) {
        let retired = {
            let mut state = lock(&self.state);
            cursor.closed.store(true, Ordering::Release);
            let retired = state.cursors.remove(&id);
            state.cursors.shrink_to_fit();
            retired
        };
        drop(retired);
    }
    pub(super) fn close_list(&self, id: u64) -> Result<Reply, Failure> {
        let retired = {
            let mut state = lock(&self.state);
            let cursor = state.cursors.remove(&id).ok_or_else(stale)?;
            cursor.closed.store(true, Ordering::Release);
            state.cursors.shrink_to_fit();
            cursor
        };
        drop(retired);
        Reply::stateless(Response::Unit)
    }
    pub(super) fn refresh(&self, mount: u64, node: u64) -> Result<Reply, Failure> {
        let (_, node) = self.node(mount, node)?;
        let directory = self.directory(&node)?;
        let (retired, cache) = {
            let mut state = lock(&self.state);
            directory
                .epoch
                .try_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                    epoch.checked_add(1)
                })
                .map_err(|_| limit())?;
            let cache = lock(&directory.cache).take();
            let retired: Vec<_> = state
                .cursors
                .extract_if(|_, cursor| {
                    if Arc::ptr_eq(&cursor.directory, &directory) {
                        cursor.closed.store(true, Ordering::Release);
                        true
                    } else {
                        false
                    }
                })
                .map(|(_, cursor)| cursor)
                .collect();
            if !retired.is_empty() {
                state.cursors.shrink_to_fit();
            }
            (retired, cache)
        };
        drop(retired);
        drop(cache);
        Reply::stateless(Response::Unit)
    }
}

#[cfg(test)]
mod tests;
