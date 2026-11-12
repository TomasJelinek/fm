//! Navigation controls carry opaque, generation-bound targets, never parsed display paths.
use super::*;

pub(crate) struct PathPart {
    pub(crate) identity: String,
    pub(crate) label: String,
    pub(crate) prefix: String,
    path: PathBuf,
    archive: Option<ArchiveLocation>,
    network: Option<NetworkAction>,
}

pub(crate) struct HistoryChoice {
    pub(crate) identity: String,
    pub(crate) path: PathBuf,
    pub(crate) display: Option<String>,
    archive: bool,
    network: bool,
    index: usize,
}

impl Presenter {
    pub(crate) fn navigate_as_other_panel(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.panes[pane.other().index()].network_pending.is_some() {
            let Some(action) = self.panes[pane.other().index()].pending_network_browse() else {
                return false;
            };
            return self.network_navigate(pane, action, send);
        }
        if let Some(network) = &self.panes[pane.other().index()].network {
            return self.network_navigate(
                pane,
                NetworkAction::Browse(network.address.clone()),
                send,
            );
        }
        if let Some(a) = &self.panes[pane.other().index()].archive {
            return self.archive_navigate(pane, ArchiveAction::List(a.location.clone()), send);
        }
        let path = self.panes[pane.other().index()].display_path().to_owned();
        self.navigate(pane, path, None, send)
    }

    pub(super) fn queue_clipboard_text(
        &mut self,
        text: String,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let request = self.desktop.request();
        if send(Command::CopyText { request, text }) {
            self.desktop.requests.insert(request);
            true
        } else {
            self.desktop
                .report("Clipboard queue busy", "Try copying again.".into());
            false
        }
    }

    pub(crate) fn copy_path_part(
        &mut self,
        pane: PaneId,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(part) = self
            .path_parts(pane)
            .into_iter()
            .find(|part| part.identity == token)
        else {
            return false;
        };
        self.queue_clipboard_text(part.prefix, send)
    }

