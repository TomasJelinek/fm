//! Hardware renderer configuration shared by native windows and the viewer timing probe.
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu};
use std::sync::OnceLock;

/// Select the application's Windows graphics policy. A successfully prepared device is
/// kept for the process lifetime so Slint's thread-local teardown only releases a count.
/// When a shared device cannot present on the desktop, Slint selects per window.
pub fn windows_gpu_configuration(share: bool) -> WGPUConfiguration {
    static SHARED: OnceLock<WGPUConfiguration> = OnceLock::new();
    let settings = graphics_settings();
    if share {
        if let Some(configuration) = SHARED.get() {
            return configuration.clone();
        }
        if let Some(configuration) = shared_device(&settings) {
            return SHARED.get_or_init(|| configuration).clone();
        }
    }
    WGPUConfiguration::Automatic(settings)
}

fn graphics_settings() -> WGPUSettings {
    let mut graphics = WGPUSettings::default();
    if std::env::var_os("WGPU_BACKEND").is_none() {
        graphics.backends = wgpu::Backends::VULKAN | wgpu::Backends::DX12;
    }
    // Driver debug layers are opt-in. Always validate indirect GPU calls.
    let mut flags = wgpu::InstanceFlags::from_build_config();
    flags.remove(wgpu::InstanceFlags::DEBUG | wgpu::InstanceFlags::VALIDATION);
    flags = flags.with_env();
    flags.insert(wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL);
    graphics.instance_flags = flags;
    graphics
}

fn shared_device(settings: &WGPUSettings) -> Option<WGPUConfiguration> {
    // Only share the native Windows backends. Other explicit backend choices keep
    // Slint's automatic configuration, including its GL pixel-format negotiation.
    let backends = settings.backends & (wgpu::Backends::VULKAN | wgpu::Backends::DX12);
    if backends.is_empty() {
        return None;
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: settings.instance_flags,
        backend_options: settings.backend_options.clone(),
        memory_budget_thresholds: settings.instance_memory_budget_thresholds,
        display: None,
    });
    let adapter = presentation::adapter(&instance)?;
    if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
        return None;
    }
    // Match Slint's Automatic device descriptor, including the adapter's actual
    // swapchain resolution limits. Each viewer still owns its own native surface.
    let descriptor = wgpu::DeviceDescriptor {
        label: settings.device_label.as_deref(),
        required_features: settings.device_required_features,
        required_limits: settings
            .device_required_limits
            .clone()
            .using_resolution(adapter.limits()),
        experimental_features: settings.device_experimental_features,
        memory_hints: settings.device_memory_hints.clone(),
        trace: wgpu::Trace::default(),
    };
    let (device, queue) = ready(adapter.request_device(&descriptor))?.ok()?;
    Some(WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

// wgpu's native requests complete on the first poll. A deferred request falls back.
fn ready<F: std::future::Future>(future: F) -> Option<F::Output> {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).poll(&mut context) {
        std::task::Poll::Ready(value) => Some(value),
        std::task::Poll::Pending => None,
    }
}

#[allow(unsafe_code)]
mod presentation {
    use super::{ready, wgpu};
    use std::{num::NonZeroIsize, ptr};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_EX_NOACTIVATE, WS_POPUP},
    };

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: this function's thread created and exclusively owns this HWND.
            unsafe { DestroyWindow(self.0) };
        }
    }

    // SAFETY: EnumDisplayMonitors calls this synchronously with the caller's live
    // vector and a valid monitor rectangle. No window or pointer escapes enumeration.
    unsafe extern "system" fn collect(_: HMONITOR, _: HDC, bounds: *mut RECT, data: LPARAM) -> i32 {
        let points = unsafe { &mut *(data as *mut Vec<(i32, i32)>) };
        let rect = unsafe { &*bounds };
        points.push((rect.left, rect.top));
        1
    }

    pub(super) fn adapter(instance: &wgpu::Instance) -> Option<wgpu::Adapter> {
        let mut screens: Vec<(i32, i32)> = Vec::new();
        // SAFETY: callback and vector live throughout this synchronous enumeration.
        if unsafe {
            EnumDisplayMonitors(
                ptr::null_mut(),
                ptr::null(),
                Some(collect),
                (&mut screens as *mut _) as LPARAM,
            )
        } == 0
        {
            return None;
        }
        // SAFETY: a null module name borrows the running process's module.
        let module = NonZeroIsize::new(unsafe { GetModuleHandleW(ptr::null()) } as isize)?;
        let class: Vec<_> = "STATIC\0".encode_utf16().collect();
        let mut adapter = None;
        for (x, y) in screens {
            // SAFETY: the system window class is NUL-terminated; this hidden popup has
            // no parent, callbacks or input. Its guard destroys it on the creating thread.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    ptr::null(),
                    WS_POPUP,
                    x,
                    y,
                    1,
                    1,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    module.get() as _,
                    ptr::null(),
                )
            };
            let hwnd = NonZeroIsize::new(hwnd as isize)?;
            let _window = Window(hwnd.get() as _);
            let mut raw = wgpu::rwh::Win32WindowHandle::new(hwnd);
            raw.hinstance = Some(module);
            // SAFETY: the surface is a local declared after the owning window. It
            // is dropped first on every exit; neither handle is transferred to Slint.
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_window_handle: raw.into(),
                    raw_display_handle: Some(wgpu::rwh::WindowsDisplayHandle::new().into()),
                })
            }
            .ok()?;
            if adapter.is_none() {
                adapter = ready(wgpu::util::initialize_adapter_from_env_or_default(
                    instance,
                    Some(&surface),
                ))?
                .ok();
            }
            // A device chosen without checking presentation can work for compute and
            // still fail when Slint creates its swapchain, especially on hybrid systems.
            surface.get_default_config(adapter.as_ref()?, 1, 1)?;
        }
        adapter
    }
}
