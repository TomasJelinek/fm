//! Wayland frame callbacks missing from Slint 1.18.1's FemtoVG-WGPU adapter.
use slint::{
    GraphicsAPI, RenderingState,
    winit_030::{
        WinitWindowAccessor,
        winit::{
            event::WindowEvent,
            raw_window_handle::{HasWindowHandle, RawWindowHandle},
            window::{Window, WindowId},
        },
    },
};
use std::{collections::HashSet, rc::Rc};

/// Installs successful-presentation notifications for native Wayland windows.
/// Feed every event from the application's winit handler before Slint handles it.
/// X11 and renderers that do not expose a rendering notifier need no workaround.
/// An existing custom rendering notifier keeps its slot and owns presentation notification.
#[derive(Default)]
pub struct NativePresentation {
    installed: HashSet<WindowId>,
}

impl NativePresentation {
    /// Observe a native transition without consuming or drawing the event.
    pub fn observe(
        &mut self,
        id: WindowId,
        native: Option<&Window>,
        window: Option<&slint::Window>,
        event: &WindowEvent,
    ) {
        if matches!(event, WindowEvent::Destroyed) {
            self.installed.remove(&id);
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested) || self.installed.contains(&id) {
            return;
        }
        let (Some(native), Some(window)) = (native, window) else {
            return;
        };
        if !native
            .window_handle()
            .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
        {
            return;
        }
        let weak =
            Rc::downgrade(&i_slint_core::window::WindowInner::from_pub(window).window_adapter());
        // Only successful rendering reaches AfterRendering, immediately before
        // FemtoVG presents. Registering a callback before an acquire that skips
        // would leave Wayland waiting for a surface commit that never happens.
        let _ = window.set_rendering_notifier(move |phase, api| {
            if matches!(phase, RenderingState::AfterRendering)
                && matches!(api, GraphicsAPI::WGPU30 { .. })
                && let Some(adapter) = weak.upgrade()
            {
                adapter
                    .window()
                    .with_winit_window(|native| native.pre_present_notify());
            }
        });
        // Unsupported renderers and an existing notifier retain their ownership.
        self.installed.insert(id);
    }
}
