use super::*;
use std::sync::Mutex;

type Parent = dyn Fn(u64, &Cancellation) -> Result<Option<u64>, NetworkFailure> + Send + Sync;

/// Location `n` carries `n` in its opaque bytes; `parent` names its parent.
struct Levels {
    parent: Box<Parent>,
    /// Bytes added to every address, to reach the byte bound.
    padding: usize,
    asked: Mutex<Vec<u64>>,
}
impl std::fmt::Debug for Levels {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Levels")
    }
}
impl Levels {
    fn new(
        parent: impl Fn(u64, &Cancellation) -> Result<Option<u64>, NetworkFailure>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            parent: Box::new(parent),
            padding: 0,
            asked: Mutex::default(),
        }
    }
    fn address(&self, level: u64) -> NetworkAddress {
        let mut opaque = level.to_le_bytes().to_vec();
        opaque.resize(opaque.len() + self.padding, 0);
        NetworkAddress {
            provider: "network.test".into(),
            opaque: opaque.into(),
            display: format!("Level {level}"),
        }
    }
    fn chain(&self, level: u64, cancel: &Cancellation) -> Result<Vec<u64>, NetworkFailure> {
        let chain = ancestors(self, &self.address(level), cancel)?;
        Ok(chain.iter().map(level_of).collect())
    }
}
fn level_of(address: &NetworkAddress) -> u64 {
    u64::from_le_bytes(address.opaque[..8].try_into().unwrap())
}
fn offline() -> NetworkFailure {
    NetworkFailure::new(NetworkFailureKind::Offline, "offline")
}
impl NetworkNeighborhood for Levels {
    fn root(&self, _: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        Ok(self.address(0))
    }
    fn parse(&self, _: &str, _: &Cancellation) -> Result<NetworkAddress, NetworkFailure> {
        panic!("no location is entered as text")
    }
    fn parent(
        &self,
        address: &NetworkAddress,
        cancel: &Cancellation,
    ) -> Result<Option<NetworkAddress>, NetworkFailure> {
        let level = level_of(address);
        self.asked.lock().unwrap().push(level);
        Ok((self.parent)(level, cancel)?.map(|parent| self.address(parent)))
    }
    fn list(
        &self,
        address: &NetworkAddress,
        _: &Cancellation,
        emit: &mut dyn FnMut(NetworkNode) -> bool,
    ) -> Result<(), NetworkFailure> {
        for row in [100, 101] {
            let mut node = self.address(row);
            node.display = format!("{} row {row}", address.display);
            if !emit(NetworkNode {
                display: node.display.clone(),
                address: node,
            }) {
                break;
            }
        }
        Ok(())
    }
    fn activate(
        &self,
        address: &NetworkAddress,
        _: &Cancellation,
    ) -> Result<NetworkDestination, NetworkFailure> {
        Ok(NetworkDestination::Browse(address.clone()))
    }
    fn refresh(&self, _: &NetworkAddress, _: &Cancellation) -> Result<(), NetworkFailure> {
        Ok(())
    }
}

