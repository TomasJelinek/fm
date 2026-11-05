//! Navigation controls carry opaque, generation-bound targets, never parsed display paths.
use super::*;

pub(crate) struct PathPart {
    pub(crate) identity: String,
    pub(crate) label: String,
    pub(crate) prefix: String,
    path: PathBuf,
}

pub(crate) struct HistoryChoice {
    pub(crate) identity: String,
    pub(crate) path: PathBuf,
    index: usize,
}

impl Presenter {
    pub(crate) fn path_parts(&self, pane: PaneId) -> Vec<PathPart> {
        let p = &self.panes[pane.index()];
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
        self.navigate(pane, part.path, None, send)
    }

    pub(crate) fn history_entries(&self, pane: PaneId, forward: bool) -> Vec<HistoryChoice> {
        let p = &self.panes[pane.index()];
        let Some(current) = p.history_index else {
            return Vec::new();
        };
        let mut indexes: Vec<_> = if forward {
            (current.saturating_add(1)..p.history.len()).collect()
        } else {
            (0..current).rev().collect()
        };
        indexes
            .drain(..)
            .map(|index| HistoryChoice {
                identity: format!("history:{}:{}:{index}", p.generation, u8::from(forward)),
                path: p.history[index].clone(),
                index,
            })
            .collect()
    }

    pub(crate) fn choose_history_entry(
        &mut self,
        pane: PaneId,
        forward: bool,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some(choice) = self
            .history_entries(pane, forward)
            .into_iter()
            .find(|choice| choice.identity == token)
        else {
            return false;
        };
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
