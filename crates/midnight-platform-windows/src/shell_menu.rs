//! Native Shell menus, entirely owned by the backend STA and its message pump.

use crate::{WindowsFileSystem, desktop::ComGuard};
use midnight_core::{
    FsError, ShellMenuModifiers, ShellMenuOutcome, ShellMenuRequest, ShellMenuService,
    ShellMenuTarget, ShellMenuVerb,
};
use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::{Component, Path},
    ptr,
};
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        System::Com::CoTaskMemFree,
        UI::{
            Shell::{
                CMF_CANRENAME, CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN,
                CMIC_MASK_PTINVOKE, CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO,
                CMINVOKECOMMANDINFOEX, Common::ITEMIDLIST, GCS_VERBA, GCS_VERBW, IContextMenu,
                IShellFolder, SEE_MASK_NOASYNC, SEE_MASK_UNICODE, SHBindToParent,
                SHParseDisplayName,
            },
            WindowsAndMessaging::HMENU,
        },
    },
    core::{PCSTR, PCWSTR, PSTR},
};
use windows_sys::Win32::{
    Foundation::HWND as RawWindow,
    System::Com::COINIT_APARTMENTTHREADED,
    UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, SW_SHOWNORMAL},
};

#[path = "shell_menu_host.rs"]
mod host;
#[cfg(test)]
#[path = "shell_menu_tests.rs"]
mod tests;

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;

impl ShellMenuService for WindowsFileSystem {
    fn show_shell_menu(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        validate_request(request)?;
        host::show(request)
    }
}

fn failure(path: &Path, error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> FsError {
    FsError::from_io("show system menu", path, io::Error::other(error))
}

fn validate_request(request: &ShellMenuRequest) -> Result<(), FsError> {
    request.validate()?;
    if request.target == ShellMenuTarget::Drive
        && request.paths[0]
            .components()
            .any(|component| !matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(failure(
            &request.paths[0],
            "A drive menu requires a native drive root.",
        ));
    }
    Ok(())
}

fn wide_path(path: &Path) -> Result<Vec<u16>, FsError> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if !path.is_absolute() || wide.contains(&0) {
        return Err(failure(
            path,
            "Shell paths must be absolute and contain no NUL.",
        ));
    }
    wide.push(0);
    Ok(wide)
}

struct OwnedPidl(*mut ITEMIDLIST);

impl OwnedPidl {
    fn parse(path: &Path) -> Result<Self, FsError> {
        let wide = wide_path(path)?;
        let mut pidl = Self(ptr::null_mut());
        // SAFETY: COM is initialized on this thread, and both buffers outlive the call.
        unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl.0, 0, None) }
            .map_err(|error| failure(path, error))?;
        if pidl.0.is_null() {
            return Err(failure(
                path,
                "The Windows Shell returned an empty item identity.",
            ));
        }
        Ok(pidl)
    }
}