#[test]
fn every_publication_names_the_provider_ancestors_outermost_first() {
    // Level 3 is below 2, 1 and the root 0.
    let provider = Levels::new(|level, _| Ok(level.checked_sub(1)));
    for action in [
        NetworkAction::Browse(provider.address(3)),
        NetworkAction::Activate(provider.address(3)),
        NetworkAction::Refresh(provider.address(3)),
        NetworkAction::Parent(provider.address(4)),
    ] {
        let mut published = Vec::new();
        run(
            &provider,
            action.clone(),
            SortSpec::default(),
            None,
            &Cancellation::default(),
            &mut |update| {
                if let NetworkUpdate::Listing { listing, .. } = update {
                    published.push(listing);
                }
                true
            },
        )
        .unwrap();
        assert!(
            published.len() >= 2,
            "{action:?}: a partial and a final listing"
        );
        for listing in &published {
            assert_eq!(level_of(&listing.address), 3, "{action:?}");
            let chain: Vec<_> = listing
                .ancestors
                .iter()
                .map(|address| address.display.as_str())
                .collect();
            assert_eq!(chain, ["Level 0", "Level 1", "Level 2"], "{action:?}");
            assert_eq!(listing.parent().map(level_of), Some(2), "{action:?}");
        }
    }
    // A root has no ancestors, and nothing beyond it is asked for.
    provider.asked.lock().unwrap().clear();
    assert!(
        provider
            .chain(0, &Cancellation::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(*provider.asked.lock().unwrap(), [0]);
}

#[test]
fn only_a_failed_parent_fails_the_request_and_a_later_failure_keeps_the_nearer_chain() {
    let provider = Levels::new(|level, _| match level {
        3 => Ok(Some(2)),
        2 => Ok(Some(1)),
        _ => Err(offline()),
    });
    assert_eq!(provider.chain(3, &Cancellation::default()).unwrap(), [1, 2]);
    // Without a parent, Up has nowhere to go: the request fails as before.
    let error = provider.chain(1, &Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, NetworkFailureKind::Offline);
    let error = run(
        &provider,
        NetworkAction::Browse(provider.address(1)),
        SortSpec::default(),
        None,
        &Cancellation::default(),
        &mut |_| panic!("nothing is published without the parent"),
    )
    .unwrap_err();
    assert_eq!(error.kind, NetworkFailureKind::Offline);
    // A lookup failing because the request was canceled cancels the request.
    let provider = Levels::new(|level, cancel| {
        if level == 2 {
            cancel.cancel();
            Err(NetworkFailure::new(
                NetworkFailureKind::Cancelled,
                "canceled",
            ))
        } else {
            Ok(level.checked_sub(1))
        }
    });
    let error = provider.chain(3, &Cancellation::default()).unwrap_err();
    assert_eq!(error.kind, NetworkFailureKind::Cancelled);
    assert_eq!(*provider.asked.lock().unwrap(), [3, 2]);
}

#[test]
fn a_location_already_in_the_chain_ends_it() {
    // 1 and 2 name each other as parents.
    let provider = Levels::new(|level, _| Ok(Some(if level == 1 { 2 } else { level - 1 })));
    assert_eq!(provider.chain(3, &Cancellation::default()).unwrap(), [1, 2]);
    // A provider that names a location below the shown one as its ancestor.
    let provider = Levels::new(|level, _| Ok(Some(if level == 1 { 3 } else { level - 1 })));
    assert_eq!(provider.chain(3, &Cancellation::default()).unwrap(), [1, 2]);
}

#[test]
fn a_long_or_large_chain_keeps_the_nearest_ancestors_within_the_bounds() {
    // Every location has a parent: the chain stops at its length bound.
    let provider = Levels::new(|level, _| Ok(Some(level + 1)));
    let chain = provider.chain(0, &Cancellation::default()).unwrap();
    assert_eq!(chain.len(), MAX_ANCESTORS);
    assert_eq!(chain.first(), Some(&(MAX_ANCESTORS as u64)));
    assert_eq!(chain.last(), Some(&1), "the parent is the nearest");
    // Large addresses: the ancestors beyond the parent stop at the byte bound.
    let mut provider = Levels::new(|level, _| Ok(Some(level + 1)));
    provider.padding = 3 * 1024 * 1024;
    let chain = ancestors(&provider, &provider.address(0), &Cancellation::default()).unwrap();
    let size = |address: &NetworkAddress| address.opaque.len() + address.display.len();
    let beyond: usize = chain[..chain.len() - 1].iter().map(size).sum();
    assert!(beyond <= MAX_ANCESTOR_BYTES);
    let next = provider.address(chain.len() as u64 + 1);
    assert!(
        beyond + size(&next) > MAX_ANCESTOR_BYTES,
        "{} ancestors stopped before the byte bound",
        chain.len()
    );
    assert_eq!(chain.last().map(level_of), Some(1), "the parent is kept");
}
