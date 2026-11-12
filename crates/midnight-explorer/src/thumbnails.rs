//! Composition bridge: copy bounded visible identities, and present completed RGBA buffers.
use midnight_core::Command;
use midnight_thumbnail_host::scheduler::Work;
use midnight_thumbnail_host::service::{Service, View};
use midnight_ui::{MainWindow, Presenter, ThumbnailTarget};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};
#[path = "thumbnail_content.rs"]
mod content;

pub(crate) struct Controller {
    timer: slint::Timer,
    service: Rc<Service>,
}
impl Controller {
    pub(crate) fn stop(&self) {
        self.timer.stop();
        self.service.stop();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
struct Deliveries {
    previous: Vec<ThumbnailTarget>,
    scale: f32,
    rejected: HashMap<(u64, u8), Work>,
}
impl Deliveries {
    fn record(&mut self, work: Work, retained_edge: u32) {
        if retained_edge < work.edge {
            self.rejected.insert((work.token, work.frame), work);
        } else {
            self.rejected.remove(&(work.token, work.frame));
        }
    }

    fn changed(&self, rows: &[ThumbnailTarget], scale: f32) -> bool {
        rows != self.previous || scale != self.scale
    }

    fn set_view(
        &mut self,
        rows: Vec<ThumbnailTarget>,
        scale: f32,
        mut retry: impl FnMut(Work) -> bool,
    ) {
        if !self.changed(&rows, scale) {
            return;
        }
        // Keep only bounded metadata until the view changes enough to reclaim space.
        for (_, work) in std::mem::take(&mut self.rejected) {
            if !retry(work) {
                self.rejected.insert((work.token, work.frame), work);
            }
        }
        self.previous = rows;
        self.scale = scale;
    }
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    filesystem: std::sync::Arc<midnight_native_filesystem_host::InstalledProvider>,
    commands: tokio::sync::mpsc::Sender<Command>,
) -> Controller {
    install_service(
        window,
        presenter,
        Rc::new(Service::installed()),
        content::Factory::installed(filesystem),
        Rc::new(move |command| commands.try_send(command).is_ok()),
    )
}

fn install_service(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
    capture: content::Factory,
    send: Rc<dyn Fn(Command) -> bool>,
) -> Controller {
    let control = service.clone();
    let epoch = Rc::new(Cell::new(1u64));
    let clear_epoch = epoch.clone();
    let clear_state = presenter.clone();
    let clear = service.clone();
    window.on_thumbnail_clear_cache(move || {
        clear_epoch.set(clear_epoch.get().wrapping_add(1));
        clear_state.borrow_mut().clear_thumbnail_images();
        clear.clear_cache();
    });
    let timer = slint::Timer::default();
    let window = window.as_weak();
    let mut deliveries = Deliveries::default();
    let mut tokens: HashMap<String, u64> = HashMap::new();
    let mut next_token = 1u64;
    let mut tick = 0u64;
    let mut last_epoch = 1u64;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = window.upgrade() else {
                service.stop();
                return;
            };
            if !window.window().is_visible() {
                return;
            }
            if let Some(path) = service.take_cache_location() {
                window.set_thumbnail_cache_location(path.to_string_lossy().as_ref().into());
            }
            let left_hover = window.get_left_hovered_row();
            let right_hover = window.get_right_hovered_row();
            let ranges = [
                (
                    window.get_left_thumbnail_row_start().max(0) as usize,
                    window.get_left_thumbnail_row_end().max(0) as usize,
                ),
                (
                    window.get_right_thumbnail_row_start().max(0) as usize,
                    window.get_right_thumbnail_row_end().max(0) as usize,
                ),
            ];
            let mut state = presenter.borrow_mut();
            let rows = state.thumbnail_targets(ranges, [left_hover.as_str(), right_hover.as_str()]);
            // Native command artwork shares visible admission, but never enters an image decoder.
            // Retry a rejected send on the next pulse, even when the visible rows are unchanged.
            state.refresh_command_icons(&rows, send.as_ref());
            let scale = window.window().scale_factor();
            let mut refresh_all = false;
            let mut changed = Vec::new();
            if epoch.get() != last_epoch {
                deliveries = Deliveries::default();
                tokens.clear();
                last_epoch = epoch.get();
                refresh_all = true;
            }
            if let Some(notice) = service.take_notice() {
                state.report_thumbnail_error(notice);
                refresh_all = true;
            }
            if deliveries.changed(&rows, scale) {
                // Entering/leaving an animated state can change the displayed
                // frame. Viewport scheduling alone has no presentation change.
                for row in deliveries.previous.iter().chain(&rows) {
                    let old_active = deliveries
                        .previous
                        .iter()
                        .find(|old| old.identity == row.identity)
                        .is_some_and(|old| old.selected || old.hovered);
                    let new_active = rows
                        .iter()
                        .find(|new| new.identity == row.identity)
                        .is_some_and(|new| new.selected || new.hovered);
                    if old_active != new_active {
                        changed.push(row.identity.clone());
                    }
                }
                retain_source_tokens(&mut tokens, &deliveries.previous, &rows);
                let targets = rows
                    .iter()
                    .filter_map(|row| {
                        if matches!(
                            row.content,
                            midnight_ui::ThumbnailContent::ExecutableIcon(_)
                        ) {
                            return None;
                        }
                        let token = if let Some(token) = tokens.get(&row.identity) {
                            *token
                        } else {
                            let token = next_token;
                            next_token = next_token.checked_add(1)?;
                            tokens.insert(row.identity.clone(), token);
                            token
                        };
                        capture.target(row, token, physical_edge(row, scale))
                    })
                    .collect();
                service.set_view(View {
                    generation: epoch.get(),
                    targets,
                });
                // Budget refusals wait for a changed view. Retain only bounded metadata,
                // never the decoded pixels, and keep the same pinned source token.
                deliveries.set_view(rows, scale, |work| {
                    let Some((identity, _)) =
                        tokens.iter().find(|(_, token)| **token == work.token)
                    else {
                        return true;
                    };
                    let retained = state.thumbnail_coverage(identity, work.frame);
                    retained >= work.edge || service.retry_delivery(work, retained)
                });
            }
            // Capacity four bounds pixel uploads per pulse even if a backend finishes rapidly.
            for _ in 0..4 {
                let Some(ready) = service.poll() else {
                    break;
                };
                if ready.generation != epoch.get() {
                    continue;
                }
                let Some((identity, _)) = tokens.iter().find(|(_, token)| **token == ready.token)
                else {
                    continue;
                };
                if deliveries
                    .previous
                    .iter()
                    .find(|row| &row.identity == identity)
                    .is_none_or(|row| ready.edge < physical_edge(row, scale))
                {
                    continue;
                }
                let frame = &ready.frame;
                if state.receive_thumbnail_at_resolution(
                    identity,
                    frame.frame,
                    frame.width,
                    frame.height,
                    ready.edge,
                    &frame.rgba,
                ) {
                    changed.push(identity.clone());
                }
                deliveries.record(
                    Work {
                        generation: ready.generation,
                        token: ready.token,
                        edge: ready.edge,
                        frame: frame.frame,
                    },
                    state.thumbnail_coverage(identity, frame.frame),
                );
            }
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(2) && state.advance_thumbnail_animation() {
                changed.extend(state.animated_thumbnail_identities());
            }
            // Slint can synchronously read presenter-backed models while refreshing.
            // Finish all mutations before it borrows the presenter again.
            drop(state);
            if refresh_all {
                midnight_ui::refresh(&window, &presenter.borrow());
            } else if !changed.is_empty() {
                midnight_ui::refresh_thumbnail_rows(&window, &presenter.borrow(), &changed);
            }
        },
    );
    Controller {
        timer,
        service: control,
    }
}

fn retain_source_tokens(
    tokens: &mut HashMap<String, u64>,
    previous: &[ThumbnailTarget],
    current: &[ThumbnailTarget],
) {
    tokens.retain(|identity, _| {
        previous
            .iter()
            .find(|row| &row.identity == identity)
            .is_some_and(|old| {
                current.iter().any(|new| {
                    new.identity == old.identity
                        && new.content == old.content
                        && new.confirmed_ssd == old.confirmed_ssd
                })
            })
    });
}

fn physical_edge(target: &ThumbnailTarget, scale: f32) -> u32 {
    if matches!(
        target.content,
        midnight_ui::ThumbnailContent::ExecutableIcon(_)
    ) {
        return 64;
    }
    if target.identity.starts_with("command-icon:") {
        return 128;
    }
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    midnight_thumbnail_host::cache::resolution_edge((target.edge as f32 * scale).ceil() as u32)
}

#[cfg(test)]
#[path = "thumbnails_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "thumbnails_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "command_icon_bridge_tests.rs"]
mod command_icon_bridge_tests;
