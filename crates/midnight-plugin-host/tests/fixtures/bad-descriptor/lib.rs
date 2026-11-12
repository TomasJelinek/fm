//! Generic hostile descriptor fixture; no application or plugin implementation dependency.
#[repr(C)]
struct Header {
    struct_size: u32,
    major: u16,
    minor: u16,
    kind: u32,
    minimum: u16,
    reserved: u16,
}
/// # Safety
/// Caller supplies writable capacity bytes aligned as the public descriptor header.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn midnight_plugin_entry_v1(
    output: *mut std::ffi::c_void,
    capacity: u32,
) -> u32 {
    if capacity < 16 || output.is_null() {
        return 1;
    }
    // SAFETY: caller's capacity and alignment contract; only the 16-byte header is written.
    unsafe {
        output.cast::<Header>().write(Header {
            struct_size: capacity,
            major: 2,
            minor: 0,
            kind: 1,
            minimum: 0,
            reserved: 0,
        });
    }
    0
}
