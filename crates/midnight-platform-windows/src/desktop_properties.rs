//! Native property sheets on the desktop service's bounded STA worker.

use super::ComGuard;
use crate::WindowsFileSystem;
use midnight_core::{FsError, PropertyService};
use std::{
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    ptr::{self, NonNull},
};
use windows::{
    Win32::{
        System::{
            Com::{CoTaskMemFree, IDataObject},
            SystemServices::SFGAO_FILESYSTEM,
        },
        UI::Shell::{
            CIDLData_CreateFromIDArray, Common::ITEMIDLIST, SHMultiFileProperties,
            SHParseDisplayName,
        },
    },
    core::PCWSTR,
};
use windows_sys::Win32::{
    System::Com::COINIT_APARTMENTTHREADED,
    UI::Shell::{SHOP_FILEPATH, SHObjectProperties},
};

#[cfg(test)]
#[path = "desktop_properties_tests.rs"]
mod tests;

impl PropertyService for WindowsFileSystem {
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError> {
        let Some(selection) = property_selection(paths) else {
            return Ok(false);
        };
        let Some(_com) = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32) else {
            return Ok(false);
        };
        if let [wide] = selection.as_slice() {
            // SAFETY: the Unicode path is NUL-terminated and live through the call.
            return Ok(unsafe {
                SHObjectProperties(
                    ptr::null_mut(),
                    SHOP_FILEPATH as u32,
                    wide.0.as_ptr(),
                    ptr::null(),
                )
            } != 0);
        }
        let Some(data) = property_data_object(&selection) else {
            return Ok(false);
        };
        // SAFETY: data supplies the complete Shell ID list and is live on this STA.
        Ok(unsafe { SHMultiFileProperties(&data, 0) }.is_ok())
    }
}

struct PropertyPath(Vec<u16>);

fn property_selection(paths: &[PathBuf]) -> Option<Vec<PropertyPath>> {
    if paths.is_empty() || u32::try_from(paths.len()).is_err() {
        return None;
    }
    paths
        .iter()
        .map(|path| {
            if !path.is_absolute() {
                return None;
            }
            let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
            if wide.contains(&0) {
                return None;
            }
            wide.push(0);
            Some(PropertyPath(wide))
        })
        .collect()
}

fn property_data_object(selection: &[PropertyPath]) -> Option<IDataObject> {
    let items = selection
        .iter()
        .map(PropertyItem::parse)
        .collect::<Option<Vec<_>>>()?;
    let ids: Vec<_> = items
        .iter()
        .map(|item| item.0.as_ptr().cast_const())
        .collect();
    let desktop = ITEMIDLIST::default();
    // SAFETY: the empty desktop PIDL lets the Shell copy absolute IDs from all selected
    // folders into the data object. The input array and all PIDLs live through the call.
    unsafe { CIDLData_CreateFromIDArray(&desktop, Some(&ids)) }.ok()
}

struct PropertyItem(NonNull<ITEMIDLIST>);

impl PropertyItem {
    fn parse(wide: &PropertyPath) -> Option<Self> {
        let mut raw = ptr::null_mut();
        let mut attributes = 0;
        // SAFETY: the prepared selection supplies a live NUL-terminated path. The
        // Shell writes the absolute PIDL and requested attributes to valid outputs.
        unsafe {
            SHParseDisplayName(
                PCWSTR(wide.0.as_ptr()),
                None,
                &mut raw,
                SFGAO_FILESYSTEM.0,
                Some(&mut attributes),
            )
        }
        .ok()?;
        let item = Self(NonNull::new(raw)?);
        (attributes & SFGAO_FILESYSTEM.0 != 0).then_some(item)
    }
}

impl Drop for PropertyItem {
    fn drop(&mut self) {
        // SAFETY: a successful SHParseDisplayName transferred this COM allocation.
        unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
    }
}
