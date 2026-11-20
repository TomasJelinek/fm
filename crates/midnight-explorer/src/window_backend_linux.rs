use slint::{BackendSelector, wgpu_30::wgpu};

pub(super) fn initialize_identity() -> Result<(), slint::PlatformError> {
    // Wayland resolves the launcher icon through this matching desktop entry.
    slint::set_xdg_app_id("io.github.TomasJelinek.MidnightExplorer")
}

pub(super) fn configure(selector: BackendSelector, default_renderer: bool) -> BackendSelector {
    // Winit selects X11/Wayland; the worker-owned XDND proxy is attached after
    // the actual X11 window exists. Keep the caller's renderer configuration.
    let selector =
        selector.with_winit_window_attributes_hook(midnight_ui::configure_dialog_window_attributes);
    if !default_renderer || !shared_device_enabled(std::env::var_os("MIDNIGHT_SHARED_WGPU")) {
        return selector;
    }
    match shared_device() {
        Some(configuration) => selector.require_wgpu_30(configuration),
        // No hardware adapter: Slint's own probe then chooses the software fallback as before.
        None => selector,
    }
}

/// `MIDNIGHT_SHARED_WGPU=0` restores Slint's per-window device creation for comparison.
fn shared_device_enabled(setting: Option<std::ffi::OsString>) -> bool {
    setting.is_none_or(|value| value != "0")
}

/// One wgpu device for every window. Without it Slint enumerates adapters while
/// constructing each window and creates a new instance, adapter and device when
/// the window is shown: about 90 ms and 300 ms of every viewer opening on a
/// discrete-GPU X11 desktop (docs/verification/2026-11-20-viewer-speed.md).
/// Adapter and backend choice follow Slint's defaults and the WGPU_* variables.
fn shared_device() -> Option<slint::wgpu_30::WGPUConfiguration> {
    // Slint avoids these backends unless the environment names only them.
    let avoided = wgpu::Backends::GL | wgpu::Backends::METAL | wgpu::Backends::DX12;
    let requested = wgpu::Backends::from_env().unwrap_or_default();
    let backends = if (requested & !avoided).is_empty() {
        requested
    } else {
        requested & !avoided
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = ready(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))?
    .ok()?;
    if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
        return None;
    }
    // Slint's default device: every stable adapter feature and limit.
    let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: adapter.features() - wgpu::Features::all_experimental_mask(),
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::default(),
    }))?
    .ok()?;
    // Slint keeps its copy in thread-local platform state that is dropped after other
    // thread-locals at exit; dropping the last reference there aborts the process.
    // This reference lives until the process ends, so that drop only releases a count.
    std::mem::forget((
        instance.clone(),
        adapter.clone(),
        device.clone(),
        queue.clone(),
    ));
    Some(slint::wgpu_30::WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

/// Native wgpu requests complete on their first poll.
fn ready<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut context) {
        std::task::Poll::Ready(value) => Some(value),
        std::task::Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests {
    use super::shared_device_enabled;

    #[test]
    fn only_an_explicit_zero_disables_the_shared_device() {
        assert!(shared_device_enabled(None));
        assert!(shared_device_enabled(Some("1".into())));
        assert!(shared_device_enabled(Some("".into())));
        assert!(!shared_device_enabled(Some("0".into())));
    }
}
