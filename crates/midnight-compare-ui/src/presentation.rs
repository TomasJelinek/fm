//! Bounded presentation and consumer guards for the engine's typed comparison result.
use super::{ImageMode, OpenWindow, fonts, show_names};
use midnight_compare_engine::{ExternalCompareChoiceOutcome, IdentityVerdict, Output};

impl OpenWindow {
    pub(super) fn present_output(&self, output: &Output) {
        if self.closed.get() {
            return;
        }
        // A producer-admitted terminal Started outcome outranks passive chooser traffic.
        // Unknown remains visible until an explicit new choice is accepted.
        let terminal = matches!(output, Output::ExternalStarted { .. });
        if !terminal && self.external_outcome.get() == Some(ExternalCompareChoiceOutcome::Unknown) {
            return;
        }
        let window = &self.window;
        match output {
            Output::ExternalStarted { .. } => self.close(),
            Output::Frame {
                frame,
                names,
                appearance_revision,
            } => {
                if !self.appearance.accepts(*appearance_revision) {
                    return;
                }
                let generation = self.comparison_generation.get();
                if frame.comparison_generation == 0
                    || frame.comparison_generation < generation
                    || (self.comparison_requested.get()
                        && frame.comparison_generation <= generation)
                    || (window.get_choosing_files() && !self.comparison_requested.get())
                {
                    return;
                }
                // Session admission must also fence unread and later old publications, including
                // an old first frame whose generation this UI has never observed. An increment
                // from the last rendered generation cannot establish that boundary on its own.
                self.comparison_generation.set(frame.comparison_generation);
                self.comparison_requested.set(false);
                window.set_identity_state(match frame.identity {
                    IdentityVerdict::Pending => 0,
                    IdentityVerdict::IdenticalFiles => 1,
                    IdentityVerdict::IdenticalText => 2,
                    IdentityVerdict::Different => 3,
                });
                fonts::present(self, frame.fonts.as_ref());
                if window.get_choosing_files() {
                    window.set_choosing_files(false);
                    window.set_choice_busy(false);
                    window.invoke_focus_content();
                }
                window.set_frame_revision(frame.geometry_revision as i32);
                window.set_rendered_mode(match frame.mode {
                    Some(ImageMode::SideBySide) => 0,
                    Some(ImageMode::Swipe) => 1,
                    Some(ImageMode::OnionSkin) => 2,
                    None => -1,
                });
                show_names(window, names);
                window.set_image_mode(frame.image);
                window.set_position(frame.position as f32);
                window.set_status(midnight_core::visible_name(&frame.status).as_ref().into());
                window.set_accessible_text(frame.accessible_text.as_str().into());
                if let Some(layers) = &frame.layers {
                    self.surfaces.borrow_mut().present(
                        window,
                        layers,
                        frame.mode == Some(ImageMode::SideBySide),
                    );
                } else {
                    let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                        &frame.pixels,
                        frame.width,
                        frame.height,
                    );
                    window.set_surface(slint::Image::from_rgba8_premultiplied(pixels));
                }
            }
            Output::Status(message) => {
                window.set_identity_state(-1);
                window.set_status(midnight_core::visible_name(message).as_ref().into());
            }
            Output::ChooseFiles(choice) => self.show_file_choice(choice),
        }
    }
}
