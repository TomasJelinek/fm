//! Resource extraction only; no executable is loaded as code or started.
use super::*;
use midnight_core::Cancellation;
use windows_sys::Win32::UI::Shell::SHDefExtractIconW;

pub(super) fn load(path: &Path, size: u16, cancel: &Cancellation) -> Option<IconPixels> {
    if !path.is_absolute() || !(1..=64).contains(&size) || cancel.is_cancelled() {
        return None;
    }
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    let _mode = ErrorMode::new().ok()?;
    let _com = ComGuard::initialize(COINIT_MULTITHREADED as u32)?;
    let mut handle = ptr::null_mut();
    // SAFETY: the native path is terminated and the output points to a writable HICON.
    // Index zero selects the executable's default resource; LOWORD requests this size.
    let result = unsafe {
        SHDefExtractIconW(
            wide.as_ptr(),
            0,
            0,
            &mut handle,
            ptr::null_mut(),
            u32::from(size),
        )
    };
    let icon = (!handle.is_null()).then_some(OwnedIcon(handle))?;
    if result < 0 || cancel.is_cancelled() {
        return None;
    }
    let mut surface = Dib::new(size)?;
    let black = surface.draw(icon.0, 0)?;
    let white = surface.draw(icon.0, 255)?;
    let rgba = straight_rgba(&black, &white);
    if cancel.is_cancelled() || !rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0) {
        return None;
    }
    IconPixels::new(u32::from(size), u32::from(size), rgba).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executable_icon_rejects_invalid_or_cancelled_requests() {
        let cancel = Cancellation::default();
        for path in ["tool.exe", "C:\\tool\0.exe"] {
            assert!(load(Path::new(path), 64, &cancel).is_none());
        }
        let path = std::env::current_exe().unwrap();
        assert!(load(&path, 0, &cancel).is_none());
        assert!(load(&path, 65, &cancel).is_none());
        cancel.cancel();
        assert!(load(&path, 64, &cancel).is_none());
    }
    #[test]
    fn executable_icon_extracts_real_unicode_path_resources_without_launching() {
        let directory = tempfile::tempdir().unwrap();
        let source = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
        let target = directory.path().join("executable 文档.exe");
        std::fs::copy(source, &target).unwrap();
        let pixels = load(&target, 64, &Cancellation::default()).expect("system executable icon");
        assert_eq!(
            (pixels.width, pixels.height, pixels.rgba.len()),
            (64, 64, 16384)
        );
        assert!(
            pixels
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0)
        );
    }
}
