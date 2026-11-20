//! Navigation controls carry opaque, generation-bound targets, never parsed display paths.
use super::*;

pub(crate) struct PathPart {
    pub(crate) identity: String,
    pub(crate) label: String,
    pub(crate) prefix: String,
    /// The archive's own name in an archive: a shortened path keeps it whole.
    pub(crate) keep_whole: bool,
    path: PathBuf,
    archive: Option<ArchiveLocation>,
    network: Option<NetworkAction>,
}

/// One part per folder of a native path, the root first. Each label is the part of
/// its prefix after the previous one, so that the labels join into the path.
fn native_parts(path: &Path, identity: &str) -> Vec<PathPart> {
    let ancestors: Vec<_> = path
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
                identity: format!("{identity}:{index}"),
                label,
                prefix,
                keep_whole: false,
                path: path.to_owned(),
                archive: None,
                network: None,
            }
        })
        .collect()
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
        // A native part copies its exact path; its lossy label is display text.
        // Archive and network parts copy their provider display text.
        if part.archive.is_none() && part.network.is_none() {
            let Some(text) = part.path.to_str() else {
                self.desktop.report(
                    "Cannot copy path",
                    "This native path cannot be represented exactly as clipboard text.".into(),
                );
                return false;
            };
            return self.queue_clipboard_text(text.to_owned(), send);
        }
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
                keep_whole: false,
                path: PathBuf::new(),
                archive: None,
                network: Some(action.clone()),
            }];
        }
        // A network address is opaque and names only its own node: no folders to show.
        if let Some(network) = &p.network {
            return vec![PathPart {
                identity: format!("network-path:{}", p.generation),
                label: network.address.display.clone(),
                prefix: network.address.display.clone(),
                keep_whole: false,
                path: PathBuf::new(),
                archive: None,
                network: Some(NetworkAction::Browse(network.address.clone())),
            }];
        }
        if let Some(a) = &p.archive {
            // The native folders that hold the archive come first, as on a native
            // path, and leave the archive; the archive's name is a part of its own.
            let container: PathBuf = a
                .location
                .outermost()
                .address
                .container
                .components()
                .collect();
            let mut parts = container.parent().map_or_else(Vec::new, |folder| {
                native_parts(folder, &format!("archive-path:{}", p.generation))
            });
            let mut previous = parts
                .last()
                .map_or_else(String::new, |part| part.prefix.clone());
            let locations = a.path_locations();
            // The root of the archive that holds this folder.
            let own_root = locations.len().saturating_sub(a.location.trail.len());
            // Archives inside archives: the enclosing archives' folders come first.
            parts.extend(locations.into_iter().enumerate().map(|(index, location)| {
                let prefix = location.display_text();
                let label = prefix.strip_prefix(&previous).unwrap_or(&prefix).to_owned();
                previous = prefix.clone();
                PathPart {
                    identity: format!(
                        "archive:{}:{}:{index}:{}",
                        p.generation,
                        location.mount_id().unwrap_or(0),
                        location.directory
                    ),
                    label,
                    prefix,
                    keep_whole: index == own_root,
                    path: PathBuf::new(),
                    archive: Some(location),
                    network: None,
                }
            }));
            return parts;
        }
        native_parts(
            p.display_path(),
            &format!("path:{}:{}", p.generation, u8::from(p.pending.is_some())),
        )
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
        // A native folder that holds the shown archive leaves it, with the cursor on
        // the folder or archive on the way back, as an ancestor of a native path does.
        let container = self.panes[pane.index()]
            .archive
            .as_ref()
            .map(|a| -> PathBuf {
                a.location
                    .outermost()
                    .address
                    .container
                    .components()
                    .collect()
            });
        if !self.navigate(pane, part.path.clone(), None, send) {
            return false;
        }
        if let Some(container) = container {
            let child = container
                .ancestors()
                .find(|child| child.parent() == Some(part.path.as_path()))
                .map(Path::to_owned);
            let p = &mut self.panes[pane.index()];
            if let Some(request) = &mut p.pending {
                request.wanted_cursor = child;
            }
            p.parent_cursor = false;
        }
        true
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
