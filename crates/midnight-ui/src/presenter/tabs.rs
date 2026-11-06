//! Each pane owns a bounded collection of locations and one live backend slot.
//! Parked tabs keep small navigation state, never directory snapshots or scans.
use super::*;
mod session;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabSummary {
    pub id: TabId,
    pub title: String,
    pub path: PathBuf,
    pub active: bool,
}

struct TabLocation {
    path: PathBuf,
    history: Vec<PathBuf>,
    history_index: Option<usize>,
    navigation_history: Option<usize>,
    sort: SortSpec,
    cursor_path: Option<PathBuf>,
    parent_cursor: bool,
    volume_paths: HashMap<PathBuf, PathBuf>,
}

impl TabLocation {
    fn new(path: PathBuf, sort: SortSpec) -> Self {
        Self {
            parent_cursor: parent_path(&path).is_some(),
            path,
            history: Vec::new(),
            history_index: None,
            navigation_history: None,
            sort,
            cursor_path: None,
            volume_paths: HashMap::new(),
        }
    }

    fn park(pane: &mut Pane) -> Self {
        let path = pane.display_path().to_owned();
        let cursor_path = pane
            .cursor
            .and_then(|id| pane.entry_by_id(id))
            .map(|entry| entry.path.clone())
            .or_else(|| pane.pending.as_ref().and_then(|p| p.wanted_cursor.clone()));
        let navigation_history = pane
            .pending
            .as_ref()
            .and_then(|request| request.history)
            .or(pane.history_index)
            .filter(|index| pane.history.get(*index) == Some(&path));
        Self {
            path,
            history: mem::take(&mut pane.history),
            history_index: pane.history_index,
            navigation_history,
            sort: pane.sort,
            cursor_path,
            parent_cursor: pane.parent_cursor,
            volume_paths: mem::take(&mut pane.volume_paths),
        }
    }
}

struct Tab {
    id: TabId,
    /// The active tab's state lives in Presenter::panes instead.
    parked: Option<TabLocation>,
}

struct PaneTabs {
    tabs: Vec<Tab>,
    active: usize,
}

pub(super) struct TabsState {
    panes: [PaneTabs; 2],
    next_id: u64,
    edited: [bool; 2],
    restore: [Option<PaneTabPreferences>; 2],
    capture_initial: bool,
}

impl TabsState {
    pub(super) fn new() -> Self {
        let pane = |id| PaneTabs {
            tabs: vec![Tab {
                id: TabId(id),
                parked: None,
            }],
            active: 0,
        };
        Self {
            panes: [pane(1), pane(2)],
            next_id: 3,
            edited: [false; 2],
            restore: [None, None],
            capture_initial: false,
        }
    }

    pub(super) fn retire(&mut self, retired: &mut Vec<DeferredDrop>) {
        for pane in &mut self.panes {
            retired.push(Box::new(mem::take(&mut pane.tabs)));
        }
    }
}

