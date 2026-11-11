//! Presentation of virtual devices/shares; none of these rows authorizes file operations.
use super::*;

#[derive(Clone)]
pub(super) struct NetworkPane {
    pub(super) address: NetworkAddress,
    pub(super) parent: Option<NetworkAddress>,
    pub(super) origin: PathBuf,
    pub(super) history: Vec<NetworkAddress>,
    pub(super) history_index: usize,
    requested_history: Option<usize>,
    wanted_cursor: Option<NetworkAddress>,
}
#[derive(Clone)]
pub(super) struct NetworkReturn {
    pub(super) root: PathBuf,
    pub(super) context: NetworkPane,
}
impl NetworkPane {
    pub(super) fn restored(address: NetworkAddress, origin: PathBuf) -> Self {
        Self {
            address: address.clone(),
            parent: None,
            origin,
            history: vec![address],
            history_index: 0,
            requested_history: None,
            wanted_cursor: None,
        }
    }
}

impl Pane {
    /// Sorting/refreshing/copying a pending location must never repeat share activation.
    pub(super) fn pending_network_browse(&self) -> Option<NetworkAction> {
        self.network_pending
            .as_ref()
            .and_then(|action| match action {
                NetworkAction::Activate(_) | NetworkAction::Parse(_) => None,
                action => Some(action.clone()),
            })
    }

    /// The pending caption is never parsed into a filesystem path or provider identity.
    pub(super) fn pending_network_text(&self) -> Option<&str> {
        self.network_pending.as_ref().map(|action| match action {
            NetworkAction::OpenRoot => NETWORK_NEIGHBORHOOD_LABEL,
            NetworkAction::Parse(text) => text.as_str(),
            NetworkAction::Browse(address)
            | NetworkAction::Activate(address)
            | NetworkAction::Parent(address)
            | NetworkAction::Refresh(address) => address.display.as_str(),
        })
    }

    pub(super) fn network_hides_rows(&self) -> bool {
        self.network_pending.as_ref().is_some_and(|action| {
            !matches!(action, NetworkAction::Browse(address) | NetworkAction::Refresh(address)
                if self.network.as_ref().is_some_and(|network|
                    network.address.provider == address.provider
                        && network.address.opaque == address.opaque))
        })
    }
}

impl Presenter {
    pub(crate) fn network_navigate(
        &mut self,
        pane: PaneId,
        action: NetworkAction,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &mut self.panes[pane.index()];
        let same = p.network.as_ref().is_some_and(|network| matches!(&action,
            NetworkAction::Browse(address) | NetworkAction::Refresh(address)
                if address.provider == network.address.provider && address.opaque == network.address.opaque));
        let activated = match &action {
            NetworkAction::Activate(address) => Some(address.clone()),
            _ => None,
        };
        let Some(navigation) = p.generation.checked_add(1).map(NavigationId) else {
            return false;
        };
        if !send(Command::NetworkNavigate {
            pane,
            navigation,
            action: action.clone(),
            sort: p.sort,
        }) {
            p.set_error(
                "Network navigation queue is busy. Try again.",
                &mut self.retired,
            );
            return false;
        }
        self.clear_tree(pane, Some(navigation));
        let p = &mut self.panes[pane.index()];
        p.generation = navigation.0;
        p.network_request = true;
        self.retired
            .push(Box::new(p.network_pending.replace(action)));
        p.archive_request = false;
        p.loading = true;
        p.set_error("", &mut self.retired);
        self.retired.push(Box::new(p.pending.take()));
        self.retired.push(Box::new(mem::take(&mut p.streaming)));
        self.retired
            .push(Box::new(mem::take(&mut p.streaming_index)));
        if let Some(network) = &mut p.network {
            network.requested_history = None;
            // A native share return restores the activated row, even if opening
            // it did not first move the keyboard cursor.
            network.wanted_cursor = activated;
        }
        if let Some(scan) = p.size_scan.take() {
            send(Command::CancelFolderSizes { pane, scan });
        }
        self.search_navigation_accepted(pane, !same, send);
        drag_drop::invalidate(&self.drag_drop.targets);
        true
    }

