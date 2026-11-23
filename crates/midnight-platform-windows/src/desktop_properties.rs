//! Native property sheets. One item opens through `SHObjectProperties` on the desktop
//! service's bounded STA worker. Several items open in the Shell menu's
//! process-lifetime apartment (`shell_menu::run_task`), as the system menu's own
//! Properties command does: the Shell shows that sheet on a thread of its own, which
//! reads the selection back from the apartment that built it for as long as the sheet
//! is open. A worker apartment that ended with the request left the sheet without its
//! items ("The properties for this item are not available", titled "Desktop").

use super::ComGuard;
use crate::WindowsFileSystem;
use midnight_core::{FsError, PropertyService};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{self, NonNull},
};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{
            Com::{CoTaskMemFree, IDataObject},
            SystemServices::SFGAO_FILESYSTEM,
        },
        UI::Shell::{
            CIDLData_CreateFromIDArray, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
            Common::ITEMIDLIST, IContextMenu, ILFindLastID, IShellFolder, SEE_MASK_UNICODE,
            SHBindToObject, SHMultiFileProperties, SHParseDisplayName,
        },
    },
    core::{PCSTR, PCWSTR, w},
};
use windows_sys::Win32::{
    Foundation::HWND as RawWindow,
    System::Com::COINIT_APARTMENTTHREADED,
    UI::{
        Shell::{SHOP_FILEPATH, SHObjectProperties},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

#[cfg(test)]
#[path = "desktop_properties_tests.rs"]
mod tests;

impl PropertyService for WindowsFileSystem {
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError> {
        let Some(selection) = property_selection(paths) else {
            return Ok(false);
        };
        if let [wide] = selection.items.as_slice() {
            let Some(_com) = ComGuard::initialize(COINIT_APARTMENTTHREADED as u32) else {
                return Ok(false);
            };
            // SAFETY: the Unicode path is NUL-terminated and live through the call. The
            // Shell reads only this string, on a thread of its own.
            return Ok(unsafe {
                SHObjectProperties(
                    ptr::null_mut(),
                    SHOP_FILEPATH as u32,
                    wide.0.as_ptr(),
                    ptr::null(),
                )
            } != 0);
        }
        crate::shell_menu::run_task("properties", &paths[0], move |owner| {
            show_several(owner, &selection)
        })
    }
}

/// The selected items as the Shell parses them, and their common folder if they
/// share one, with each item's name in it (`names`, empty without a folder).
struct PropertySelection {
    items: Vec<PropertyPath>,
    folder: Option<PropertyPath>,
    names: Vec<PropertyPath>,
}

/// A NUL-terminated UTF-16 path.
struct PropertyPath(Vec<u16>);

/// None when any item is unusable, so the whole selection gets the application's own
/// summary instead.
fn property_selection(paths: &[PathBuf]) -> Option<PropertySelection> {
    if paths.is_empty() || u32::try_from(paths.len()).is_err() {
        return None;
    }
    let spelled = paths
        .iter()
        .map(|path| path.is_absolute().then(|| shell_spelling(path))?)
        .collect::<Option<Vec<_>>>()?;
    let parent = |path: &PathBuf| path.parent().map(Path::to_path_buf);
    let (folder, names) = parent(&spelled[0])
        .filter(|folder| {
            spelled
                .iter()
                .all(|path| parent(path).as_ref() == Some(folder))
        })
        .and_then(|folder| {
            let names = spelled
                .iter()
                .map(|path| terminated(Path::new(path.file_name()?)))
                .collect::<Option<Vec<_>>>()?;
            Some((terminated(&folder)?, names))
        })
        .unzip();
    let items = spelled
        .iter()
        .map(|path| terminated(path))
        .collect::<Option<_>>()?;
    Some(PropertySelection {
        items,
        folder,
        names: names.unwrap_or_default(),
    })
}

fn terminated(path: &Path) -> Option<PropertyPath> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    Some(PropertyPath(wide))
}

/// The spelling the Shell parses. The Shell does not parse verbatim paths, so
/// `\\?\C:\…` and `\\?\UNC\server\share\…` lose their prefix when the plain spelling
/// names the same item: no empty, `.` or `..` component, none ending in a dot or space
/// and no DOS device name, all of which Win32 would resolve differently; None for
/// any other such drive or UNC path. Other paths are kept as given; one the Shell
/// cannot parse leaves the selection to the application's summary.
fn shell_spelling(path: &Path) -> Option<PathBuf> {
    const VERBATIM: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    let Some(rest) = units.strip_prefix(&VERBATIM[..]) else {
        return Some(path.to_path_buf());
    };
    let ascii = |unit: u16| {
        u8::try_from(unit)
            .ok()
            .map(|byte| byte.to_ascii_uppercase())
    };
    let (plain, components) = match rest {
        [letter, colon, separator, components @ ..]
            if ascii(*letter).is_some_and(|byte| byte.is_ascii_alphabetic())
                && *colon == u16::from(b':')
                && *separator == u16::from(b'\\') =>
        {
            (rest.to_vec(), components)
        }
        [u, n, c, separator, components @ ..]
            if [*u, *n, *c].map(ascii) == [Some(b'U'), Some(b'N'), Some(b'C')]
                && *separator == u16::from(b'\\') =>
        {
            let mut plain = VERBATIM[..2].to_vec();
            plain.extend_from_slice(components);
            (plain, components)
        }
        _ => return Some(path.to_path_buf()),
    };
    let mut names: Vec<&[u16]> = components.split(|unit| *unit == u16::from(b'\\')).collect();
    // A trailing separator ends a folder's spelling.
    if names.len() > 1 && names.last().is_some_and(|name| name.is_empty()) {
        names.pop();
    }
    let stable = |name: &[u16]| {
        !name.is_empty()
            && !name.contains(&u16::from(b'/'))
            && !matches!(name.last(), Some(&unit) if unit == u16::from(b'.') || unit == u16::from(b' '))
            && !is_device_name(name)
    };
    let root_only = names.len() == 1 && names[0].is_empty();
    (root_only || names.iter().all(|name| stable(name)))
        .then(|| PathBuf::from(OsString::from_wide(&plain)))
}

/// A DOS device name, with or without an extension (`NUL`, `com1.txt`).
fn is_device_name(name: &[u16]) -> bool {
    let stem = name
        .split(|unit| *unit == u16::from(b'.'))
        .next()
        .unwrap_or_default();
    let stem = String::from_utf16_lossy(stem).to_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix).is_some_and(|number| {
            let mut chars = number.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT")
}

/// Runs in the Shell menu's apartment. Items of one folder get that folder's own
/// Properties command, as its system menu and Explorer do; items of several folders
/// (search results, an expanded tree) get the Shell's combined sheet for absolute
/// items, which reports "Various Folders". False when the Shell cannot parse an item.
fn show_several(owner: RawWindow, selection: &PropertySelection) -> bool {
    let Some(items) = selection
        .items
        .iter()
        .map(PropertyItem::parse)
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    if let Some(folder) = &selection.folder
        && let Ok(menu) = folder_menu(owner, folder, &selection.names)
    {
        return invoke_properties(&menu, owner, folder);
    }
    let Some(data) = property_data_object(&items) else {
        return false;
    };
    // SAFETY: data supplies the complete Shell ID list and lives in this apartment,
    // which outlives the sheet.
    unsafe { SHMultiFileProperties(&data, 0) }.is_ok()
}

/// The context menu of `folder` for the items it holds as `names`, or the step that
/// failed. The folder parses each name into its own child ID, so every item is its
/// child by construction. Absolute IDs of separately parsed paths are not compared:
/// their IDs of the same ancestor folder can differ byte for byte, and whether the
/// Shell still finds them equal depends on its environment.
fn folder_menu(
    owner: RawWindow,
    folder: &PropertyPath,
    names: &[PropertyPath],
) -> Result<IContextMenu, String> {
    if names.is_empty() {
        return Err("no items".into());
    }
    let parsed = PropertyItem::parse(folder).ok_or("the folder is no file system folder")?;
    // SAFETY: parsed is a live absolute PIDL; binding starts at the desktop.
    let shell: IShellFolder = unsafe { SHBindToObject(None, parsed.0.as_ptr(), None) }
        .map_err(|error| format!("binding the folder: {error}"))?;
    let mut children = Vec::with_capacity(names.len());
    for name in names {
        let mut child = ptr::null_mut();
        // SAFETY: the name is NUL-terminated and live; the folder writes one relative
        // PIDL, owned by the guard below, to a valid output.
        unsafe {
            shell.ParseDisplayName(
                HWND(owner),
                None,
                PCWSTR(name.0.as_ptr()),
                None,
                &mut child,
                ptr::null_mut(),
            )
        }
        .map_err(|error| format!("parsing an item's name: {error}"))?;
        let child = PropertyItem(NonNull::new(child).ok_or("the folder parsed no item")?);
        // SAFETY: child is a live PIDL; the result borrows its last ID.
        if unsafe { ILFindLastID(child.0.as_ptr()) } != child.0.as_ptr() {
            return Err("an item's name reaches into a subfolder".into());
        }
        children.push(child);
    }
    let ids: Vec<_> = children
        .iter()
        .map(|child| child.0.as_ptr().cast_const())
        .collect();
    // SAFETY: the folder and every child ID stay live through the call on this STA.
    unsafe { shell.GetUIObjectOf(HWND(owner), &ids, None) }
        .map_err(|error| format!("the folder's menu: {error}"))
}

fn invoke_properties(menu: &IContextMenu, owner: RawWindow, folder: &PropertyPath) -> bool {
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: SEE_MASK_UNICODE,
        hwnd: HWND(owner),
        lpVerb: PCSTR(c"properties".as_ptr().cast()),
        lpVerbW: w!("properties"),
        lpDirectoryW: PCWSTR(folder.0.as_ptr()),
        nShow: SW_SHOWNORMAL,
        ..Default::default()
    };
    // SAFETY: the structure, its strings and the menu are live on this STA through the
    // call. The permanent apartment and its owner window outlive the sheet it opens.
    unsafe { menu.InvokeCommand((&raw const info).cast::<CMINVOKECOMMANDINFO>()) }.is_ok()
}

fn property_data_object(items: &[PropertyItem]) -> Option<IDataObject> {
    let ids: Vec<_> = items
        .iter()
        .map(|item| item.0.as_ptr().cast_const())
        .collect();
    let desktop = ITEMIDLIST::default();
    // SAFETY: the empty desktop PIDL lets the Shell copy absolute IDs from all selected
    // folders into the data object. The input array and all PIDLs live through the call.
    unsafe { CIDLData_CreateFromIDArray(&desktop, Some(&ids)) }.ok()
}

/// A Shell item ID list owned by this process: absolute, or relative to the folder that
/// parsed it (`folder_menu`).
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

    /// A copy of `item` without its last ID: its folder.
    #[cfg(test)]
    fn parent_of(item: &Self) -> Option<Self> {
        use windows::Win32::UI::Shell::{ILClone, ILRemoveLastID};
        // SAFETY: item is live; ILClone returns a new COM allocation owned by the result.
        let parent = Self(NonNull::new(unsafe { ILClone(item.0.as_ptr()) })?);
        // SAFETY: parent is this process's own copy, shortened in place.
        unsafe { ILRemoveLastID(Some(parent.0.as_ptr())) }
            .as_bool()
            .then_some(parent)
    }
}

impl Drop for PropertyItem {
    fn drop(&mut self) {
        // SAFETY: SHParseDisplayName and ILClone transferred this COM allocation.
        unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
    }
}