impl Drop for OwnedPidl {
    fn drop(&mut self) {
        // SAFETY: SHParseDisplayName allocated this PIDL with the COM allocator.
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

fn bind_context_menu(
    owner: RawWindow,
    request: &ShellMenuRequest,
) -> Result<IContextMenu, FsError> {
    validate_request(request)?;
    let items = request
        .paths
        .iter()
        .map(|path| OwnedPidl::parse(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut children = Vec::with_capacity(items.len());
    let mut parent = None;
    for (item, path) in items.iter().zip(&request.paths) {
        let mut child = ptr::null_mut();
        // SAFETY: each absolute PIDL is live. The returned child borrows its last component;
        // all items remain allocated through GetUIObjectOf. Core requires one common parent.
        let folder: IShellFolder = unsafe { SHBindToParent(item.0, Some(&mut child)) }
            .map_err(|error| failure(path, error))?;
        if child.is_null() {
            return Err(failure(
                path,
                "The Windows Shell returned no relative item identity.",
            ));
        }
        if parent.is_none() {
            parent = Some(folder);
        }
        children.push(child.cast_const());
    }
    let parent = parent.ok_or_else(|| failure(Path::new(""), "The menu selection is empty."))?;
    // SAFETY: the parent and every relative child PIDL remain live on the same STA.
    unsafe { parent.GetUIObjectOf(HWND(owner), &children, None) }
        .map_err(|error| failure(&request.paths[0], error))
}

struct PopupMenu(windows_sys::Win32::UI::WindowsAndMessaging::HMENU);

impl PopupMenu {
    fn new() -> Result<Self, FsError> {
        // SAFETY: no arguments; ownership of the returned handle is transferred to this guard.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return Err(failure(Path::new(""), io::Error::last_os_error()));
        }
        Ok(Self(menu))
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        // SAFETY: this guard exclusively owns the menu after tracking has finished.
        unsafe { DestroyMenu(self.0) };
    }
}

fn query_menu(
    menu: &IContextMenu,
    popup: &PopupMenu,
    modifiers: ShellMenuModifiers,
) -> Result<u32, FsError> {
    let flags = CMF_NORMAL
        | CMF_CANRENAME
        | if modifiers.shift {
            CMF_EXTENDEDVERBS
        } else {
            0
        };
    // SAFETY: menu and HMENU are live on this STA. IDs start at 1 so zero means cancel.
    let result =
        unsafe { menu.QueryContextMenu(HMENU(popup.0), 0, FIRST_COMMAND, LAST_COMMAND, flags) };
    result.ok().map_err(|error| failure(Path::new(""), error))?;
    Ok((result.0 as u32) & 0xffff)
}

fn command_offset(command: u32, count: u32) -> Option<u32> {
    command
        .checked_sub(FIRST_COMMAND)
        .filter(|offset| *offset < count && command <= LAST_COMMAND)
}

fn canonical_verb(menu: &IContextMenu, offset: u32) -> Option<ShellMenuVerb> {
    // The buffer bounds canonical command names, not file paths. Prefer Unicode; older
    // extensions may only implement GCS_VERBA. Canonical managed verbs are ASCII either way.
    let mut wide = [0u16; 256];
    // SAFETY: GCS_VERBW defines the nominal PSTR buffer as UTF-16 with cchMax wide units.
    let result = unsafe {
        menu.GetCommandString(
            offset as usize,
            GCS_VERBW,
            None,
            PSTR(wide.as_mut_ptr().cast()),
            256,
        )
    };
    if result.is_ok() {
        let length = wide.iter().position(|unit| *unit == 0)?;
        if let Ok(text) = String::from_utf16(&wide[..length])
            && let Some(verb) = ShellMenuVerb::from_canonical(&text)
        {
            return Some(verb);
        }
    }
    let mut narrow = [0u8; 256];
    // SAFETY: this ANSI buffer has the declared capacity and lives through the synchronous call.
    unsafe {
        menu.GetCommandString(
            offset as usize,
            GCS_VERBA,
            None,
            PSTR(narrow.as_mut_ptr()),
            256,
        )
    }
    .ok()?;
    let length = narrow.iter().position(|byte| *byte == 0)?;
    std::str::from_utf8(&narrow[..length])
        .ok()
        .and_then(ShellMenuVerb::from_canonical)
}

fn invoke_command(
    menu: &IContextMenu,
    owner: RawWindow,
    request: &ShellMenuRequest,
    offset: u32,
) -> Result<ShellMenuOutcome, FsError> {
    if let Some(verb) = canonical_verb(menu, offset) {
        return Ok(ShellMenuOutcome::Managed(verb));
    }
    let directory = if request.target == ShellMenuTarget::Drive {
        request.paths[0].as_path()
    } else {
        request.paths[0].parent().unwrap_or(&request.paths[0])
    };
    let wide_directory = wide_path(directory)?;
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: SEE_MASK_UNICODE
            | SEE_MASK_NOASYNC
            | CMIC_MASK_PTINVOKE
            | if request.modifiers.shift {
                CMIC_MASK_SHIFT_DOWN
            } else {
                0
            }
            | if request.modifiers.control {
                CMIC_MASK_CONTROL_DOWN
            } else {
                0
            },
        hwnd: HWND(owner),
        // MAKEINTRESOURCE command offsets are explicitly supported by InvokeCommand.
        lpVerb: PCSTR(offset as usize as *const u8),
        lpVerbW: PCWSTR(offset as usize as *const u16),
        lpDirectoryW: PCWSTR(wide_directory.as_ptr()),
        nShow: SW_SHOWNORMAL,
        ptInvoke: POINT {
            x: request.position.x,
            y: request.position.y,
        },
        ..Default::default()
    };
    // SAFETY: the extended structure, directory string and interface remain live on this
    // STA through invocation. The permanent host continues pumping after it returns.
    unsafe {
        menu.InvokeCommand((&info as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>())
    }
    .map_err(|error| failure(&request.paths[0], error))?;
    Ok(ShellMenuOutcome::Invoked)
}