    pub(super) fn network_open(
        &mut self,
        pane: PaneId,
        row: usize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let p = &self.panes[pane.index()];
        if row == 0 && p.has_parent() {
            return self.network_up(pane, send);
        }
        let Some(address) = row
            .checked_sub(p.parent_offset())
            .and_then(|row| p.display_entry(row))
            .and_then(|entry| entry.network_node.as_ref())
            .map(|node| node.address.clone())
        else {
            return false;
        };
        self.network_navigate(pane, NetworkAction::Activate(address), send)
    }
    pub(super) fn network_up(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        if self.panes[pane.index()].network_hides_rows() {
            let p = &self.panes[pane.index()];
            if let Some(network) = &p.network {
                return self.network_navigate(
                    pane,
                    NetworkAction::Browse(network.address.clone()),
                    send,
                );
            }
            return self.navigate(pane, p.path.clone(), None, send);
        }
        let Some(network) = &self.panes[pane.index()].network else {
            return false;
        };
        if let Some(parent) = &network.parent {
            let address = network.address.clone();
            let result = self.network_navigate(pane, NetworkAction::Browse(parent.clone()), send);
            if result {
                self.panes[pane.index()]
                    .network
                    .as_mut()
                    .expect("existing network pane")
                    .wanted_cursor = Some(address);
            }
            result
        } else {
            self.navigate(pane, network.origin.clone(), None, send)
        }
    }
    pub(super) fn network_history(
        &mut self,
        pane: PaneId,
        delta: isize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(network) = &self.panes[pane.index()].network else {
            return false;
        };
        let Some(index) = network
            .history_index
            .checked_add_signed(delta)
            .filter(|i| *i < network.history.len())
        else {
            return false;
        };
        let action = NetworkAction::Browse(network.history[index].clone());
        if !self.network_navigate(pane, action, send) {
            return false;
        }
        self.panes[pane.index()]
            .network
            .as_mut()
            .expect("existing network pane")
            .requested_history = Some(index);
        true
    }
    pub(super) fn apply_network_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::NetworkAvailability(available) => {
                if self.network_available != available {
                    self.network_available = available;
                    self.sync_navigation_choices();
                }
            }
            Event::NetworkUpdated {
                pane,
                navigation,
                update,
            } => {
                let p = &mut self.panes[pane.index()];
                if p.generation != navigation.0 || !p.network_request {
                    self.retired.push(Box::new(update));
                    return None;
                }
                match *update {
                    NetworkUpdate::Failed(error) => {
                        p.loading = false;
                        p.network_request = false;
                        self.retired.push(Box::new(p.network_pending.take()));
                        p.set_error(error.to_string(), &mut self.retired);
                        // Tab activation uses the common navigation transition,
                        // but its pending native placeholder grants no authority.
                        self.retired.push(Box::new(p.pending.take()));
                        p.retire_stream(&mut self.retired);
                        if let Some(network) = &mut p.network {
                            network.requested_history = None;
                            network.wanted_cursor = None;
                        }
                    }
                    NetworkUpdate::Native(_) => {
                        self.retired.push(Box::new(update));
                    }
                    NetworkUpdate::Listing {
                        listing,
                        done,
                        error,
                    } => {
                        let focused = p
                            .cursor
                            .and_then(|id| p.entry_by_id(id))
                            .and_then(|entry| entry.network_node.as_ref())
                            .map(|node| node.address.clone());
                        self.retired.push(Box::new(p.network_pending.take()));
                        let same = p.network.as_ref().is_some_and(|network| {
                            network.address.opaque == listing.address.opaque
                                && network.address.provider == listing.address.provider
                        });
                        if let Some(network) = &mut p.network {
                            if let Some(index) = network.requested_history.take() {
                                network.history_index = index;
                            } else if !same {
                                self.retired.push(Box::new(
                                    network.history.split_off(network.history_index + 1),
                                ));
                                network.history.push(listing.address.clone());
                                if network.history.len() > 128 {
                                    self.retired.push(Box::new(network.history.remove(0)));
                                }
                                network.history_index = network.history.len() - 1;
                            }
                            network.address = listing.address;
                            network.parent = listing.parent;
                        } else {
                            let mut network = p
                                .network_return
                                .as_ref()
                                .filter(|previous| {
                                    previous.context.address.provider == listing.address.provider
                                        && previous.context.address.opaque == listing.address.opaque
                                })
                                .map(|previous| previous.context.clone())
                                .unwrap_or_else(|| {
                                    NetworkPane::restored(listing.address.clone(), p.path.clone())
                                });
                            network.address = listing.address;
                            network.parent = listing.parent;
                            network.requested_history = None;
                            p.network = Some(network);
                        }
                        self.retired.push(Box::new(p.network_return.take()));
                        self.retired.push(Box::new(p.archive.take()));
                        self.retired.push(Box::new(p.pending.take()));
                        self.retired
                            .push(Box::new(mem::replace(&mut p.entries, listing.entries)));
                        self.retired
                            .push(Box::new(mem::replace(&mut p.index, listing.index)));
                        self.retired.push(Box::new(mem::take(&mut p.selected)));
                        self.retired.push(Box::new(mem::take(&mut p.streaming)));
                        self.retired.push(Box::new(mem::take(&mut p.metadata)));
                        p.folder_column_samples = SizeColumnSamples::default();
                        self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
                        p.selection_preview.clear();
                        p.selection_rebase_pending = false;
                        p.successful_generation = Some(navigation);
                        p.committed_sort = p.sort;
                        let wanted = p
                            .network
                            .as_ref()
                            .and_then(|network| network.wanted_cursor.clone())
                            .or_else(|| focused.filter(|_| same));
                        p.cursor = wanted.and_then(|focused| {
                            p.entries
                                .iter()
                                .find(|entry| {
                                    entry
                                        .network_node
                                        .as_ref()
                                        .is_some_and(|node| node.address.opaque == focused.opaque)
                                })
                                .map(|entry| entry.id)
                        });
                        if done {
                            p.network
                                .as_mut()
                                .expect("published network pane")
                                .wanted_cursor = None;
                        }
                        p.parent_cursor = p.cursor.is_none();
                        p.loading = !done;
                        p.network_request = !done;
                        p.set_error(
                            error.map_or_else(String::new, |error| error.to_string()),
                            &mut self.retired,
                        );
                    }
                }
            }
            other => return Some(other),
        }
        None
    }
}
