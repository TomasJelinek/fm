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
        // Slint 1.18's generic Direct3D requirement is rejected by its WGPU
        // adapter preflight and silently selects software. Explicit WGPU
        // settings keep the same backend policy in preflight and initialization.
        let share = std::env::var_os("MIDNIGHT_SHARED_WGPU").is_none_or(|value| value != "0");
        selector.require_wgpu_30(midnight_ui::windows_gpu_configuration(share))
    } else {
        selector
    }
}

#[cfg(test)]
mod tests {
    use slint::wgpu_30::wgpu::{Backends, InstanceFlags};

    #[test]
    fn native_debug_layers_are_explicit_and_indirect_validation_is_retained() {
        if let Ok(mode) = std::env::var("MIDNIGHT_RENDER_FLAGS_PROBE") {
            let slint::wgpu_30::WGPUConfiguration::Automatic(graphics) =
                midnight_ui::windows_gpu_configuration(false)
            else {
                panic!("disabled device sharing must use automatic configuration")
            };
            let backend = std::env::var("MIDNIGHT_RENDER_BACKEND_PROBE").unwrap();
            let expected = match backend.as_str() {
                "default" => {
                    assert!(std::env::var_os("WGPU_BACKEND").is_none());
                    Backends::VULKAN | Backends::DX12
                }
                "dx12" => Backends::DX12,
                "vulkan" => Backends::VULKAN,
                "gl" => {
                    assert!(matches!(
                        midnight_ui::windows_gpu_configuration(true),
                        slint::wgpu_30::WGPUConfiguration::Automatic(_)
                    ));
                    Backends::GL
                }
                "vulkan,dx12" => Backends::VULKAN | Backends::DX12,
                other => panic!("unknown backend probe {other}"),
            };
            assert_eq!(
                graphics.backends, expected,
                "backend {backend}, flags {mode}"
            );
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
        for (backend, mode) in ["default", "dx12", "vulkan", "vulkan,dx12", "gl"]
            .into_iter()
            .flat_map(|backend| ["default", "off", "on"].map(|mode| (backend, mode)))
        {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    std::thread::current().name().unwrap(),
                    "--nocapture",
                ])
                .env("MIDNIGHT_RENDER_FLAGS_PROBE", mode)
                .env("MIDNIGHT_RENDER_BACKEND_PROBE", backend)
                .env_remove("WGPU_BACKEND")
                .env("WGPU_VALIDATION_INDIRECT_CALL", "0")
                .env_remove("WGPU_DEBUG")
                .env_remove("WGPU_VALIDATION");
            if mode == "on" {
                command.env("WGPU_DEBUG", "1").env("WGPU_VALIDATION", "1");
            } else if mode == "off" {
                command.env("WGPU_DEBUG", "0").env("WGPU_VALIDATION", "0");
            }
            if backend != "default" {
                command.env("WGPU_BACKEND", backend);
            }
            // Other tests of this binary may start workers; the creation lock covers only the
            // spawn, never the wait for the probe's output.
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let output = midnight_worker_platform::spawn_command(&mut command)
                .unwrap()
                .wait_with_output()
                .unwrap();
            assert!(
                output.status.success(),
                "backend {backend}, flags {mode}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
