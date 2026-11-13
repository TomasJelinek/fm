//! Retain one resolved source/route while its frontend establishes the canvas.
use super::*;

pub(super) struct PreparedOpening {
    pub file: midnight_plugin_host::CapturedFile,
    pub route: crate::routing::Route,
}

impl Worker {
    pub(super) fn invalidate_layout(&mut self) {
        self.layout.invalidate();
        self.pending_layout = None;
    }

    pub(super) fn prepare_opening(
        &mut self,
        path: &std::path::Path,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<PreparedOpening, String> {
        let route = crate::routing::choose(
            self.runtime.catalog(),
            self.images
                .iter()
                .map(|provider| crate::routing::ProcessCandidate {
                    id: &provider.id,
                    extensions: &provider.extensions,
                })
                .chain(self.web_providers.iter().map(|provider| {
                    crate::routing::ProcessCandidate {
                        id: &provider.id,
                        extensions: &provider.extensions,
                    }
                })),
            path,
            self.basic,
            &self.disabled_plugins,
            &self.associations,
        )
        .ok_or_else(|| {
            "No compatible viewer plugin is installed beside the executable in viewers/.".to_owned()
        })?;
        let route = match route {
            crate::routing::Route::Process { index } => {
                self.mode = if index >= self.images.len() {
                    VIEWER_MODE_HTML
                } else {
                    VIEWER_MODE_IMAGE
                };
                self.modes = vec![self.mode];
                self.binary = false;
                route
            }
            crate::routing::Route::Native {
                index,
                mut mode,
                detect_binary,
            } => {
                let plugin = &self.runtime.catalog()[index];
                if detect_binary {
                    mode = file
                        .preferred_basic_mode()
                        .map_err(|error| error.to_string())?;
                }
                if let Some(selected) = self
                    .selected_mode
                    .filter(|mode| plugin.modes.contains(mode))
                {
                    mode = selected;
                }
                self.mode = mode;
                self.modes = plugin.modes.clone();
                self.binary = mode == viewer::MODE_BINARY;
                self.length = file.length();
                crate::routing::Route::Native {
                    index,
                    mode,
                    detect_binary: false,
                }
            }
        };
        Ok(PreparedOpening { file, route })
    }

    pub(super) fn select_opening(
        &mut self,
        opening: PreparedOpening,
        output: &Mailbox,
    ) -> Result<(), String> {
        if let Some(selected) = self.layout.select(self.mode, self.positions.generation)? {
            self.pending_layout = Some(opening);
            output.put(ViewerOutput::LayoutSelected {
                token: selected.token,
                mode: selected.mode,
                content_generation: selected.content_generation,
            });
            self.status("Opening…".into(), output);
            Ok(())
        } else {
            self.start_opening(opening, output)
        }
    }

    pub(super) fn take_layout_ready(
        &mut self,
        token: u64,
        geometry: ViewerGeometry,
    ) -> Option<PreparedOpening> {
        if geometry.width == 0
            || geometry.height == 0
            || !geometry.scale.is_finite()
            || geometry.scale <= 0.0
        {
            return None;
        }
        self.layout.accept(token)?;
        let opening = self.pending_layout.take()?;
        // Image/PDF Fit uses the raw physical canvas; text and HTML retain the
        // existing independently bounded surface and effective device scale.
        self.image_size = [geometry.width, geometry.height];
        self.geometry = bounded_geometry(geometry);
        Some(opening)
    }

    pub(super) fn start_opening(
        &mut self,
        opening: PreparedOpening,
        output: &Mailbox,
    ) -> Result<(), String> {
        let PreparedOpening { file, route } = opening;
        self.session = match route {
            crate::routing::Route::Process { index } if index >= self.images.len() => {
                let provider = self
                    .web_providers
                    .get(index - self.images.len())
                    .ok_or("Selected HTML viewer is no longer available.")?
                    .clone();
                self.open_web(provider, file)?;
                None
            }
            crate::routing::Route::Process { index } => {
                self.image = Some(crate::image_view::ImageView::new(
                    crate::image_worker::ImageWorker::open(
                        self.images[index].clone(),
                        Arc::new(file),
                        self.wake.clone(),
                        self.image_startup.clone(),
                    )?,
                    self.image_ids.clone(),
                ));
                None
            }
            crate::routing::Route::Native { index, mode, .. } => Some(
                self.runtime
                    .open(
                        &self.runtime.catalog()[index].id,
                        file,
                        self.generation,
                        mode,
                    )
                    .map_err(|error| error.to_string())?,
            ),
        };
        self.apply_font();
        if let Some(session) = &self.session {
            session.notify_on_ready(self.wake.clone());
        }
        self.status("Opening…".into(), output);
        Ok(())
    }
}
