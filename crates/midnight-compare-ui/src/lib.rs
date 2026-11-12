//! Presentation and short input callbacks only. The comparator service owns file work.
#![deny(unsafe_code)]
mod file_choice;
mod fonts;
mod images;
use midnight_compare_engine::{ImageMode, Input, Output, Service, Session};
use midnight_core::{FileCompareLaunch, ShortcutChord, ViewerAppearance, ViewerGeometry};
use slint::{
    ComponentHandle, Timer, TimerMode,
    winit_030::{EventResult, WinitWindowAccessor},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::ComparatorWindow;
pub use midnight_ui_input::WindowPlacement;

pub type KeyResolver = Rc<dyn Fn(&ShortcutChord, bool) -> Option<String>>;
pub type FontListener = Rc<dyn Fn(String)>;

pub struct ComparatorWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
    picker_owner: file_choice::PickerOwner,
    font_listener: Rc<RefCell<Option<FontListener>>>,
}
struct OpenWindow {
    window: ComparatorWindow,
    session: Session,
    timer: Timer,
    closed: Cell<bool>,
    geometry: Cell<(u32, u32, u32)>,
    geometry_revision: Cell<u64>,
    pending: RefCell<Vec<Input>>,
    choice_paths: RefCell<[std::path::PathBuf; 2]>,
    fonts: RefCell<Option<midnight_compare_engine::FontChoices>>,
    requested_font: RefCell<Option<String>>,
    surfaces: RefCell<images::Surfaces>,
}
impl OpenWindow {
    fn send(&self, input: Input) {
        let mut pending = self.pending.borrow_mut();
        if let Some(old) = pending.last_mut().filter(|old| {
            !matches!(
                input,
                Input::Difference { .. } | Input::DifferenceEdge { .. } | Input::Zoom(_)
            ) && std::mem::discriminant(*old) == std::mem::discriminant(&input)
        }) {
            if let (Input::Scroll { x, y }, Input::Scroll { x: new_x, y: new_y }) =
                (&mut *old, &input)
            {
                *x = (*x + new_x).clamp(-262144.0, 262144.0);
                *y = (*y + new_y).clamp(-262144.0, 262144.0);
            } else {
                *old = input;
            }
        } else if pending.len() < 64 {
            pending.push(input);
        }
    }
    fn close(&self) {
        if !self.closed.replace(true) {
            self.session.close();
            self.timer.stop();
            self.window.set_surface(slint::Image::default());
            self.window.set_left_surface(slint::Image::default());
            self.window.set_right_surface(slint::Image::default());
            *self.surfaces.borrow_mut() = images::Surfaces::default();
            self.window.set_accessible_text("".into());
            let _ = self.window.hide();
        }
    }
}

