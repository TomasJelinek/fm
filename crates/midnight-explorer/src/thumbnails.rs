//! Composition bridge: copy bounded visible identities, and present completed RGBA buffers.
use midnight_thumbnail_host::service::{self, Service, Target, View};
use midnight_ui::{MainWindow, Presenter, ThumbnailTarget};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

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

pub(crate) fn install(window: &MainWindow, presenter: Rc<RefCell<Presenter>>) -> Controller {
    install_service(window, presenter, Rc::new(Service::installed()))
}

fn install_service(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    service: Rc<Service>,
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
    let mut previous: Vec<ThumbnailTarget> = Vec::new();
    let mut tokens: HashMap<String, u64> = HashMap::new();
    let mut next_token = 1u64;
    let mut tick = 0u64;
    let mut last_epoch = 1u64;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = window
                .upgrade()
                .filter(|window| window.window().is_visible())
            else {
                service.stop();
                return;
            };
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
            let mut changed = false;
            if epoch.get() != last_epoch {
                previous.clear();
                tokens.clear();
                last_epoch = epoch.get();
                changed = true;
            }
            if let Some(notice) = service.take_notice() {
                state.report_thumbnail_error(notice);
                changed = true;
            }
            if rows != previous {
                tokens.retain(|identity, _| rows.iter().any(|row| &row.identity == identity));
                let targets = rows
                    .iter()
                    .filter_map(|row| {
                        let token = if let Some(token) = tokens.get(&row.identity) {
                            *token
                        } else {
                            let token = next_token;
                            next_token = next_token.checked_add(1)?;
                            tokens.insert(row.identity.clone(), token);
                            token
                        };
                        Some(Target {
                            token,
                            identity: row.path.as_os_str().as_encoded_bytes().to_vec(),
                            extension: row
                                .path
                                .extension()
                                .and_then(|value| value.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase(),
                            selected: row.selected,
                            hovered: row.hovered,
                            capture: service::native_capture(row.path.clone(), row.confirmed_ssd),
                        })
                    })
                    .collect();
                service.set_view(View {
                    generation: epoch.get(),
                    targets,
                });
                previous = rows;
                changed = true;
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
                let frame = &ready.frame;
                changed |= state.receive_thumbnail(
                    identity,
                    frame.frame,
                    frame.width,
                    frame.height,
                    &frame.rgba,
                );
            }
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(2) {
                changed |= state.advance_thumbnail_animation();
            }
            // Slint can synchronously read presenter-backed models while refreshing.
            // Finish all mutations before it borrows the presenter again.
            drop(state);
            if changed {
                midnight_ui::refresh(&window, &presenter.borrow());
            }
        },
    );
    Controller {
        timer,
        service: control,
    }
}

#[cfg(test)]
#[path = "thumbnails_tests.rs"]
mod tests;
