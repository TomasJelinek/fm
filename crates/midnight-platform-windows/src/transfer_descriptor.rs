//! A security descriptor that `GetKernelObjectSecurity` returned, checked before its
//! owner, group and ACLs are walked. The descriptor of a file on a share comes from the
//! remote server, and the walk trusts the offsets and sizes inside it: a malformed one
//! could make it read past the buffer. Windows returns self-relative descriptors.
use std::io;
use windows_sys::Win32::Security::{
    GetSecurityDescriptorLength, IsValidSecurityDescriptor, SE_SELF_RELATIVE,
};

/// `SECURITY_DESCRIPTOR_RELATIVE`: revision, padding and control, then the offsets of
/// the owner, group, SACL and DACL.
const HEADER: usize = 20;
/// A SID's fixed part (revision, sub-authority count, authority); four bytes follow per
/// sub-authority. An ACL's header (revision, padding, size, count, padding).
const PART_HEADER: usize = 8;

/// Refuses a descriptor in the first `length` bytes of `storage` unless its header and
/// every SID and ACL it names lie within those bytes, Windows validates it
/// (`IsValidSecurityDescriptor`) and its own length (`GetSecurityDescriptorLength`)
/// fits them.
pub(super) fn checked(storage: &[u64], length: usize) -> io::Result<()> {
    let invalid = |why: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("The file system returned an invalid security descriptor ({why})"),
        )
    };
    // SAFETY: plain bytes of the initialized u64 storage, no longer than it.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            storage.as_ptr().cast::<u8>(),
            length.min(size_of_val(storage)),
        )
    };
    if bytes.len() < HEADER {
        return Err(invalid("shorter than its header"));
    }
    let control = u16::from_le_bytes([bytes[2], bytes[3]]);
    if bytes[0] != 1 || control & SE_SELF_RELATIVE == 0 {
        return Err(invalid("not a self-relative descriptor"));
    }
    for (at, sid) in [(4, true), (8, true), (12, false), (16, false)] {
        let offset = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        if offset == 0 {
            continue;
        }
        let Some(part) = bytes
            .get(offset as usize..)
            .filter(|part| part.len() >= PART_HEADER)
        else {
            return Err(invalid("a part lies outside it"));
        };
        let size = if sid {
            PART_HEADER + 4 * usize::from(part[1])
        } else {
            usize::from(u16::from_le_bytes([part[2], part[3]]))
        };
        if size < PART_HEADER || size > part.len() {
            return Err(invalid("a part extends past it"));
        }
    }
    let descriptor = storage.as_ptr().cast_mut().cast();
    // SAFETY: the header and every SID and ACL it names lie within `bytes` (checked
    // above), so Windows reads only the live, aligned storage.
    if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
        return Err(invalid("Windows refuses it"));
    }
    // SAFETY: as above; the descriptor was validated.
    if unsafe { GetSecurityDescriptorLength(descriptor) } as usize > bytes.len() {
        return Err(invalid("longer than the returned buffer"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetKernelObjectSecurity,
        OWNER_SECURITY_INFORMATION,
    };

    /// The owner, group and DACL of this test's own executable, and their length.
    fn descriptor() -> (Vec<u64>, usize) {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let information =
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut needed = 0;
        // SAFETY: a live READ_CONTROL handle and a zero-size output probe.
        unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                information,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        assert!(needed > 0);
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: the writable aligned allocation covers the probed length.
        let read = unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                information,
                storage.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        assert_ne!(read, 0, "{}", io::Error::last_os_error());
        let length = storage.len() * 8;
        (storage, length)
    }

    fn bytes(storage: &mut [u64]) -> &mut [u8] {
        // SAFETY: plain bytes of the u64 storage.
        unsafe { std::slice::from_raw_parts_mut(storage.as_mut_ptr().cast(), storage.len() * 8) }
    }

    fn dacl(storage: &mut [u64]) -> usize {
        let bytes = bytes(storage);
        u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize
    }

    #[test]
    fn a_descriptor_of_the_file_system_is_accepted() {
        let (storage, length) = descriptor();
        checked(&storage, length).unwrap();
    }

    #[test]
    fn a_truncated_descriptor_is_refused_before_it_is_walked() {
        let (storage, _) = descriptor();
        // SAFETY: as in `checked`.
        let used = unsafe { GetSecurityDescriptorLength(storage.as_ptr().cast_mut().cast()) };
        let used = used as usize;
        for length in [0, 4, HEADER - 1, HEADER, used / 2, used - 1] {
            assert!(
                checked(&storage, length).is_err(),
                "{length} of {used} bytes"
            );
        }
        checked(&storage, used).unwrap();
    }

    #[test]
    fn garbage_and_out_of_bounds_parts_are_refused() {
        assert!(checked(&[0xabab_abab_abab_abab; 16], 128).is_err());
        assert!(checked(&[0; 16], 128).is_err());
        // A DACL offset past the buffer.
        let (mut storage, length) = descriptor();
        bytes(&mut storage)[16..20].copy_from_slice(&(length as u32 + 64).to_le_bytes());
        assert!(checked(&storage, length).is_err());
        // A DACL whose own size extends past the buffer.
        let (mut storage, length) = descriptor();
        let at = dacl(&mut storage);
        assert_ne!(at, 0, "the fixture has a DACL");
        bytes(&mut storage)[at + 2..at + 4].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(checked(&storage, length).is_err());
        // In bounds, but not a valid ACL: Windows refuses it.
        let (mut storage, length) = descriptor();
        let at = dacl(&mut storage);
        bytes(&mut storage)[at] = 0x7f;
        assert!(checked(&storage, length).is_err());
    }
}
