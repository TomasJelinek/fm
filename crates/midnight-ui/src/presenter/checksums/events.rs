use super::*;

impl Presenter {
    pub(in crate::presenter) fn apply_checksum(&mut self, event: ChecksumEvent) {
        let Some(s) = self
            .checksums
            .session_mut(event.job)
            .filter(|s| s.generation == event.generation)
        else {
            self.retired.push(Box::new(event));
            return;
        };
        match event.update {
            ChecksumUpdate::Queued | ChecksumUpdate::Started => {}
            ChecksumUpdate::Batch(batch) => {
                let remaining = ChecksumLimits::default()
                    .max_rows
                    .saturating_sub(s.streamed.len());
                s.streamed.extend(batch.iter().take(remaining).cloned());
                self.retired.push(Box::new(batch));
            }
            ChecksumUpdate::Progress(progress) => s.progress = progress,
            ChecksumUpdate::Finished(run) => {
                s.running = false;
                s.stopping = false;
                s.progress = run.summary.progress.clone();
                self.retired.push(Box::new(mem::take(&mut s.streamed)));
                self.retired.push(Box::new(s.run.replace(run)));
            }
        }
    }

    pub(in crate::presenter) fn reject_checksum(&mut self, event: ChecksumRejected) {
        if let Some(s) = self
            .checksums
            .session_mut(event.job)
            .filter(|s| s.generation == event.generation)
        {
            s.running = false;
            s.stopping = false;
            s.error = crate::error_presentation::describe(&event.error);
        }
        self.retired.push(Box::new(event));
    }

    pub(in crate::presenter) fn apply_checksum_manifest(&mut self, event: ChecksumManifestEvent) {
        let Some(s) = self.checksums.session_mut(event.job).filter(|s| {
            s.generation == event.generation
                && s.save.as_ref().is_some_and(|save| save.id == event.save_id)
        }) else {
            self.retired.push(Box::new(event));
            return;
        };
        match event.update {
            ChecksumManifestUpdate::Queued | ChecksumManifestUpdate::Saving => {
                s.save.as_mut().unwrap().accepted = true;
                s.save_message = "Saving and independently verifying manifest…".into();
            }
            ChecksumManifestUpdate::Rejected(error) => {
                // A duplicate-ID rejection must not finish an earlier accepted save.
                if !s.save.as_ref().unwrap().accepted {
                    s.save = None;
                }
                s.error = crate::error_presentation::describe(&error);
                self.retired.push(Box::new(error));
            }
            ChecksumManifestUpdate::Finished(result) => {
                s.save = None;
                match result {
                    Ok(receipt) => {
                        s.error.clear();
                        s.save_message = format!(
                            "Saved and verified {} bytes\n{}\n{} included · {} excluded",
                            self.number_locale.format_u64(receipt.verified_bytes),
                            receipt.path.display(),
                            self.number_locale.format_u64(receipt.included),
                            self.number_locale.format_u64(receipt.excluded)
                        );
                        if let Some(parent) = receipt.path.parent() {
                            for pane in [PaneId::Left, PaneId::Right] {
                                if self.panes[pane.index()].native_capabilities()
                                    && self.panes[pane.index()].display_path() == parent
                                    && !self
                                        .operations
                                        .refresh
                                        .iter()
                                        .any(|(target, _, _)| *target == pane)
                                {
                                    self.operations
                                        .refresh
                                        .push((pane, parent.to_owned(), None));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        s.save_message.clear();
                        s.error = crate::error_presentation::describe(&error);
                        self.retired.push(Box::new(error));
                    }
                }
            }
        }
    }
}
