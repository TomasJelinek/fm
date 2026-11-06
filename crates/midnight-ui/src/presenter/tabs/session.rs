//! Persist bounded native locations; restoring a session never revives rows,
//! selections, history, scans, or callback identities from an earlier process.
use super::*;

impl Presenter {
    /// Enable startup session persistence. Explicit paths override only their
    /// corresponding saved panels. Call immediately after construction.
    pub fn set_tab_restore_overrides(&mut self, overrides: [bool; 2]) {
        if self.preferences.loaded {
            return;
        }
        self.tabs.capture_initial = true;
        for (edited, explicit) in self.tabs.edited.iter_mut().zip(overrides) {
            *edited |= explicit;
        }
        let mut values = self.preferences.values.clone();
        values.tabs = self.current_tab_sessions();
        // Initial loading gates the save, including the application's exit drain.
        self.update_preferences(values, &|_| false);
    }

    fn current_tab_sessions(&self) -> [PaneTabPreferences; 2] {
        [
            self.tab_session(PaneId::Left),
            self.tab_session(PaneId::Right),
        ]
    }

    fn tab_session(&self, pane: PaneId) -> PaneTabPreferences {
        let slot = pane.index();
        let tabs = &self.tabs.panes[slot];
        PaneTabPreferences {
            active: tabs.active,
            paths: tabs
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| {
                    if index == tabs.active {
                        self.panes[slot].display_path().to_owned()
                    } else {
                        tab.parked
                            .as_ref()
                            .expect("inactive tab owns its location")
                            .path
                            .clone()
                    }
                })
                .collect(),
        }
    }

    pub(super) fn tab_session_changed(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.tabs.edited[pane.index()] = true;
        self.tabs.restore[pane.index()] = None;
        let mut values = self.preferences.values.clone();
        for side in [PaneId::Left, PaneId::Right] {
            if self.tabs.restore[side.index()].is_none() {
                values.tabs[side.index()] = self.tab_session(side);
            }
        }
        self.update_preferences(values, send);
    }

    pub(crate) fn tab_navigation_accepted(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        let slot = pane.index();
        if self.tabs.restore[slot].is_some()
            && self.panes[slot].display_path() == self.panes[slot].path
        {
            // An initial listing or watch refresh may enter the queue while
            // restoration is waiting for capacity. It does not override it.
            return;
        }
        let current = self.tab_session(pane);
        let stored = &self.preferences.values.tabs[slot];
        // Startup listing and watch refreshes are not local location edits.
        // An accepted new location, including reverting a provisional one,
        // must still win if the initial preferences read arrives later.
        let changed = if stored.paths.is_empty() {
            self.panes[slot].display_path() != self.panes[slot].path
        } else {
            &current != stored
        };
        if changed {
            self.tab_session_changed(pane, send);
        }
    }

    pub(crate) fn prepare_loaded_tab_sessions(&mut self, loaded: &mut UserPreferences) {
        let tracked = self.tabs.capture_initial
            || self.tabs.edited.iter().any(|edited| *edited)
            || loaded.tabs.iter().any(|tabs| !tabs.paths.is_empty());
        for pane in [PaneId::Left, PaneId::Right] {
            let slot = pane.index();
            if self.tabs.edited[slot] {
                loaded.tabs[slot] = self.tab_session(pane);
            } else if !loaded.tabs[slot].paths.is_empty() {
                self.tabs.restore[slot] = Some(loaded.tabs[slot].clone());
            } else if tracked {
                loaded.tabs[slot] = self.tab_session(pane);
            }
        }
    }

    pub(crate) fn flush_tab_sessions(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(saved) = self.tabs.restore[pane.index()].clone() else {
                continue;
            };
            self.restore_tab_session(pane, saved, send);
        }
        if self
            .preferences
            .values
            .tabs
            .iter()
            .all(|tabs| tabs.paths.is_empty())
        {
            return;
        }
        let mut values = self.preferences.values.clone();
        for pane in [PaneId::Left, PaneId::Right] {
            if self.tabs.restore[pane.index()].is_none() {
                values.tabs[pane.index()] = self.tab_session(pane);
            }
        }
        // A failed navigation restores the previous displayed location; persist
        // that correction without waiting for another explicit tab action.
        if values.tabs != self.preferences.values.tabs {
            self.update_preferences(values, send);
        }
    }

    fn restore_tab_session(
        &mut self,
        pane: PaneId,
        saved: PaneTabPreferences,
        send: &dyn Fn(Command) -> bool,
    ) {
        let slot = pane.index();
        if let Err(message) = saved.validate() {
            self.desktop
                .report("Could not restore tabs", message.into());
            self.tabs.restore[slot] = None;
            return;
        }
        let Some(path) = saved.paths.get(saved.active) else {
            self.tabs.restore[slot] = None;
            return;
        };
        let Some(next_id) = self.tabs.next_id.checked_add(saved.paths.len() as u64) else {
            return;
        };
        let Some(navigation) = self.panes[slot].generation.checked_add(1).map(NavigationId) else {
            return;
        };
        let sort = self.panes[slot].sort;
        if !send(Command::Navigate {
            pane,
            navigation,
            path: path.clone(),
            sort,
        }) {
            return;
        }
        let location = TabLocation::new(path.clone(), sort);
        let tabs = saved
            .paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| Tab {
                id: TabId(self.tabs.next_id + index as u64),
                parked: (index != saved.active).then(|| TabLocation::new(path, sort)),
            })
            .collect();
        let previous_active = self.tabs.panes[slot].active;
        self.activate_tab(pane, previous_active, location, navigation, send);
        let previous = mem::replace(
            &mut self.tabs.panes[slot],
            PaneTabs {
                tabs,
                active: saved.active,
            },
        );
        self.retired.push(Box::new(previous));
        self.tabs.next_id = next_id;
        self.tabs.restore[slot] = None;
    }
}
