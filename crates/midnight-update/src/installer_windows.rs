//! Windows Installer and process details of the MSI hand-off (`installer`): the product
//! records in msi.dll, the pinned package, `msiexec` (through UAC for a per-machine
//! installation) and the relaunch.
#![allow(unsafe_code)]
use crate::installer::{
    self, Host, INSTANCES_WAIT, InstallRequest, MsiCommand, ProductInfo, Products, Scope,
};
use std::{
    any::Any,
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    ptr, thread,
};
use windows_sys::Win32::{
    Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, WAIT_OBJECT_0},
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, READ_CONTROL,
    },
    System::{
        ApplicationInstallationAndServicing::{
            INSTALLPROPERTY_INSTALLLOCATION, INSTALLPROPERTY_VERSIONSTRING, MSIINSTALLCONTEXT,
            MSIINSTALLCONTEXT_MACHINE, MSIINSTALLCONTEXT_USERUNMANAGED, MsiEnumRelatedProductsW,
            MsiGetProductInfoExW,
        },
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        SystemInformation::GetSystemDirectoryW,
        Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Windows Installer's product records, through msi.dll.
pub struct SystemProducts;

fn context(scope: Scope) -> MSIINSTALLCONTEXT {
    match scope {
        Scope::User => MSIINSTALLCONTEXT_USERUNMANAGED,
        Scope::Machine => MSIINSTALLCONTEXT_MACHINE,
    }
}

/// One property of an installed product in one context; `None` when it is not
/// installed there or has no such property.
fn product_property(product: &[u16], scope: Scope, property: *const u16) -> Option<String> {
    let mut buffer = vec![0u16; 260];
    for _ in 0..3 {
        let mut length = u32::try_from(buffer.len()).ok()?;
        // SAFETY: terminated product code and property name; the buffer holds `length`
        // characters and Windows Installer writes at most that many, terminator included.
        let status = unsafe {
            MsiGetProductInfoExW(
                product.as_ptr(),
                ptr::null(),
                context(scope),
                property,
                buffer.as_mut_ptr(),
                &mut length,
            )
        };
        match status {
            ERROR_SUCCESS => return Some(from_wide(&buffer)),
            ERROR_MORE_DATA => buffer = vec![0u16; length as usize + 1],
            _ => return None,
        }
    }
    None
}

impl Products for SystemProducts {
    fn related(&self, upgrade_code: &str) -> Vec<String> {
        let code = wide(OsStr::new(upgrade_code));
        let mut products = Vec::new();
        // Product codes are 38 characters; a few products at most share the code.
        for index in 0..64 {
            let mut buffer = [0u16; 39];
            // SAFETY: a terminated upgrade code and a 39-character output buffer, as the
            // API requires.
            let status =
                unsafe { MsiEnumRelatedProductsW(code.as_ptr(), 0, index, buffer.as_mut_ptr()) };
            if status != ERROR_SUCCESS {
                break;
            }
            products.push(from_wide(&buffer));
        }
        products
    }

    fn product(&self, product_code: &str, scope: Scope) -> Option<ProductInfo> {
        let product = wide(OsStr::new(product_code));
        let location = product_property(&product, scope, INSTALLPROPERTY_INSTALLLOCATION)?;
        let version = product_property(&product, scope, INSTALLPROPERTY_VERSIONSTRING)?;
        (!location.is_empty()).then(|| ProductInfo {
            location: PathBuf::from(location),
            version,
        })
    }
}

/// The real hand-off steps.
pub struct SystemHost;

/// The package itself (never a link target), shared for reading only: no process can
/// write, rename or delete it while it is open.
fn pin_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// The package's folder (never a link target): it can be neither renamed nor removed
/// while it is open, so the package's path keeps naming the pinned file.
fn pin_folder(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | READ_CONTROL | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn is_link(file: &File) -> io::Result<bool> {
    Ok(file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// Pins `package` and its folder and checks the package's digest through its pin.
pub(crate) fn pin(package: &Path, sha256: &[u8; 32]) -> Result<(File, File), String> {
    let refused = |error: io::Error| format!("The downloaded installer cannot be opened: {error}");
    let folder = package
        .parent()
        .ok_or_else(|| "The downloaded installer has no folder.".to_owned())?;
    let folder = pin_folder(folder).map_err(refused)?;
    let file = pin_file(package).map_err(refused)?;
    if is_link(&folder).map_err(refused)? || is_link(&file).map_err(refused)? {
        return Err("The downloaded installer is a link, not a file.".into());
    }
    installer::check_digest(&file, sha256)?;
    Ok((file, folder))
}

fn system_folder() -> io::Result<PathBuf> {
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer holds the size passed.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..length as usize],
    )))
}

