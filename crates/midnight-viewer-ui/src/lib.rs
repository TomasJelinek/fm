//! Thin UI adapters. Reading, parsing, search, layout and selection run on the backend.
#![deny(unsafe_code)]

mod scrolling;

use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::ViewerWindow;
pub use midnight_ui_input::WindowPlacement;

pub type KeyResolver = Rc<dyn Fn(&ShortcutChord, bool) -> Option<String>>;

pub struct ViewerWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
}

struct OpenWindow {
    window: ViewerWindow,
    session: WindowSession,
    timer: Timer,
    closed: Cell<bool>,
    last_geometry: Cell<(u32, u32, u32)>,
    frame: Cell<u64>,
    raster_size: Cell<(u32, u32)>,
    modes: RefCell<Vec<u16>>,
}

impl ViewerWindows {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            windows: RefCell::new(Vec::new()),
        }
    }

    pub fn open(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
        copy: Rc<dyn Fn(String)>,
    ) -> Result<slint::Weak<ViewerWindow>, String> {
        self.windows
            .borrow_mut()
            .retain(|window| !window.closed.get());
        if self.windows.borrow().len() >= 4 {
            return Err("Close a viewer before opening another (maximum four windows).".into());
        }
        let window = ViewerWindow::new().map_err(|e| e.to_string())?;
        let placement = placement.into();
        let size = placement.logical_size();
        window.set_compact(size.width < 500.0);
        placement.apply(window.window());
        window.set_foreground(color(appearance.foreground));
        window.set_background_color(color(appearance.background));
        window.set_accent(color(appearance.accent));
        window.set_ui_font_size(appearance.font_size);
        window.set_ui_font_family(appearance.family.as_str().into());
        window.invoke_apply_palette(
            u16::from(appearance.background[0])
                + u16::from(appearance.background[1])
                + u16::from(appearance.background[2])
                > 384,
        );
        let geometry = ViewerGeometry {
            width: size.width.max(1.0) as u32,
            height: (size.height - 48.0).max(1.0) as u32,
            scale: window.window().scale_factor(),
        };
        let session = self.service.open(launch, appearance, geometry)?;
        let open = Rc::new(OpenWindow {
            window,
            session,
            timer: Timer::default(),
            closed: Cell::new(false),
            last_geometry: Cell::new((0, 0, 0)),
            frame: Cell::new(0),
            raster_size: Cell::new((1, 1)),
            modes: RefCell::new(Vec::new()),
        });
        bind(&open, keys);
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
                let width = (window.get_content_width() * scale).max(1.0) as u32;
                let height = (window.get_content_height() * scale).max(1.0) as u32;
                let geometry = (width, height, scale.to_bits());
                if geometry != open.last_geometry.get()
                    && open.session.send(ViewerInput::Resize(ViewerGeometry {
                        width,
                        height,
                        scale,
                    }))
                {
                    open.last_geometry.set(geometry);
                }
                for output in open.session.mailbox.take() {
                    match &output {
                        ViewerOutput::Frame(frame) => {
                            if *open.modes.borrow() != frame.modes {
                                *open.modes.borrow_mut() = frame.modes.clone();
                                window.set_modes(slint::ModelRc::new(slint::VecModel::from(
                                    frame
                                        .modes
                                        .iter()
                                        .map(|mode| mode_label(*mode).into())
                                        .collect::<Vec<slint::SharedString>>(),
                                )));
                            }
                            window.set_mode_index(
                                frame
                                    .modes
                                    .iter()
                                    .position(|mode| *mode == frame.mode)
                                    .unwrap_or(0) as i32,
                            );
                            open.frame.set(frame.revision);
                            let image_mode = frame.image.is_some();
                            if image_mode != window.get_image_mode() {
                                window.set_search_visible(false);
                                window.invoke_focus_content();
                            }
                            window.set_image_mode(image_mode);
                            window.set_document_page(
                                frame.image.as_ref().map_or(0, |image| image.page as i32),
                            );
                            window.set_document_pages(
                                frame
                                    .image
                                    .as_ref()
                                    .map_or(0, |image| image.page_count as i32),
                            );
                            window.set_zoom_percent(
                                frame
                                    .image
                                    .as_ref()
                                    .map_or(100.0, |image| (image.zoom * 100.0) as f32),
                            );
                            window.set_wrap_enabled(frame.wrap_enabled);
                            window.set_wrap_available(frame.wrap_available);
                            open.raster_size.set((frame.width, frame.height));
                            let pixels =
                                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                    frame.pixels.as_ref(),
                                    frame.width,
                                    frame.height,
                                );
                            window.set_surface(slint::Image::from_rgba8_premultiplied(pixels));
                            window.set_accessible_text(frame.accessible_text.as_ref().into());
                            window.set_position(frame.position as f32);
                            window.set_vertical_extent(frame.vertical_extent as f32);
                            window.set_horizontal_position(frame.horizontal_position as f32);
                            window.set_horizontal_extent(frame.horizontal_extent as f32);
                            window.set_caption(frame.caption.as_str().into());
                            window.set_status(frame.status.as_str().into());
                        }
                        ViewerOutput::Status(message) => window.set_status(message.as_str().into()),
                        ViewerOutput::CopyText(text) => copy(text.clone()),
                        ViewerOutput::Closed => close(&open),
                    }
                    service.retire(output);
                }
            });
        open.window.show().map_err(|e| e.to_string())?;
        open.window.invoke_focus_content();
        let weak_window = open.window.as_weak();
        self.windows.borrow_mut().push(open);
        Ok(weak_window)
    }

    pub fn close_all(&self) {
        for open in self.windows.borrow_mut().drain(..) {
            close(&open);
        }
    }
}

