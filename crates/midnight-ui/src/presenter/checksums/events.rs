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
            ChecksumUpdate::Planned(files) => {
                s.plan(&files);
                self.retired.push(Box::new(files));
            }
            ChecksumUpdate::Batch(batch) => {
                s.stream(&batch);
                self.retired.push(Box::new(batch));
            }
            ChecksumUpdate::Progress(progress) => s.progress = progress,
            ChecksumUpdate::Finished(run) => {
                s.running = false;
                s.stopping = false;
                s.progress = run.summary.progress.clone();
                self.retired.push(Box::new(s.finish_selection()));
                self.retired.push(Box::new(s.take_streamed()));
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
                let last = s.save.take();
                s.save_message.clear();
                match result {
                    Ok(receipt) => {
                        s.error.clear();
                        // The window reports where it went and what it replaced.
                        let parent = receipt.path.parent().map(Path::to_owned);
                        s.saved = Some(receipt);
                        if let Some(parent) = parent {
                            for pane in [PaneId::Left, PaneId::Right] {
                                // As after transfers and file operations, an adopted
                                // native tree reconciles its loaded branches; a flat
                                // re-read would leave tree mode.
                                if self.panes[pane.index()]
                                    .queue_native_tree_refresh(&mut self.retired)
                                {
                                    continue;
                                }
                                self.queue_operation_refresh(pane, &parent, None, false);
                            }
                        }
                    }
                    Err(error) if error.kind == std::io::ErrorKind::AlreadyExists => {
                        // Ask instead of failing: the user may want the file replaced.
                        // The question keeps this run's exact acknowledgement.
                        if let Some(save) = last {
                            let name = save
                                .destination
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            s.save_prompt = Some(SavePrompt {
                                generation: event.generation,
                                algorithm: save.algorithm,
                                destination: save.destination,
                                acknowledged: save.acknowledged,
                                overwrite: true,
                                message: format!(
                                    "{} already exists. Replace it?",
                                    crate::shown_text(&name)
                                ),
                            });
                        }
                        self.retired.push(Box::new(error));
                    }
                    Err(error) => {
                        s.error = crate::error_presentation::describe(&error);
                        self.retired.push(Box::new(error));
                    }
                }
            }
        }
    }
}
