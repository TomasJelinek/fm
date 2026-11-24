//! Native mutation authority for fed rows, independent of the hidden directory.
use super::*;

pub(super) struct PendingMutation {
    id: OperationId,
    metadata: bool,
    paths: Vec<PathBuf>,
}

fn captured(entry: &Entry) -> CapturedPath {
    CapturedPath {
        path: entry.path.clone(),
        kind: entry.kind,
        size: entry.metadata.size,
    }
}

impl Presenter {
    fn feed_mutation_ready(&mut self, pane: PaneId) -> bool {
        let Some(feed) = self.feed(pane) else {
            return false;
        };
        if feed.mutation.is_some() || feed.deleting.is_some() {
            self.desktop.report(
                "File change pending",
                "Wait for the previous change to these search results to finish.".into(),
            );
            return false;
        }
        true
    }

    pub(in crate::presenter) fn feed_rename(
        &mut self,
        pane: PaneId,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing
            || self.comparison_refuses("Rename")
            || !self.feed_mutation_ready(pane)
        {
            return;
        }
        let Some(target) = self.feed(pane).and_then(FeedPane::focused).map(captured) else {
            return;
        };
        let Some(serial) = self.finds.feed_serial.checked_add(1) else {
            return;
        };
        let source = match CapturedPathRename::new(pane, serial, target) {
            Ok(source) => Arc::new(source),
            Err(error) => {
                self.desktop.report("Rename", error.into());
                return;
            }
        };
        let path = source.target().path.clone();
        let Some(directory) = path.parent().map(std::path::Path::to_owned) else {
            return;
        };
        let Some(id) = self.next_operation_id(send) else {
            return;
        };
        self.retired.push(Box::new(source.clone()));
        if send(Command::PrepareCapturedRename(PrepareCapturedRename {
            id,
            source,
            source_directory: directory.clone(),
        })) {
            self.finds.feed_serial = serial;
            self.register_preparing_operation(id, MutationKind::Rename, directory);
            if let Some(feed) = self.feed_mut(pane) {
                feed.mutation = Some(PendingMutation {
                    id,
                    metadata: false,
                    paths: vec![path],
                });
            }
        } else {
            self.desktop
                .report("Rename queue busy", "Try the action again.".into());
        }
    }

    pub(in crate::presenter) fn feed_metadata(
        &mut self,
        pane: PaneId,
        case: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing
            || self.comparison_refuses("Attributes, times and case")
            || !self.feed_mutation_ready(pane)
        {
            return;
        }
        let Some(feed) = self.feed(pane) else { return };
        if feed.selected_count > MAX_METADATA_TARGETS {
            self.desktop.report(
                "Metadata selection too large",
                "Select at most 256 search results to edit metadata.".into(),
            );
            return;
        }
        let targets = feed.targets().into_iter().map(captured).collect();
        let Some(serial) = self.finds.feed_serial.checked_add(1) else {
            return;
        };
        let sources = match CapturedPathsMetadata::new(pane, serial, targets) {
            Ok(sources) => Arc::new(sources),
            Err(error) => {
                self.desktop
                    .report("Attributes, times and case", error.into());
                return;
            }
        };
        let paths: Vec<_> = sources
            .targets()
            .iter()
            .map(|target| target.path.clone())
            .collect();
        let Some(directory) = paths
            .first()
            .and_then(|path| path.parent())
            .map(std::path::Path::to_owned)
        else {
            return;
        };
        let Some(id) = self.next_operation_id(send) else {
            return;
        };
        self.retired.push(Box::new(sources.clone()));
        if send(Command::PrepareCapturedMetadata(PrepareCapturedMetadata {
            id,
            sources,
            source_directory: directory.clone(),
        })) {
            self.finds.feed_serial = serial;
            self.register_preparing_operation(id, MutationKind::EditMetadata, directory);
            self.retire_metadata();
            self.metadata = crate::presenter::metadata::MetadataState {
                id: Some(id),
                start_case: case,
                busy: true,
                ..Default::default()
            };
            if let Some(feed) = self.feed_mut(pane) {
                feed.mutation = Some(PendingMutation {
                    id,
                    metadata: true,
                    paths,
                });
            }
        } else {
            self.desktop
                .report("Metadata queue busy", "Try the action again.".into());
        }
    }

    /// Return stale roots after a confirmed change. The feed's shared row-removal
    /// path also removes descendants, preserving unaffected rows and their marks.
    pub(in crate::presenter) fn completed_feed_mutation_paths(
        &mut self,
        pane: PaneId,
    ) -> Option<Vec<PathBuf>> {
        let pending = self.feed(pane)?.mutation.as_ref()?;
        let (id, metadata) = (pending.id, pending.metadata);
        let Some(progress) = self
            .operations
            .progress
            .iter()
            .find(|progress| progress.id == id)
        else {
            let pending = self.feed_mut(pane)?.mutation.take();
            self.retired.push(Box::new(pending));
            return None;
        };
        if !progress.state.is_terminal() {
            return None;
        }
        let changed = if metadata {
            self.operations
                .metadata_results
                .get(&id)
                .map_or(progress.completed > 0, |result| result.changed > 0)
        } else {
            progress.renamed_to.is_some()
        };
        let pending = self.feed_mut(pane)?.mutation.take()?;
        if changed {
            Some(pending.paths)
        } else {
            self.retired.push(Box::new(pending));
            None
        }
    }
}

#[cfg(test)]
#[path = "find_feed_mutations_tests.rs"]
mod tests;
