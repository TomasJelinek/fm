//! Wayland frame callbacks missing from Slint 1.18.1's FemtoVG-WGPU adapter, and the
//! present mode of native Wayland surfaces.
use slint::{
    GraphicsAPI, RenderingState,
    wgpu_30::wgpu::PresentMode,
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

/// Installs successful-presentation notifications for native Wayland windows, and selects
/// their surfaces' present mode ([`present_mode`]) before their first frame is drawn.
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
        // All surfaces of the process are then native Wayland. The first call takes effect.
        i_slint_renderer_femtovg::wgpu::set_present_mode_selector(wayland_present_mode);
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

/// The present mode of a window surface. A native Wayland surface presents in Mailbox mode
/// when it offers it. In FIFO mode the discrete GPU's driver 615 committed frames with
/// commit-timing targets that the compositor presented up to a second late
/// (docs/verification/2026-11-25-wayland-fifo-presentation.md). Wayland frame callbacks
/// still pace rendering, so Mailbox draws no extra frames. Everything else keeps Slint's
/// `AutoVsync` (FIFO).
fn present_mode(native_wayland: bool, offered: &[PresentMode]) -> PresentMode {
    if native_wayland && offered.contains(&PresentMode::Mailbox) {
        PresentMode::Mailbox
    } else {
        PresentMode::AutoVsync
    }
}

fn wayland_present_mode(offered: &[PresentMode]) -> PresentMode {
    present_mode(true, offered)
}

#[cfg(test)]
mod tests {
    use super::{
        PresentMode::{AutoVsync, Fifo, Immediate, Mailbox},
        present_mode,
    };

    #[test]
    fn native_wayland_presents_in_mailbox_mode_when_offered() {
        assert_eq!(present_mode(true, &[Mailbox, Fifo]), Mailbox);
        assert_eq!(present_mode(true, &[Fifo, Immediate, Mailbox]), Mailbox);
    }

    #[test]
    fn other_surfaces_keep_slint_fifo() {
        assert_eq!(
            present_mode(true, &[Fifo]),
            AutoVsync,
            "Wayland without Mailbox"
        );
        assert_eq!(
            present_mode(true, &[Fifo, Immediate]),
            AutoVsync,
            "Wayland without Mailbox"
        );
        assert_eq!(
            present_mode(false, &[Mailbox, Fifo]),
            AutoVsync,
            "X11 offering Mailbox"
        );
        assert_eq!(present_mode(false, &[Fifo, Immediate]), AutoVsync, "X11");
    }
}