/// Starts `msiexec` through UAC (`runas`) and returns its process. Windows shows the
/// consent prompt; declining fails with `ERROR_CANCELLED`. On its own short-lived STA,
/// because Shell verbs may be delegated to COM extensions.
fn launch_elevated(msiexec: &Path, parameters: &str, folder: &Path) -> io::Result<OwnedHandle> {
    let verb = wide(OsStr::new("runas"));
    let file = wide(msiexec.as_os_str());
    let parameters = wide(OsStr::new(parameters));
    let folder = wide(folder.as_os_str());
    thread::Builder::new()
        .name("installer-launch".into())
        .spawn(move || {
            // SAFETY: this new thread initializes and later uninitializes its own apartment.
            let apartment = unsafe {
                CoInitializeEx(
                    ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                )
            };
            // SAFETY: SHELLEXECUTEINFOW is a plain C structure; all-zero is a valid start.
            let mut launch: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            launch.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
            launch.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
            launch.lpVerb = verb.as_ptr();
            launch.lpFile = file.as_ptr();
            launch.lpParameters = parameters.as_ptr();
            launch.lpDirectory = folder.as_ptr();
            launch.nShow = SW_SHOWNORMAL;
            // SAFETY: every string is terminated and outlives the call.
            let result = if unsafe { ShellExecuteExW(&mut launch) } == 0 {
                Err(io::Error::last_os_error())
            } else if launch.hProcess.is_null() {
                Err(io::Error::other("Windows started no installer process"))
            } else {
                // SAFETY: SEE_MASK_NOCLOSEPROCESS hands the process handle to the caller.
                Ok(unsafe { OwnedHandle::from_raw_handle(launch.hProcess) })
            };
            if apartment >= 0 {
                // SAFETY: balances the successful initialization on this thread.
                unsafe { CoUninitialize() };
            }
            result
        })?
        .join()
        .map_err(|_| io::Error::other("the installer launch failed"))?
}

fn wait_exit(process: &OwnedHandle) -> io::Result<u32> {
    // SAFETY: a live process handle with synchronize and query rights (ShellExecuteEx's).
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
        return Err(io::Error::last_os_error());
    }
    let mut code = 0u32;
    // SAFETY: a live process handle and a valid output.
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(code)
}

impl Host for SystemHost {
    fn wait_for_instances(&self, request: &InstallRequest) -> bool {
        installer::wait_for_instances(&request.private_folder, &request.folder, INSTANCES_WAIT)
            .is_some()
    }

    fn pin(&self, package: &Path, sha256: &[u8; 32]) -> Result<Box<dyn Any>, String> {
        pin(package, sha256).map(|pins| Box::new(pins) as Box<dyn Any>)
    }

    fn run(&self, command: &MsiCommand) -> io::Result<u32> {
        let system = system_folder()?;
        let msiexec = system.join("msiexec.exe");
        if command.elevated {
            let process = launch_elevated(&msiexec, &command.parameters, &system)?;
            wait_exit(&process)
        } else {
            // msiexec reads its own command line, so the parameters go as they are.
            let status = Command::new(&msiexec)
                .raw_arg(&command.parameters)
                .current_dir(&system)
                .status()?;
            status
                .code()
                .map(|code| code as u32)
                .ok_or_else(|| io::Error::other("Windows Installer ended without a status"))
        }
    }

    fn relaunch(
        &self,
        executable: &Path,
        arguments: &[OsString],
        working_folder: &Path,
    ) -> io::Result<()> {
        let folder = if working_folder.is_dir() {
            working_folder
        } else {
            executable.parent().unwrap_or(working_folder)
        };
        Command::new(executable)
            .args(arguments)
            .current_dir(folder)
            .spawn()
            .map(drop)
    }
}
