//! Presentation accepts only the exact requested selection and newest backend total.
use super::*;

#[derive(Default)]
pub(super) struct State {
    next: u64,
    sent: Option<(SelectionSizeId, NavigationId, SelectionRevision)>,
    version: u64,
    summary: Option<SelectedFolderSizes>,
}

impl Presenter {
    pub(super) fn flush_selection_sizes(&mut self, send: &dyn Fn(Command) -> bool) {
        self.flush_archive_size_observers(send);
        for pane in [PaneId::Left, PaneId::Right] {
            let p = &mut self.panes[pane.index()];
            if p.tree_view().is_some()
                || p.loading
                || !p.native_capabilities()
                || p.selection_revision != p.selection_applied
                || !p.selection_preview.is_empty()
                || p.selection_pending_bulk
                || p.selection_deferred.any()
                || p.selection_rebase_pending
            {
                continue;
            }
            let navigation = NavigationId(p.generation);
            if p.selection_sizes.sent.is_some_and(|(_, nav, revision)| {
                nav == navigation && revision == p.selection_applied
            }) || (p.selected.folder_count() == 0 && p.selection_sizes.sent.is_none())
            {
                continue;
            }
            let Some(next) = p.selection_sizes.next.checked_add(1) else {
                continue;
            };
            let id = SelectionSizeId(next);
            if send(Command::SummarizeSelectedFolders(SelectionSizeRequest {
                id,
                pane,
                navigation,
                source_navigation: p.displayed_generation(),
                revision: p.selection_applied,
                location: p.display_path().to_owned(),
                selection: p.selected.clone(),
            })) {
                p.selection_sizes.next = next;
                p.selection_sizes.sent = Some((id, navigation, p.selection_applied));
                p.selection_sizes.version = 0;
                p.selection_sizes.summary = None;
            }
        }
    }

    pub(super) fn apply_selection_sizes(&mut self, event: Event) -> Option<Event> {
        let Event::SelectedFolderSizes {
            request,
            pane,
            navigation,
            selection_revision,
            version,
            summary,
        } = event
        else {
            return Some(event);
        };
        let p = &mut self.panes[pane.index()];
        if p.generation == navigation.0
            && p.selection_applied == selection_revision
            && p.selection_sizes.sent == Some((request, navigation, selection_revision))
            && version > p.selection_sizes.version
        {
            p.selection_sizes.version = version;
            p.selection_sizes.summary = Some(summary);
        }
        None
    }
}

impl Pane {
    pub(super) fn selected_folder_sizes(&self) -> Option<SelectedFolderSizes> {
        if self.tree_view().is_some() {
            return self.tree_size_summary();
        }
        if let Some(archive) = &self.archive {
            return archive
                .sizes
                .summary(self.successful_generation?, archive.selection_revision);
        }
        self.selection_sizes
            .sent
            .filter(|(_, navigation, revision)| {
                navigation.0 == self.generation && *revision == self.selection_applied
            })
            .and(self.selection_sizes.summary)
    }
}