    pub(crate) fn path_parts(&self, pane: PaneId) -> Vec<PathPart> {
        let p = &self.panes[pane.index()];
        if let Some(action) = &p.network_pending {
            let label = p.display_location_text();
            return vec![PathPart {
                identity: format!("network-pending-path:{}", p.generation),
                label: label.clone(),
                prefix: label,
                path: PathBuf::new(),
                archive: None,
                network: Some(action.clone()),
            }];
        }
        if let Some(network) = &p.network {
            return vec![PathPart {
                identity: format!("network-path:{}", p.generation),
                label: network.address.display.clone(),
                prefix: network.address.display.clone(),
                path: PathBuf::new(),
                archive: None,
                network: Some(NetworkAction::Browse(network.address.clone())),
            }];
        }
        if let Some(a) = &p.archive {
            let mut previous = String::new();
            return a
                .location
                .trail
                .iter()
                .enumerate()
                .map(|(index, (node, _))| {
                    let mut location = a.location.clone();
                    location.directory = *node;
                    location.trail.truncate(index + 1);
                    location.address.components.truncate(index);
                    let prefix = location.display_text();
                    let label = prefix.strip_prefix(&previous).unwrap_or(&prefix).to_owned();
                    previous = prefix.clone();
                    PathPart {
                        identity: format!(
                            "archive:{}:{}:{node}",
                            p.generation,
                            a.location.mount_id().unwrap_or(0)
                        ),
                        label,
                        prefix,
                        path: PathBuf::new(),
                        archive: Some(location),
                        network: None,
                    }
                })
                .collect();
        }
        let ancestors: Vec<_> = p
            .display_path()
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        let mut previous = String::new();
        ancestors
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, path)| {
                let prefix = path.to_string_lossy().into_owned();
                let label = prefix.strip_prefix(&previous).unwrap_or(&prefix).to_owned();
                previous = prefix.clone();
                PathPart {
                    identity: format!(
                        "path:{}:{}:{index}",
                        p.generation,
                        u8::from(p.pending.is_some())
                    ),
                    label,
                    prefix,
                    path: path.to_owned(),
                    archive: None,
                    network: None,
                }
            })
            .collect()
    }

    pub(crate) fn navigate_path_part(
        &mut self,
        pane: PaneId,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(part) = self
            .path_parts(pane)
            .into_iter()
            .find(|part| part.identity == token)
        else {
            return false;
        };
        if let Some(action) = part.network {
            return self.network_navigate(pane, action, send);
        }
        if let Some(location) = part.archive {
            return self.archive_navigate(pane, ArchiveAction::List(location), send);
        }
        self.navigate(pane, part.path, None, send)
    }

    /// Cached shape for lazy history rows; path/display strings stay unallocated.
    pub(crate) fn history_model_state(&self, pane: PaneId) -> (u8, usize, usize, Option<usize>) {
        let p = &self.panes[pane.index()];
        if let Some(network) = &p.network {
            (
                1,
                network.history.as_ptr() as usize,
                network.history.len(),
                Some(network.history_index),
            )
        } else if let Some(archive) = &p.archive {
            (
                2,
                archive.history.as_ptr() as usize,
                archive.history.len(),
                Some(archive.history_index),
            )
        } else {
            (
                0,
                p.history.as_ptr() as usize,
                p.history.len(),
                p.history_index,
            )
        }
    }

    pub(crate) fn history_entry(
        &self,
        pane: PaneId,
        forward: bool,
        row: usize,
    ) -> Option<HistoryChoice> {
        let p = &self.panes[pane.index()];
        let (_, _, length, current) = self.history_model_state(pane);
        let current = current.filter(|index| *index < length)?;
        let index = if forward {
            current.checked_add(row)?.checked_add(1)?
        } else {
            current.checked_sub(row)?.checked_sub(1)?
        };
        if let Some(network) = &p.network {
            Some(HistoryChoice {
                identity: format!("network-history:{}:{index}", p.generation),
                path: network.origin.clone(),
                display: Some(network.history.get(index)?.display.clone()),
                archive: false,
                network: true,
                index,
            })
        } else if let Some(archive) = &p.archive {
            Some(HistoryChoice {
                identity: format!(
                    "archive-history:{}:{}:{index}",
                    p.generation,
                    archive.location.mount_id().unwrap_or(0)
                ),
                path: archive.origin.clone(),
                display: Some(archive.history.get(index)?.display_text()),
                archive: true,
                network: false,
                index,
            })
        } else {
            Some(HistoryChoice {
                identity: format!("history:{}:{}:{index}", p.generation, u8::from(forward)),
                path: p.history.get(index)?.clone(),
                display: None,
                archive: false,
                network: false,
                index,
            })
        }
    }

    #[cfg(test)]
    pub(crate) fn history_entries(&self, pane: PaneId, forward: bool) -> Vec<HistoryChoice> {
        (0..)
            .map_while(|row| self.history_entry(pane, forward, row))
            .collect()
    }

    pub(crate) fn choose_history_entry(
        &mut self,
        pane: PaneId,
        forward: bool,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let (_, _, _, current) = self.history_model_state(pane);
        let index = token
            .rsplit(':')
            .next()
            .and_then(|index| index.parse::<usize>().ok());
        let row = current.zip(index).and_then(|(current, index)| {
            if forward {
                index.checked_sub(current)?.checked_sub(1)
            } else {
                current.checked_sub(index)?.checked_sub(1)
            }
        });
        let Some(choice) = row
            .and_then(|row| self.history_entry(pane, forward, row))
            .filter(|choice| choice.identity == token)
        else {
            return false;
        };
        if choice.network {
            let Some(network) = &self.panes[pane.index()].network else {
                return false;
            };
            return self.network_history(
                pane,
                choice.index as isize - network.history_index as isize,
                send,
            );
        }
        if choice.archive {
            let Some(a) = &self.panes[pane.index()].archive else {
                return false;
            };
            return self.archive_history(
                pane,
                choice.index as isize - a.history_index as isize,
                send,
            );
        }
        self.navigate(pane, choice.path, Some(choice.index), send)
    }

    pub(crate) fn select_tab_number(
        &mut self,
        pane: PaneId,
        number: i32,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(index) = number
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < 9)
        else {
            return false;
        };
        let Some(tab) = self.tabs(pane).get(index).cloned() else {
            return false;
        };
        self.select_tab(pane, tab.id, send)
    }
}