fn bind(open: &Rc<OpenWindow>, keys: KeyResolver) {
    bind_image(open);
    let weak = Rc::downgrade(open);
    open.window.on_change_wrap(move |wrap| {
        if let Some(open) = weak.upgrade() {
            open.session.send(ViewerInput::Wrap(wrap));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_mode(move |index| {
        if let Some(open) = weak.upgrade()
            && let Ok(index) = usize::try_from(index)
            && let Some(mode) = open.modes.borrow().get(index)
        {
            open.session.send(ViewerInput::Mode(*mode));
        }
    });
    let native = Rc::new(RefCell::new(midnight_ui_input::ShortcutState::default()));
    let weak = Rc::downgrade(open);
    let native_state = native.clone();
    let native_keys = keys.clone();
    open.window.window().on_winit_window_event(move |_, event| {
        let Some(open) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        if midnight_ui_input::native_escape_pressed(event) {
            close(&open);
            return EventResult::PreventDefault;
        }
        let consumed = native_state.borrow_mut().handle(event, |chord| {
            native_keys(&chord, !open.window.get_content_focused())
                .is_some_and(|action| dispatch_key(&open, &action))
        });
        if consumed {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_input_key(move |event, search| {
        let Some(open) = weak.upgrade() else {
            return false;
        };
        if midnight_ui_input::escape_pressed(&event) {
            close(&open);
            return true;
        }
        let Some(chord) = native.borrow().logical_chord(&event) else {
            return false;
        };
        keys(&chord, search).is_some_and(|action| dispatch_key(&open, &action))
    });
    bind_pointer(open);
    let weak = Rc::downgrade(open);
    open.window.on_search(move |backwards| {
        if let Some(open) = weak.upgrade() {
            open.session.send(ViewerInput::Search {
                query: open.window.get_query().to_string(),
                backwards,
                case_sensitive: open.window.get_case_sensitive(),
            });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.window().on_close_requested(move || {
        if let Some(open) = weak.upgrade() {
            close(&open);
        }
        slint::CloseRequestResponse::HideWindow
    });
}

fn dispatch_key(open: &OpenWindow, action: &str) -> bool {
    let input = match action {
        "viewer.close" => {
            close(open);
            return true;
        }
        "viewer.find" => {
            if open.window.get_image_mode() {
                return false;
            }
            open.window.invoke_focus_search();
            return true;
        }
        "viewer.next-file" => ViewerInput::NextFile,
        "viewer.document-previous" | "viewer.document-next" => {
            if open.window.get_document_page() == 0 {
                return false;
            }
            ViewerInput::DocumentPage {
                frame: open.frame.get(),
                delta: if action == "viewer.document-next" {
                    1
                } else {
                    -1
                },
            }
        }
        "viewer.wrap" => ViewerInput::Wrap(!open.window.get_wrap_enabled()),
        "viewer.zoom-in" | "viewer.zoom-out" | "viewer.fit" | "viewer.actual-pixels" => {
            if !open.window.get_image_mode() {
                return false;
            }
            let input = match action {
                "viewer.fit" => ViewerImageInput::Fit,
                "viewer.actual-pixels" => ViewerImageInput::ActualPixels,
                _ => ViewerImageInput::Zoom {
                    factor: if action == "viewer.zoom-in" {
                        1.25
                    } else {
                        0.8
                    },
                    anchor: None,
                },
            };
            ViewerInput::Image {
                frame: open.frame.get(),
                input,
            }
        }
        "viewer.copy" => ViewerInput::Copy {
            frame: open.frame.get(),
        },
        "viewer.select-page" => ViewerInput::SelectPage {
            frame: open.frame.get(),
        },
        "viewer.find-next" | "viewer.find-previous" => {
            if open.window.get_image_mode() {
                return false;
            }
            if open.window.get_query().is_empty() {
                open.window.invoke_focus_search();
                return true;
            }
            ViewerInput::Search {
                query: open.window.get_query().to_string(),
                backwards: action == "viewer.find-previous",
                case_sensitive: open.window.get_case_sensitive(),
            }
        }
        "viewer.first" => ViewerInput::Seek(0.0),
        "viewer.last" => ViewerInput::Seek(1.0),
        action => match scrolling::keyboard(
            action,
            open.window.get_content_height(),
            open.window.window().scale_factor(),
        ) {
            Some(input) => input,
            None => return false,
        },
    };
    open.session.send(input)
}

fn bind_pointer(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_scroll(move |x, y| {
        if let Some(open) = weak.upgrade() {
            let scale = open.window.window().scale_factor();
            open.session.send(ViewerInput::Scroll {
                x: x * scale,
                y: y * scale,
            });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_seek(move |value| {
        if let Some(open) = weak.upgrade() {
            open.session.send(ViewerInput::Seek(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_pan(move |value| {
        if let Some(open) = weak.upgrade() {
            open.session.send(ViewerInput::Pan(value.into()));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_select_point(move |x, y, extend| {
        if let Some(open) = weak.upgrade() {
            let (width, height) = open.raster_size.get();
            open.session.send(ViewerInput::Select {
                frame: open.frame.get(),
                x: x * width as f32 / open.window.get_content_width().max(1.0),
                y: y * height as f32 / open.window.get_content_height().max(1.0),
                extend,
            });
        }
    });
}

fn bind_image(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_document_turn(move |delta| {
        if let Some(open) = weak.upgrade() {
            open.session.send(ViewerInput::DocumentPage {
                frame: open.frame.get(),
                delta,
            });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_command(move |command| {
        if let Some(open) = weak.upgrade() {
            let action = match command {
                0 => "viewer.fit",
                1 => "viewer.actual-pixels",
                2 => "viewer.zoom-in",
                3 => "viewer.zoom-out",
                _ => return,
            };
            dispatch_key(&open, action);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_zoom(move |delta, x, y| {
        if let Some(open) = weak.upgrade()
            && delta.is_finite()
            && delta != 0.0
        {
            let scale = open.window.window().scale_factor();
            open.session.send(ViewerInput::Image {
                frame: open.frame.get(),
                input: ViewerImageInput::Zoom {
                    factor: if delta > 0.0 { 1.25 } else { 0.8 },
                    anchor: Some([x * scale, y * scale]),
                },
            });
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_drag(move |x, y| {
        if let Some(open) = weak.upgrade() {
            let scale = open.window.window().scale_factor();
            open.session.send(ViewerInput::Image {
                frame: open.frame.get(),
                input: ViewerImageInput::Drag {
                    x: x * scale,
                    y: y * scale,
                },
            });
        }
    });
}

fn close(open: &OpenWindow) {
    if !open.closed.replace(true) {
        open.session.close();
        open.timer.stop();
        let _ = open.window.hide();
        open.window.set_surface(slint::Image::default());
        open.window.set_accessible_text("".into());
    }
}

fn mode_label(mode: u16) -> &'static str {
    match mode {
        1 => "Text",
        2 => "Source",
        3 => "Binary",
        4 => "Markdown",
        VIEWER_MODE_IMAGE => "Image",
        _ => "Unknown",
    }
}

fn color(value: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(value[3], value[0], value[1], value[2])
}
