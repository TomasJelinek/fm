use crate::Error;
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr, sync::OnceLock};
use windows_sys::Win32::{
    Foundation::{GENERIC_ALL, GENERIC_WRITE, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::{
        DELETE, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
        FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
    },
    System::{
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

const RENAME_RIGHTS: u32 = DELETE
    | FILE_DELETE_CHILD
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_ALL
    | GENERIC_WRITE
    | FILE_WRITE_ATTRIBUTES
    | FILE_WRITE_EA;
const MODIFY_RIGHTS: u32 = RENAME_RIGHTS | FILE_WRITE_DATA | FILE_APPEND_DATA;

pub(crate) fn check(path: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    for (index, component) in path.ancestors().enumerate() {
        check_one(
            component,
            &identity,
            if index == 0 {
                MODIFY_RIGHTS
            } else {
                RENAME_RIGHTS
            },
        )?;
    }
    Ok(())
}

struct Identity {
    user: Vec<u64>,
    installer: [u32; 17],
}

fn cached_installer_sid(
    cache: &OnceLock<[u32; 17]>,
    resolve: impl FnOnce() -> Result<[u32; 17], Error>,
) -> Result<[u32; 17], Error> {
    if let Some(sid) = cache.get() {
        return Ok(*sid);
    }
    // Only the fixed service's successfully resolved identifier is immutable.
    // Never retain a token, descriptor, path decision, or failed resolution here.
    let resolved = resolve()?;
    Ok(*cache.get_or_init(|| resolved))
}

fn installer_sid() -> Result<[u32; 17], Error> {
    static INSTALLER: OnceLock<[u32; 17]> = OnceLock::new();
    cached_installer_sid(&INSTALLER, || {
        let mut installer = [0u32; 17];
        let mut length = 68;
        let mut domain = [0u16; 256];
        let mut domain_len = 256;
        let mut kind = 0;
        let name: Vec<_> = "NT SERVICE\\TrustedInstaller"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // Resolve the fixed OS service SID locally; arbitrary account owners are not trusted.
        // SAFETY: terminated local account name and correctly sized writable native outputs.
        if unsafe {
            LookupAccountNameW(
                ptr::null(),
                name.as_ptr(),
                installer.as_mut_ptr().cast(),
                &mut length,
                domain.as_mut_ptr(),
                &mut domain_len,
                &mut kind,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(installer)
    })
}

impl Identity {
    fn current() -> Result<Self, Error> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        let mut token = ptr::null_mut();
        // SAFETY: read-only query of this process token; owned handle closed on every exit.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut user = vec![0u64; 32];
        let mut needed = 0;
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                user.as_mut_ptr().cast(),
                (user.len() * 8) as u32,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(Self {
            user,
            installer: installer_sid()?,
        })
    }
    fn user_sid(&self) -> PSID {
        // SAFETY: GetTokenInformation initialized the aligned TOKEN_USER and its contained SID.
        unsafe { (*(self.user.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
    fn trusted(&self, sid: PSID) -> bool {
        if sid.is_null() {
            return false;
        }
        // SAFETY: SID pointers belong to valid Win32 token/security descriptors retained here.
        unsafe {
            IsValidSid(sid) != 0
                && (EqualSid(sid, self.user_sid()) != 0
                    || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
                    || EqualSid(sid, self.installer.as_ptr().cast_mut().cast()) != 0)
        }
    }
}

fn check_one(path: &Path, identity: &Identity, mutable: u32) -> Result<(), Error> {
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut owner = ptr::null_mut();
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: absolute terminated path and writable outputs; descriptor owns owner and ACL.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(Error(format!(
            "Cannot validate plugin path permissions: {}",
            io::Error::from_raw_os_error(status as i32)
        )));
    }
    let _descriptor = Local(descriptor);
    if !identity.trusted(owner) || acl.is_null() {
        return Err(Error(format!(
            "Untrusted owner or unrestricted DACL on {}",
            path.display()
        )));
    }
    // SAFETY: GetNamedSecurityInfo returned a valid OS-owned ACL until descriptor is freed.
    for index in 0..unsafe { (*acl).AceCount } {
        let mut raw = ptr::null_mut();
        if unsafe { GetAce(acl, u32::from(index), &mut raw) } == 0 || raw.is_null() {
            return Err(Error("Invalid plugin path ACL".into()));
        }
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        match u32::from(header.AceType) {
            ACCESS_DENIED_ACE_TYPE => {}
            ACCESS_ALLOWED_ACE_TYPE => {
                if usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err(Error("Short plugin path ACE".into()));
                }
                let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
                let sid = ptr::addr_of!(ace.SidStart).cast_mut().cast();
                if ace.Mask & mutable != 0 && !identity.trusted(sid) {
                    return Err(Error(format!(
                        "Plugin path is writable or replaceable by another account: {}",
                        path.display()
                    )));
                }
            }
            _ => {
                return Err(Error(format!(
                    "Unsupported conditional/object ACL on plugin path: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

struct Local(*mut std::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[cfg(test)]
pub(crate) fn fixture_acl(path: &Path, sddl: &str) -> Result<(), Error> {
    let sddl: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = ptr::null_mut();
    // SAFETY: owned terminated SDDL and writable output; Win32 allocates the descriptor.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    let descriptor = Local(descriptor);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
        || present == 0
    {
        return Err(Error("Invalid fixture ACL".into()));
    }
    let status = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl,
            ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32).into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn protect_fixture(path: &Path) -> Result<(), Error> {
    let identity = Identity::current()?;
    let mut raw = ptr::null_mut();
    // SAFETY: valid retained user SID; Win32 allocates a terminated SID string.
    if unsafe { ConvertSidToStringSidW(identity.user_sid(), &mut raw) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let owner_string = Local(raw.cast());
    let mut length = 0;
    // SID strings have a bounded native representation, and returned memory is terminated.
    while length < 256 && unsafe { *raw.add(length) } != 0 {
        length += 1;
    }
    if length == 256 {
        return Err(Error("Oversized user SID string".into()));
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, length) })
        .map_err(|_| Error("Invalid user SID string".into()))?;
    drop(owner_string);
    fixture_acl(
        path,
        &format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_installer_sid_reuses_success_and_retries_failed_resolution() {
        let cache = OnceLock::new();
        let calls = std::cell::Cell::new(0);
        assert!(
            cached_installer_sid(&cache, || {
                calls.set(calls.get() + 1);
                Err(Error("transient account lookup failure".into()))
            })
            .is_err()
        );
        assert!(
            cache.get().is_none(),
            "failed account resolution must remain retryable"
        );
        let expected = [42; 17];
        for _ in 0..4 {
            assert_eq!(
                cached_installer_sid(&cache, || {
                    calls.set(calls.get() + 1);
                    Ok(expected)
                })
                .unwrap(),
                expected
            );
        }
        assert_eq!(
            calls.get(),
            2,
            "only failure and first success should resolve the fixed SID"
        );
    }

    #[test]
    fn warmed_identity_still_rechecks_file_and_ancestor_permissions() {
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("secure HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        protect_fixture(root.path()).unwrap();
        let child = root.path().join("private");
        std::fs::create_dir(&child).unwrap();
        protect_fixture(&child).unwrap();
        let file = child.join("library.dll");
        std::fs::write(&file, b"fixture").unwrap();
        let file = file.canonicalize().unwrap();
        for _ in 0..4 {
            crate::validate_plugin_path(&file).unwrap();
        }
        fixture_acl(&file, "D:P(A;;FA;;;WD)").unwrap();
        let file_error = crate::validate_plugin_path(&file).unwrap_err();
        assert!(file_error.to_string().contains("writable or replaceable"));
        protect_fixture(&file).unwrap();
        crate::validate_plugin_path(&file).unwrap();
        fixture_acl(root.path(), "D:P(A;OICI;FA;;;WD)").unwrap();
        let ancestor_error = crate::validate_plugin_path(&file).unwrap_err();
        assert!(
            ancestor_error
                .to_string()
                .contains("writable or replaceable")
        );
        protect_fixture(root.path()).unwrap();
        crate::validate_plugin_path(&file).unwrap();
    }

    #[test]
    #[ignore = "manual cold/warm Windows trust attribution; requires private fixture root"]
    fn profile_trust_validation() {
        use std::time::Instant;
        fn measure(name: &str, mut operation: impl FnMut()) {
            let mut samples = Vec::new();
            for _ in 0..50 {
                let start = Instant::now();
                operation();
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let first = samples[0];
            let sum: f64 = samples.iter().sum();
            samples.sort_by(f64::total_cmp);
            println!(
                "TRUST name={name} count=50 total_ms={sum:.3} first_ms={first:.3} median_ms={:.3}",
                samples[25]
            );
        }
        measure("trusted_installer_lookup", || {
            let mut installer = [0u32; 17];
            let mut length = 68;
            let mut domain = [0u16; 256];
            let mut domain_len = 256;
            let mut kind = 0;
            let name: Vec<_> = "NT SERVICE\\TrustedInstaller"
                .encode_utf16()
                .chain(Some(0))
                .collect();
            // SAFETY: local account lookup with terminated name and correctly sized outputs.
            assert_ne!(
                unsafe {
                    LookupAccountNameW(
                        ptr::null(),
                        name.as_ptr(),
                        installer.as_mut_ptr().cast(),
                        &mut length,
                        domain.as_mut_ptr(),
                        &mut domain_len,
                        &mut kind,
                    )
                },
                0,
                "{}",
                io::Error::last_os_error()
            );
        });
        measure("fresh_identity", || {
            std::hint::black_box(Identity::current().unwrap());
        });
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("secure HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        protect_fixture(root.path()).unwrap();
        let file = root.path().join("library.dll");
        std::fs::write(&file, b"fixture").unwrap();
        check(&file).unwrap();
        let identity = Identity::current().unwrap();
        println!("TRUST ancestors={}", file.ancestors().count());
        measure("acl_ancestor_walk", || {
            for (index, path) in file.ancestors().enumerate() {
                check_one(
                    path,
                    &identity,
                    if index == 0 {
                        MODIFY_RIGHTS
                    } else {
                        RENAME_RIGHTS
                    },
                )
                .unwrap();
            }
        });
        measure("complete_check", || check(&file).unwrap());
    }

    #[test]
    fn shared_writable_installation_root_is_rejected() {
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("secure HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        protect_fixture(root.path()).unwrap();
        check(root.path())
            .expect("test installation and ancestors must be private before applying unsafe ACL");
        fixture_acl(root.path(), "D:P(A;OICI;FA;;;WD)").unwrap();
        assert!(check(root.path()).is_err());
        protect_fixture(root.path()).unwrap();
        let file = root.path().join("library.dll");
        std::fs::write(&file, b"fixture").unwrap();
        fixture_acl(&file, "D:P(A;;FA;;;WD)").unwrap();
        assert!(check(&file).is_err());
    }

    #[test]
    fn private_child_does_not_hide_a_replaceable_parent() {
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("secure HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        protect_fixture(root.path()).unwrap();
        let child = root.path().join("private");
        std::fs::create_dir(&child).unwrap();
        protect_fixture(&child).unwrap();
        check(&child).unwrap();
        fixture_acl(root.path(), "D:P(A;OICI;FA;;;WD)").unwrap();
        assert!(check(&child).is_err());
    }
}