impl ComparatorWindows {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            windows: RefCell::new(Vec::new()),
            picker_owner: |_| midnight_core::PathPickerOwner::None,
            font_listener: Rc::new(RefCell::new(None)),
        }
    }
    /// Persist an explicitly chosen family through the application's preferences backend.
    pub fn set_font_listener(&self, listener: FontListener) {
        *self.font_listener.borrow_mut() = Some(listener);
    }
    /// Reuse the host's platform-specific window identity without performing native I/O here.
    pub fn with_picker_owner(
        mut self,
        owner: fn(&slint::Window) -> midnight_core::PathPickerOwner,
    ) -> Self {
        self.picker_owner = owner;
        self
    }
    pub fn open(
        &self,
        launch: FileCompareLaunch,
        appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
    ) -> Result<slint::Weak<ComparatorWindow>, String> {
        self.windows.borrow_mut().retain(|open| !open.closed.get());
        if self.windows.borrow().len() >= 2 {
            return Err("Close a comparator before opening another (maximum two).".into());
        }
        let window = ComparatorWindow::new().map_err(|error| error.to_string())?;
        let placement = placement.into();
        let size = placement.logical_size();
        window.set_compact(size.width < 500.0);
        placement.apply(window.window());
        window.set_foreground(color(appearance.foreground));
        window.set_background_color(color(appearance.background));
        window.set_accent(color(appearance.accent));
        window.set_ui_font_size(appearance.font_size);
        window.set_ui_font_family(appearance.family.as_str().into());
        window.set_picker_available(self.service.has_path_picker());
        window.invoke_apply_palette(
            appearance.background[..3]
                .iter()
                .map(|channel| u16::from(*channel))
                .sum::<u16>()
                > 384,
        );
        let session = self.service.open(
            launch,
            appearance,
            ViewerGeometry {
                width: size.width.max(2.0) as u32,
                height: (size.height - 100.0).max(1.0) as u32,
                scale: window.window().scale_factor(),
            },
        )?;
        let open = Rc::new(OpenWindow {
            window,
            session,
            timer: Timer::default(),
            closed: Cell::new(false),
            geometry: Cell::new((0, 0, 0)),
            geometry_revision: Cell::new(0),
            pending: RefCell::new(Vec::new()),
            choice_paths: RefCell::new(Default::default()),
            fonts: RefCell::new(None),
            requested_font: RefCell::new(None),
            surfaces: RefCell::new(images::Surfaces::default()),
        });
        bind(&open, keys);
        fonts::bind(&open, self.font_listener.clone());
        file_choice::bind(&open, self.picker_owner);
        let weak = Rc::downgrade(&open);
        let service = self.service.clone();
        open.timer
            .start(TimerMode::Repeated, Duration::from_millis(16), move || {
                let Some(open) = weak.upgrade() else {
                    return;
                };
                if open.closed.get() {
                    return;
                }
                let window = &open.window;
                let scale = window.window().scale_factor();
                window.set_compact(window.window().size().to_logical(scale).width < 500.0);
                let geometry = (
                    (window.get_content_width() * scale).max(2.0) as u32,
                    (window.get_content_height() * scale).max(1.0) as u32,
                    scale.to_bits(),
                );
                if open.geometry.replace(geometry) != geometry {
                    let revision = open.geometry_revision.get().saturating_add(1);
                    open.geometry_revision.set(revision);
                    window.set_geometry_revision(revision as i32);
                    open.send(Input::Resize(
                        ViewerGeometry {
                            width: geometry.0,
                            height: geometry.1,
                            scale,
                        },
                        revision,
                    ));
                }
                let mut pending = open.pending.borrow_mut();
                let sent = pending
                    .iter()
                    .take_while(|input| open.session.send((*input).clone()))
                    .count();
                pending.drain(..sent);
                drop(pending);
                if let Some(output) = open.session.mailbox.take() {
                    match &output {
                        Output::Frame { frame, names } => {
                            fonts::present(&open, frame.fonts.as_ref());
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
                            window.set_left_name(names[0].as_str().into());
                            window.set_right_name(names[1].as_str().into());
                            window.set_image_mode(frame.image);
                            window.set_position(frame.position as f32);
                            window.set_status(frame.status.as_str().into());
                            window.set_accessible_text(frame.accessible_text.as_str().into());
                            if let Some(layers) = &frame.layers {
                                open.surfaces.borrow_mut().present(
                                    window,
                                    layers,
                                    frame.mode == Some(ImageMode::SideBySide),
                                );
                            } else {
                                let pixels =
                                    slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                        &frame.pixels,
                                        frame.width,
                                        frame.height,
                                    );
                                window.set_surface(slint::Image::from_rgba8_premultiplied(pixels));
                            }
                        }
                        Output::Status(message) => window.set_status(message.as_str().into()),
                        Output::ChooseFiles(choice) => open.show_file_choice(choice),
                    }
                    service.retire(output);
                }
            });
        open.window.show().map_err(|error| error.to_string())?;
        open.window.invoke_focus_content();
        let weak = open.window.as_weak();
        self.windows.borrow_mut().push(open);
        Ok(weak)
    }
    pub fn close_all(&self) {
        for open in self.windows.borrow_mut().drain(..) {
            open.close();
            self.service.retire(open.session.mailbox.take());
        }
    }
}

