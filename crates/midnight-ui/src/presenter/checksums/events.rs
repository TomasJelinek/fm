use super::*;

impl Presenter {
    pub(in crate::presenter) fn apply_checksum(&mut self, event: ChecksumEvent) {
        let Some(s) = self
            .checksums
            .session_mut_if(event.job, |s| s.generation == event.generation)
        else {
            self.retired.push(Box::new(event));
            return;
        };
        match event.update {
            ChecksumUpdate::Queued | ChecksumUpdate::Started => {}
            ChecksumUpdate::Verifying(algorithm) => {
                if let Ok(algorithms) = ChecksumAlgorithms::new(&[algorithm]) {
                    s.algorithms = algorithms;
                }
            }
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
            .session_mut_if(event.job, |s| s.generation == event.generation)
        {
            s.running = false;
            s.stopping = false;
            s.error = crate::error_presentation::describe(&event.error);
        }
        self.retired.push(Box::new(event));
    }

    pub(in crate::presenter) fn apply_checksum_manifest(&mut self, event: ChecksumManifestEvent) {
        let Some(s) = self.checksums.session_mut_if(event.job, |s| {
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
                let last = s
                    .save
                    .take()
                    .map(|save| (save.algorithm, save.destination, save.acknowledged));
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
                                // As after transfers and file operations, an adopted
                                // native tree reconciles its loaded branches; a flat
                                // re-read would leave tree mode.
                                if self.panes[pane.index()]
                                    .queue_native_tree_refresh(&mut self.retired)
                                {
                                    continue;
                                }
                                self.queue_operation_refresh(pane, parent, None);
                            }
                        }
                    }
                    Err(error)
                        if error.kind == std::io::ErrorKind::AlreadyExists && last.is_some() =>
                    {
                        // Ask instead of failing: the user may want the file replaced.
                        let (algorithm, destination, acknowledged) = last.unwrap();
                        let name = destination
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        s.save_message.clear();
                        s.save_prompt = Some(SavePrompt {
                            algorithm,
                            destination,
                            acknowledged,
                            overwrite: true,
                            message: format!(
                                "{} already exists. Replace it?",
                                crate::shown_text(&name)
                            ),
                        });
                        self.retired.push(Box::new(error));
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
