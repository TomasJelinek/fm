//! Windows-only diagnostic UI-thread CPU sampling; same contract as native_scroll.
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn GetThreadTimes(
        thread: *mut std::ffi::c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}

pub(super) fn thread_cpu_ms() -> Option<f64> {
    let (mut creation, mut exit, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // SAFETY: outputs have Win32 FILETIME layout and stay live for the call.
    // GetCurrentThread returns a pseudo handle that must not be closed.
    let valid = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    let ticks = |time: FileTime| ((u64::from(time.high) << 32) | u64::from(time.low)) as f64;
    (valid != 0).then_some((ticks(kernel) + ticks(user)) / 10_000.0)
}