fn bind(open: &Rc<OpenWindow>, keys: KeyResolver) {
    let weak = Rc::downgrade(open);
    open.window.on_close_comparator(move || {
        if let Some(open) = weak.upgrade() {
            open.close();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.window().on_close_requested(move || {
        if let Some(open) = weak.upgrade() {
            open.close();
        }
        slint::CloseRequestResponse::HideWindow
    });
    let weak = Rc::downgrade(open);
    open.window.on_scroll(move |x, y| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Scroll { x, y });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_seek(move |value| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Seek(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_difference(move |backwards| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Difference { backwards });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_mode(move |mode| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Mode(match mode {
                1 => ImageMode::Swipe,
                2 => ImageMode::OnionSkin,
                _ => ImageMode::SideBySide,
            }));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_amount(move |value| {
        if let Some(open) = weak.upgrade()
            && value.is_finite()
        {
            open.window.set_amount(value.clamp(0.0, 1.0));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_zoom(move |value| {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Zoom(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_fit(move || {
        if let Some(open) = weak.upgrade() {
            open.send(Input::Fit);
        }
    });
    let native = Rc::new(RefCell::new(midnight_ui_input::ShortcutState::default()));
    let state = native.clone();
    let native_keys = keys.clone();
    let mut scroll = midnight_ui_input::NativeScroll::new(&open.window);
    let weak = Rc::downgrade(open);
    open.window.window().on_winit_window_event(move |_, event| {
        let Some(open) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        let scrolled = scroll.observe(open.window.window(), event, None);
        if midnight_ui_input::native_escape_pressed(event) {
            open.close();
            return EventResult::PreventDefault;
        }
        if state.borrow_mut().handle(event, |chord| {
            native_keys(&chord, !open.window.get_content_focused())
                .is_some_and(|action| key(&open, &action))
        }) || scrolled
        {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_input_key(move |event| {
        let Some(open) = weak.upgrade() else {
            return false;
        };
        if midnight_ui_input::escape_pressed(&event) {
            open.close();
            return true;
        }
        native
            .borrow()
            .logical_chord(&event)
            .and_then(|chord| keys(&chord, !open.window.get_content_focused()))
            .is_some_and(|action| key(&open, &action))
    });
}

fn key(open: &OpenWindow, action: &str) -> bool {
    // File fields own editing/navigation keys, even if mapped globally for the viewer.
    if open.window.get_choosing_files() && action != "viewer.close" {
        return false;
    }
    let input = match action {
        "viewer.close" => {
            open.close();
            return true;
        }
        "compare.next-difference" => Input::Difference { backwards: false },
        "compare.previous-difference" => Input::Difference { backwards: true },
        "compare.first-difference" => Input::DifferenceEdge { last: false },
        "compare.last-difference" => Input::DifferenceEdge { last: true },
        "viewer.first" => Input::Seek(0.0),
        "viewer.last" => Input::Seek(1.0),
        "viewer.up" => Input::Scroll { x: 0.0, y: 24.0 },
        "viewer.down" => Input::Scroll { x: 0.0, y: -24.0 },
        "viewer.left" => Input::Scroll { x: 48.0, y: 0.0 },
        "viewer.right" => Input::Scroll { x: -48.0, y: 0.0 },
        "viewer.page-up" => Input::Scroll {
            x: 0.0,
            y: open.window.get_content_height(),
        },
        "viewer.page-down" => Input::Scroll {
            x: 0.0,
            y: -open.window.get_content_height(),
        },
        "viewer.zoom-in" => Input::Zoom(1.25),
        "viewer.zoom-out" => Input::Zoom(0.8),
        "viewer.fit" => Input::Fit,
        "viewer.actual-pixels" => Input::ActualPixels,
        _ => return false,
    };
    open.send(input);
    true
}
fn color(value: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(value[3], value[0], value[1], value[2])
}
