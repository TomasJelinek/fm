use slint::{BackendSelector, winit_030::winit::platform::windows::WindowAttributesExtWindows};

pub(super) fn initialize_identity() -> Result<(), slint::PlatformError> {
    // The executable resource and Slint window icon supply Windows branding.
    Ok(())
}

pub(super) fn configure(selector: BackendSelector, default_renderer: bool) -> BackendSelector {
    // Our backend STA owns OLE admission and acknowledges drops only after
    // reserving copy capacity. Winit's eager UI-thread target must not register.
    let selector = selector.with_winit_window_attributes_hook(|attributes| {
        midnight_ui::configure_dialog_window_attributes(attributes.with_drag_and_drop(false))
    });
    if default_renderer {
        // Prefer the native Windows API. Keep the explicit backend override
        // available for driver diagnostics.
        // Slint 1.18's generic Direct3D requirement is rejected by its WGPU
        // adapter preflight and silently selects software. The explicit WGPU
        // configuration selects DX12 in both preflight and initialization.
        selector.require_wgpu_30(slint::wgpu_30::WGPUConfiguration::Automatic(
            graphics_settings(),
        ))
    } else {
        selector
    }
}

fn graphics_settings() -> slint::wgpu_30::WGPUSettings {
    use slint::wgpu_30::wgpu::InstanceFlags;
    let mut graphics = slint::wgpu_30::WGPUSettings::default();
    if std::env::var_os("WGPU_BACKEND").is_none() {
        graphics.backends = slint::wgpu_30::wgpu::Backends::DX12;
    }
    // Native driver debugging layers substantially delay ordinary panel frames.
    // Keep them explicitly available through WGPU_DEBUG/WGPU_VALIDATION, while
    // preserving validation that prevents unsafe indirect GPU calls.
    let mut flags = InstanceFlags::from_build_config();
    flags.remove(InstanceFlags::DEBUG | InstanceFlags::VALIDATION);
    flags = flags.with_env();
    flags.insert(InstanceFlags::VALIDATION_INDIRECT_CALL);
    graphics.instance_flags = flags;
    graphics
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::wgpu_30::wgpu::{Backends, InstanceFlags};

    #[test]
    fn native_debug_layers_are_explicit_and_indirect_validation_is_retained() {
        if let Ok(mode) = std::env::var("MIDNIGHT_RENDER_FLAGS_PROBE") {
            let graphics = graphics_settings();
            assert_eq!(graphics.backends, Backends::DX12);
            assert_eq!(
                graphics.instance_flags.contains(InstanceFlags::DEBUG),
                mode == "on"
            );
            assert_eq!(
                graphics.instance_flags.contains(InstanceFlags::VALIDATION),
                mode == "on"
            );
            assert!(
                graphics
                    .instance_flags
                    .contains(InstanceFlags::VALIDATION_INDIRECT_CALL)
            );
            return;
        }
        // Environment changes are isolated to child test processes.
        for mode in ["default", "on"] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    std::thread::current().name().unwrap(),
                    "--nocapture",
                ])
                .env("MIDNIGHT_RENDER_FLAGS_PROBE", mode)
                .env("WGPU_BACKEND", "dx12")
                .env("WGPU_VALIDATION_INDIRECT_CALL", "0")
                .env_remove("WGPU_DEBUG")
                .env_remove("WGPU_VALIDATION");
            if mode == "on" {
                command.env("WGPU_DEBUG", "1").env("WGPU_VALIDATION", "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
