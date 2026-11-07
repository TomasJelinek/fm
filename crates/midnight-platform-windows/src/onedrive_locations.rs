//! Optional OneDrive account registrations; no shell execution or registry writes.
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, ptr};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS},
        System::Registry::*,
    },
    core::w,
};

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: this guard owns one successfully opened registry handle.
        unsafe { RegCloseKey(self.0) };
    }
}

pub(crate) fn accounts() -> Vec<(String, String, PathBuf)> {
    let mut raw = ptr::null_mut();
    // SAFETY: fixed, NUL-terminated subkey and valid output; read-only current-user access.
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\OneDrive\\Accounts"),
            0,
            KEY_READ,
            &mut raw,
        )
    };
    if result != ERROR_SUCCESS {
        return Vec::new();
    }
    let key = Key(raw);
    let mut accounts = Vec::new();
    // A registry key component has a documented 255-character limit; this is not a path limit.
    for index in 0..128 {
        let mut name = [0_u16; 256];
        let mut length = name.len() as u32;
        // SAFETY: the handle remains live and the writable name buffer length matches its allocation.
        let result = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if result != ERROR_SUCCESS {
            break;
        }
        let Ok(account) = String::from_utf16(&name[..length as usize]) else {
            continue;
        };
        if !valid_account(&account) {
            continue;
        }
        name[length as usize] = 0;
        let Some(path) = value(&key, &name, w!("UserFolder"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_dir())
        else {
            continue;
        };
        let display = value(&key, &name, w!("DisplayName"))
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| account.clone());
        accounts.push((account, display, path));
    }
    accounts.sort_by(|a, b| a.0.cmp(&b.0));
    accounts
}

fn valid_account(account: &str) -> bool {
    account == "Personal"
        || account.strip_prefix("Business").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn value(key: &Key, subkey: &[u16], name: *const u16) -> Option<OsString> {
    let mut bytes = 0_u32;
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_ZEROONFAILURE;
    // SAFETY: live key, terminated names and a valid size pointer; null data requests size only.
    let result = unsafe {
        RegGetValueW(
            key.0,
            subkey.as_ptr(),
            name,
            flags,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    for _ in 0..3 {
        // Bound a corrupt configuration value's allocation, independently of native path length.
        if !(2..=4 * 1024 * 1024).contains(&bytes) || !bytes.is_multiple_of(2) {
            return None;
        }
        let mut units = vec![0_u16; bytes as usize / 2];
        // SAFETY: bytes describes the live writable buffer; RegGetValueW terminates string output.
        let result = unsafe {
            RegGetValueW(
                key.0,
                subkey.as_ptr(),
                name,
                flags,
                ptr::null_mut(),
                units.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if result == ERROR_MORE_DATA {
            continue;
        }
        if result != ERROR_SUCCESS || !bytes.is_multiple_of(2) || bytes as usize / 2 > units.len() {
            return None;
        }
        units.truncate(bytes as usize / 2);
        if units.pop() != Some(0) || units.contains(&0) {
            return None;
        }
        return Some(OsString::from_wide(&units));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_native_account_key_names_are_read() {
        for name in ["Personal", "Business1", "Business42"] {
            assert!(valid_account(name));
        }
        for name in [
            "",
            "Business",
            "Business../x",
            "Personal\\Other",
            "Business🦀",
        ] {
            assert!(!valid_account(name));
        }
    }
}
