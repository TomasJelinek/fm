//! MPR imports only; no connection, credential, shell or filesystem APIs are used.
use super::*;
use std::ptr;
pub(super) struct System;
impl Api for System {
    fn open(&self, scope: u32, parent: Option<&Resource>) -> (u32, usize) {
        let mut strings: [Option<Vec<u16>>; 4] = std::array::from_fn(|_| None);
        if let Some(parent) = parent {
            for (slot, field) in strings.iter_mut().zip([
                &parent.local,
                &parent.remote,
                &parent.comment,
                &parent.provider,
            ]) {
                if let Some(field) = field {
                    let mut value = Vec::new();
                    if value.try_reserve_exact(field.len() + 1).is_err() {
                        return (ERROR_NOT_ENOUGH_MEMORY, 0);
                    }
                    value.extend_from_slice(field);
                    value.push(0);
                    *slot = Some(value);
                }
            }
        }
        let pointer = |position: usize| {
            strings[position]
                .as_ref()
                .map_or(ptr::null_mut(), |value| value.as_ptr().cast_mut())
        };
        let native = parent.map(|parent| NETRESOURCEW {
            dwScope: parent.scope,
            dwType: parent.kind,
            dwDisplayType: parent.display_type,
            // RESOURCE_CONTEXT does not define dwUsage. Explicitly selected
            // server/domain/provider nodes are containers for this one request;
            // the retained resource identity itself is never rewritten.
            dwUsage: parent.usage | RESOURCEUSAGE_CONTAINER,
            lpLocalName: pointer(0),
            lpRemoteName: pointer(1),
            lpComment: pointer(2),
            lpProvider: pointer(3),
        });
        let mut handle = ptr::null_mut();
        // SAFETY: all optional strings are owned, NUL terminated and stable for
        // this synchronous input-only call; the output handle slot is writable.
        // No ATTACHED flag or connection/authentication function is requested.
        let status = unsafe {
            WNetOpenEnumW(
                scope,
                RESOURCETYPE_DISK,
                0,
                native
                    .as_ref()
                    .map_or(ptr::null(), |value| value as *const _),
                &mut handle,
            )
        };
        (status, handle as usize)
    }
    fn next(&self, handle: usize, count: &mut u32, buffer: &mut Buffer, bytes: &mut u32) -> u32 {
        // SAFETY: the cursor owns a successful open handle exclusively. The
        // initialized, suitably aligned buffer has exactly the supplied writable
        // extent; count/size are live output variables. No pointer escapes this call.
        unsafe { WNetEnumResourceW(handle as HANDLE, count, buffer.pointer(), bytes) }
    }
    fn close(&self, handle: usize) {
        // SAFETY: Enumeration creates ownership only after successful open and
        // calls this exactly once after any in-flight synchronous call returns.
        unsafe {
            WNetCloseEnum(handle as HANDLE);
        }
    }
    fn extended_error(&self) -> Option<u32> {
        let mut code = 0;
        let mut description = [0u16; 1024];
        let mut provider = [0u16; 1024];
        // SAFETY: all three outputs are initialized writable storage of the
        // specified extent. This reads the calling thread's MPR error only;
        // descriptions/provider strings are never exposed, logged or used as identity.
        let status = unsafe {
            WNetGetLastErrorW(
                &mut code,
                description.as_mut_ptr(),
                description.len() as u32,
                provider.as_mut_ptr(),
                provider.len() as u32,
            )
        };
        (status == NO_ERROR).then_some(code)
    }
}