impl Presenter {
    pub fn tabs(&self, pane: PaneId) -> Vec<TabSummary> {
        let tabs = &self.tabs.panes[pane.index()];
        tabs.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = index == tabs.active;
                let path = if active {
                    self.panes[pane.index()].display_path()
                } else {
                    tab.parked
                        .as_ref()
                        .expect("inactive tab owns its location")
                        .path
                        .as_path()
                };
                let title = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into();
                TabSummary {
                    id: tab.id,
                    title,
                    path: path.to_owned(),
                    active,
                }
            })
            .collect()
    }

    /// Duplicate the displayed native location and activate it after enqueue succeeds.
    pub fn new_tab(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let slot = pane.index();
        if self.tabs.panes[slot].tabs.len() == MAX_TABS_PER_PANE {
            self.desktop.report(
                "Tab limit reached",
                "Each panel supports up to 32 tabs.".into(),
            );
            return false;
        }
        let Some(next_id) = self.tabs.next_id.checked_add(1) else {
            return false;
        };
        let p = &self.panes[slot];
        let location = TabLocation::new(p.display_path().to_owned(), p.sort);
        let Some(navigation) = p.generation.checked_add(1).map(NavigationId) else {
            return false;
        };
        if !send(Command::Navigate {
            pane,
            navigation,
            path: location.path.clone(),
            sort: location.sort,
        }) {
            self.panes[slot].error = "Navigation queue is busy. Try again.".into();
            return false;
        }
        let target = self.tabs.panes[slot].tabs.len();
        self.tabs.panes[slot].tabs.push(Tab {
            id: TabId(self.tabs.next_id),
            parked: None,
        });
        self.tabs.next_id = next_id;
        self.activate_tab(pane, target, location, navigation, send);
        self.tab_session_changed(pane, send);
        true
    }

    /// Requery a parked location using a generation newer than every tab in this pane.
    pub fn select_tab(&mut self, pane: PaneId, id: TabId, send: &dyn Fn(Command) -> bool) -> bool {
        if !self.select_tab_impl(pane, id, send) {
            return false;
        }
        self.tab_session_changed(pane, send);
        true
    }

    fn select_tab_impl(&mut self, pane: PaneId, id: TabId, send: &dyn Fn(Command) -> bool) -> bool {
        let slot = pane.index();
        let tabs = &self.tabs.panes[slot];
        let Some(target) = tabs.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        if target == tabs.active {
            return false;
        }
        let location = tabs.tabs[target]
            .parked
            .as_ref()
            .expect("inactive tab owns its location");
        let Some(navigation) = self.panes[slot].generation.checked_add(1).map(NavigationId) else {
            return false;
        };
        if !send(Command::Navigate {
            pane,
            navigation,
            path: location.path.clone(),
            sort: location.sort,
        }) {
            self.panes[slot].error = "Navigation queue is busy. Try again.".into();
            return false;
        }
        let location = self.tabs.panes[slot].tabs[target]
            .parked
            .take()
            .expect("validated inactive tab");
        self.activate_tab(pane, target, location, navigation, send);
        true
    }

    fn activate_tab(
        &mut self,
        pane: PaneId,
        target: usize,
        mut location: TabLocation,
        navigation: NavigationId,
        send: &dyn Fn(Command) -> bool,
    ) {
        let slot = pane.index();
        let mut incoming = Pane::new(location.path.clone());
        incoming.history = mem::take(&mut location.history);
        incoming.history_index = location.history_index;
        incoming.sort = location.sort;
        incoming.committed_sort = location.sort;
        incoming.parent_cursor = location.parent_cursor;
        incoming.volume_paths = mem::take(&mut location.volume_paths);
        incoming.generation = self.panes[slot].generation;
        incoming.size_scan_generation = self.panes[slot].size_scan_generation;
        let mut outgoing = mem::replace(&mut self.panes[slot], incoming);
        if let Some(scan) = outgoing.size_scan.take() {
            send(Command::CancelFolderSizes { pane, scan });
        }
        let tabs = &mut self.tabs.panes[slot];
        tabs.tabs[tabs.active].parked = Some(TabLocation::park(&mut outgoing));
        tabs.active = target;
        self.retired.push(Box::new(outgoing));
        self.navigation_accepted(
            pane,
            navigation,
            location.path,
            location.navigation_history,
            send,
        );
        if let Some(request) = &mut self.panes[slot].pending {
            request.wanted_cursor = location.cursor_path;
        }
    }

    /// Close an inactive tab immediately; an active close commits only when its
    /// replacement navigation was accepted. The final tab always remains open.
    pub fn close_tab(&mut self, pane: PaneId, id: TabId, send: &dyn Fn(Command) -> bool) -> bool {
        let slot = pane.index();
        let tabs = &self.tabs.panes[slot];
        if tabs.tabs.len() == 1 {
            return false;
        }
        let Some(target) = tabs.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        if target == tabs.active {
            let replacement = if target == 0 { 1 } else { target - 1 };
            let replacement_id = tabs.tabs[replacement].id;
            if !self.select_tab_impl(pane, replacement_id, send) {
                return false;
            }
        }
        let tabs = &mut self.tabs.panes[slot];
        let removed = tabs.tabs.remove(target);
        if tabs.active > target {
            tabs.active -= 1;
        }
        self.retired.push(Box::new(removed));
        self.tab_session_changed(pane, send);
        true
    }

    pub fn close_active_tab(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) -> bool {
        let tabs = &self.tabs.panes[pane.index()];
        self.close_tab(pane, tabs.tabs[tabs.active].id, send)
    }

    pub fn cycle_tab(
        &mut self,
        pane: PaneId,
        delta: isize,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let tabs = &self.tabs.panes[pane.index()];
        let target = (tabs.active as isize + delta.rem_euclid(tabs.tabs.len() as isize))
            .rem_euclid(tabs.tabs.len() as isize) as usize;
        self.select_tab(pane, tabs.tabs[target].id, send)
    }
}
