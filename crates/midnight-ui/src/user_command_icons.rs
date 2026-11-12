//! Metadata-only command icon views and stale-result admission.
use super::*;

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    sent: Option<Vec<CommandIconTarget>>,
    visible: Vec<(String, u32, bool, bool)>,
    rejected: HashSet<String>,
}
impl State {
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.sent = None;
        self.rejected.clear();
    }
}

impl Presenter {
    /// Called by the existing thumbnail pulse after the shared visible budget is assigned.
    pub fn refresh_command_icons(
        &mut self,
        rows: &[ThumbnailTarget],
        send: &dyn Fn(Command) -> bool,
    ) {
        let targets: Vec<_> = rows
            .iter()
            .filter_map(|row| {
                let crate::ThumbnailContent::ExecutableIcon(path) = &row.content else {
                    return None;
                };
                Some(CommandIconTarget {
                    identity: row.identity.clone(),
                    executable: path.clone(),
                })
            })
            .collect();
        let mut state = self.command_toolbar.native.borrow_mut();
        let visible: Vec<_> = rows
            .iter()
            .map(|row| (row.identity.clone(), row.edge, row.selected, row.hovered))
            .collect();
        let retry = !state.rejected.is_empty() && visible != state.visible;
        if state.sent.as_ref() == Some(&targets) && !retry {
            state.visible = visible;
            return;
        }
        let Some(generation) = state.generation.checked_add(1) else {
            return;
        };
        if send(Command::SetCommandIconView {
            generation,
            targets: targets.clone(),
        }) {
            state.generation = generation;
            state.sent = Some(targets);
            state.visible = visible;
            state.rejected.clear();
        }
    }

    pub(crate) fn receive_command_icon(
        &mut self,
        generation: u64,
        identity: &str,
        pixels: &IconPixels,
    ) {
        let state = self.command_toolbar.native.borrow();
        if state.generation != generation
            || !state
                .sent
                .as_ref()
                .is_some_and(|rows| rows.iter().any(|row| row.identity == identity))
        {
            return;
        }
        drop(state);
        // Configuration/visibility can change before the next bridge pulse.
        if !self.command_icon_targets().iter().any(|row| {
            row.identity == identity
                && matches!(row.content, crate::ThumbnailContent::ExecutableIcon(_))
        }) {
            return;
        }
        self.receive_thumbnail_at_resolution(
            identity,
            0,
            pixels.width,
            pixels.height,
            64,
            &pixels.rgba,
        );
        if self.thumbnail_coverage(identity, 0) < 64 {
            self.command_toolbar
                .native
                .borrow_mut()
                .rejected
                .insert(identity.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_refusal_retries_on_same_identity_size_or_animation_changes_only_once() {
        let root = std::env::current_dir().unwrap();
        let mut p = Presenter::new(root.clone(), root.clone());
        let mut rows = vec![ThumbnailTarget {
            identity: "command-icon:1:tool".into(),
            content: crate::ThumbnailContent::ExecutableIcon(root.join("tool")),
            selected: false,
            hovered: false,
            confirmed_ssd: false,
            edge: 64,
        }];
        let count = Cell::new(0);
        let send = |_: Command| {
            count.set(count.get() + 1);
            true
        };
        p.refresh_command_icons(&rows, &send);
        // Simulate the retained-pixel budget's refusal notification. No pixels are retained here.
        for change in 0..3 {
            p.command_toolbar
                .native
                .borrow_mut()
                .rejected
                .insert(rows[0].identity.clone());
            p.refresh_command_icons(&rows, &send);
            assert_eq!(
                count.get(),
                change + 1,
                "unchanged pulses must not busy-retry"
            );
            match change {
                0 => rows[0].edge += 1,
                1 => rows[0].selected = true,
                _ => rows[0].hovered = true,
            }
            p.refresh_command_icons(&rows, &send);
            assert_eq!(
                count.get(),
                change + 2,
                "same-identity resource change retries once"
            );
            p.refresh_command_icons(&rows, &send);
            assert_eq!(count.get(), change + 2);
        }
    }
}
