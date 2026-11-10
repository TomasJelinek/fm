//! Provider policy. OS work and OS resource destruction never run under the catalog lock.
use crate::{
    adapter::{Adapter, Context, NodeKind},
    address::Location,
    api::*,
};
use midnight_filesystem_plugin_sdk::{CallContext, Filesystem, Reply, allocate_handle};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
mod accounting;
mod listing;
mod state;
use state::*;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct NetworkFilesystem<A: Adapter> {
    state: Arc<Mutex<State<A>>>,
    budget: Arc<Budget>,
}
impl<A: Adapter> Filesystem for NetworkFilesystem<A> {
    fn create(_: abi::Config) -> Result<Self, Failure> {
        Ok(Self {
            state: Arc::new(Mutex::new(State::default())),
            budget: shared_budget(),
        })
    }
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        match request {
            Request::Volumes => Reply::stateless(Response::Volumes(vec![Volume {
                id: "network".into(),
                display: A::root().display,
                root: Location {
                    trail: vec![A::root()],
                }
                .encode()?,
            }])),
            Request::Resolve { address } => self.resolve(Location::decode(&address)?),
            Request::ResolveText { text } => {
                let root = A::root();
                let record = A::manual(&text)?;
                let trail = if record.target == root.target {
                    vec![root]
                } else {
                    vec![root, record]
                };
                self.resolve(Location { trail })
            }
            Request::Stat { mount, node } => {
                let (mount, node) = self.node(mount, node)?;
                Ok(self.guarded(mount, Response::Node(node.model()?)))
            }
            Request::Parent { mount, node } => {
                let (mount, node) = self.node(mount, node)?;
                let Some(parent) = node.location.parent() else {
                    return Ok(self.guarded(mount, Response::Parent(None)));
                };
                let mut transaction = NodeTransaction::new(mount.clone());
                let parent = self.bind(&mount, parent, &mut transaction)?;
                let response = Response::Parent(Some(parent.model()?));
                let state = self.state.clone();
                Ok(Reply::owned(response, move |publish| {
                    let _state = lock(&state);
                    if publish && !mount.closed.load(Ordering::Acquire) {
                        transaction.commit();
                        Ok(())
                    } else if publish {
                        Err(stale())
                    } else {
                        Ok(())
                    }
                }))
            }
            Request::Activate { mount, node } => self.activate(mount, node, context),
            Request::ListBegin {
                mount,
                directory,
                max_entries,
            } => self.list_begin(mount, directory, max_entries, context),
            Request::ListNext {
                cursor,
                expected_page,
                max_entries,
            } => self.list_next(cursor, expected_page, max_entries, context),
            Request::CloseList { cursor } => self.close_list(cursor),
            Request::Refresh { mount, node } => self.refresh(mount, node),
            Request::CloseMount { mount } => self.close_mount(mount),
            _ => Err(failure(ErrorCode::Unsupported)),
        }
    }
}
impl<A: Adapter> NetworkFilesystem<A> {
    fn guarded(&self, mount: Arc<Mount>, response: Response) -> Reply {
        let state = self.state.clone();
        Reply::owned(response, move |publish| {
            let _state = lock(&state);
            if publish && mount.closed.load(Ordering::Acquire) {
                Err(stale())
            } else {
                Ok(())
            }
        })
    }
    fn validate(location: &Location) -> Result<(), Failure> {
        location.encode()?;
        if location
            .trail
            .first()
            .is_none_or(|first| first.target != A::root().target)
        {
            return Err(failure(ErrorCode::Invalid));
        }
        for record in &location.trail {
            A::validate(&record.target)?;
        }
        Ok(())
    }
    fn resolve(&self, location: Location) -> Result<Reply, Failure> {
        Self::validate(&location)?;
        let id = allocate_handle()?;
        let mount = Arc::new(Mount {
            closed: AtomicBool::new(false),
            published: AtomicBool::new(false),
            nodes: Mutex::default(),
            _charge: self.budget.reserve(512, 0, 1)?,
        });
        let mut transaction = NodeTransaction::new(mount.clone());
        let node = self.bind(&mount, location, &mut transaction)?;
        let response = Response::Resolved {
            mount: id,
            node: node.model()?,
        };
        {
            let mut state = lock(&self.state);
            state.mounts.try_reserve(1).map_err(|_| limit())?;
            state.mounts.insert(id, mount.clone());
        }
        let state = self.state.clone();
        Ok(Reply::owned(response, move |publish| {
            let retired = {
                let mut state = lock(&state);
                if publish && !mount.closed.load(Ordering::Acquire) {
                    transaction.commit();
                    mount.published.store(true, Ordering::Release);
                    None
                } else {
                    mount.closed.store(true, Ordering::Release);
                    let retired = state.mounts.remove(&id);
                    state.mounts.shrink_to_fit();
                    retired
                }
            };
            drop(retired);
            if publish && mount.closed.load(Ordering::Acquire) {
                Err(stale())
            } else {
                Ok(())
            }
        }))
    }
    fn mount(&self, id: u64) -> Result<Arc<Mount>, Failure> {
        lock(&self.state)
            .mounts
            .get(&id)
            .filter(|mount| {
                mount.published.load(Ordering::Acquire) && !mount.closed.load(Ordering::Acquire)
            })
            .cloned()
            .ok_or_else(stale)
    }
    fn node(&self, mount: u64, id: u64) -> Result<(Arc<Mount>, Arc<NodeRecord>), Failure> {
        let mount = self.mount(mount)?;
        let node = lock(&mount.nodes)
            .by_id
            .get(&id)
            .filter(|node| node.published.load(Ordering::Acquire))
            .cloned()
            .ok_or_else(stale)?;
        Ok((mount, node))
    }
    fn bind(
        &self,
        mount: &Arc<Mount>,
        location: Location,
        transaction: &mut NodeTransaction,
    ) -> Result<Arc<NodeRecord>, Failure> {
        Self::validate(&location)?;
        let key = identity(&location);
        let mut nodes = lock(&mount.nodes);
        if mount.closed.load(Ordering::Acquire) {
            return Err(stale());
        }
        if let Some(id) = nodes.by_key.get(&key) {
            let existing = nodes.by_id.get(id).ok_or_else(stale)?;
            if !existing.published.load(Ordering::Acquire) && !transaction.ids.contains(id) {
                return Err(failure(ErrorCode::Busy));
            }
            return Ok(existing.clone());
        }
        let node_key = key.clone();
        let size = accounting::node_bytes(&location, &key, &node_key)?;
        let charge = self.budget.reserve(size, 1, 0)?;
        nodes.by_id.try_reserve(1).map_err(|_| limit())?;
        nodes.by_key.try_reserve(1).map_err(|_| limit())?;
        transaction.ids.try_reserve(1).map_err(|_| limit())?;
        let id = allocate_handle()?;
        let node = Arc::new(NodeRecord {
            id,
            location,
            key: node_key,
            published: AtomicBool::new(false),
            _charge: charge,
        });
        nodes.by_key.insert(key, id);
        nodes.by_id.insert(id, node.clone());
        transaction.ids.push(id);
        Ok(node)
    }
    fn activate(&self, mount: u64, node: u64, call: &CallContext<'_>) -> Result<Reply, Failure> {
        let (mount, node) = self.node(mount, node)?;
        let cancelled = || call.is_cancelled() || mount.closed.load(Ordering::Acquire);
        let context = Context {
            cancelled: &cancelled,
            deadline: Instant::now() + REQUEST_TIMEOUT,
        };
        context.check()?;
        let _permit = DiscoveryPermit::acquire()?;
        let target = &node.location.trail.last().ok_or_else(stale)?.target;
        let activation = A::activate(target, &context)?;
        context.check()?;
        let activation = match activation {
            ActivationTarget::WithinProvider(address)
                if address.encoding == AddressEncoding::Opaque
                    && address.bytes == target.key
                    && target.kind != NodeKind::Share =>
            {
                ActivationTarget::WithinProvider(node.location.encode()?)
            }
            ActivationTarget::Native(address) if target.kind == NodeKind::Share => {
                let result = ActivationTarget::Native(address);
                encode_response(
                    &Response::Activated(result.clone()),
                    call.config().reply_bytes as usize,
                )
                .map_err(|_| failure(ErrorCode::Invalid))?;
                result
            }
            _ => return Err(failure(ErrorCode::Invalid)),
        };
        let state = self.state.clone();
        Ok(Reply::owned(
            Response::Activated(activation),
            move |publish| {
                let _state = lock(&state);
                if publish && mount.closed.load(Ordering::Acquire) {
                    Err(stale())
                } else {
                    Ok(())
                }
            },
        ))
    }
    fn close_mount(&self, id: u64) -> Result<Reply, Failure> {
        let (mount, retired) = {
            let mut state = lock(&self.state);
            let mount = state.mounts.remove(&id).ok_or_else(stale)?;
            mount.closed.store(true, Ordering::Release);
            let retired: Vec<_> = state
                .cursors
                .extract_if(|_, cursor| {
                    if Arc::ptr_eq(&cursor.mount, &mount) {
                        cursor.closed.store(true, Ordering::Release);
                        true
                    } else {
                        false
                    }
                })
                .map(|(_, cursor)| cursor)
                .collect();
            state.mounts.shrink_to_fit();
            if !retired.is_empty() {
                state.cursors.shrink_to_fit();
            }
            (mount, retired)
        };
        drop(retired);
        drop(mount);
        Reply::stateless(Response::Unit)
    }
}
