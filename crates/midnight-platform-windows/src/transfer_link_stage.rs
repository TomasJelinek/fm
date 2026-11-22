//! Windows link stages. A link is created under a private `.me-<16 hex>` name in the
//! destination folder, held through a no-follow handle, read back from that handle and
//! published by a no-replace rename to its final name, as a copied file is.
//!
//! A junction is created as an empty folder with mount-point reparse data; that needs no
//! privilege, but only an absolute path on a local drive can be its target. A symbolic
//! link is created with `CreateSymbolicLinkW` and `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_
//! CREATE`, which succeeds in Developer Mode or with the symbolic-link privilege that an
//! elevated process holds. Without either, a directory symbolic link to a local folder
//! becomes a junction to the same folder (a relative target resolved from the link's
//! destination folder), and the job details say so; a symbolic link to a file or to a
//! network share cannot be recreated and is reported. The target text is never opened.
use super::*;
use midnight_core::{
    StagedTransferLink, TransferLinkCleanupError, TransferLinkKind, TransferLinkPlan,
    TransferLinkRepresentation, TransferLinkStageError, TransferRecovery,
};
use std::path::{Component, Prefix};
use windows::Win32::System::SystemServices::FILE_SUPPORTS_REPARSE_POINTS;
use windows_sys::Win32::{
    Foundation::{ERROR_PRIVILEGE_NOT_HELD, HANDLE, LUID},
    Security::{GetTokenInformation, LookupPrivilegeValueW, TOKEN_QUERY, TokenPrivileges},
    Storage::FileSystem::{
        CreateSymbolicLinkW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
        SYMBOLIC_LINK_FLAG_DIRECTORY,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::FSCTL_SET_REPARSE_POINT,
        Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

const SEPARATOR: u16 = b'\\' as u16;
/// `\??\`, the prefix of an absolute NT path in a link's substitute name.
const NT_PREFIX: [u16; 4] = [SEPARATOR, 63, 63, SEPARATOR];

/// What planning a link reads from the system, read once per job: whether this process may
/// create symbolic links (the Developer Mode setting and the token's privilege), and each
/// destination volume's facts, keyed by its serial number. The destination folder a job
/// starts from holds it, and every folder the job creates or merges into below it shares
/// it (`Directory::below`).
#[derive(Default)]
pub(in crate::transfer) struct LinkFacts {
    symbolic_links: std::sync::OnceLock<bool>,
    volumes: std::sync::Mutex<Vec<(u64, VolumeLinks)>>,
}

/// A destination volume's link facts.
#[derive(Clone)]
struct VolumeLinks {
    network: bool,
    reparse_points: bool,
    filesystem: String,
}

#[cfg(test)]
thread_local! {
    /// How often this thread read link facts from the system, whichever job's folder
    /// asked: (symbolic-link rights, volume facts).
    static LINK_FACT_READS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Counts one read of the system's link facts on this thread (tests).
#[cfg(test)]
fn count_read(symbolic_links: usize, volumes: usize) {
    LINK_FACT_READS.with(|reads| {
        let (known, read) = reads.get();
        reads.set((known + symbolic_links, read + volumes));
    });
}

impl LinkFacts {
    fn symbolic_links_allowed(&self) -> bool {
        *self.symbolic_links.get_or_init(|| {
            #[cfg(test)]
            count_read(1, 0);
            symbolic_links_allowed()
        })
    }

    /// The facts of the volume `serial` that holds `directory`, read once. A query that
    /// fails is not kept: the next link asks again.
    fn volume(&self, serial: u64, directory: &Directory) -> Result<VolumeLinks, FsError> {
        let mut volumes = self.volumes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, facts)) = volumes.iter().find(|(known, _)| *known == serial) {
            return Ok(facts.clone());
        }
        #[cfg(test)]
        count_read(0, 1);
        // A network location says so before anything is asked of its volume.
        let facts = if crate::storage_kind::network(&directory.path) {
            VolumeLinks {
                network: true,
                reparse_points: false,
                filesystem: String::new(),
            }
        } else {
            let (_, flags, filesystem) =
                super::super::link_destination::volume_facts(&directory.file)
                    .map_err(|e| error(&directory.path, e))?;
            VolumeLinks {
                network: false,
                reparse_points: flags & FILE_SUPPORTS_REPARSE_POINTS != 0,
                filesystem,
            }
        };
        volumes.push((serial, facts.clone()));
        Ok(facts)
    }
}

/// Why a folder on a volume with these facts creates no links (`creates_no_links`): a
/// network location, where a junction's target would be resolved on the server and a
/// symbolic link's by each client's policy, or a filesystem without reparse points.
fn no_links_reason(network: bool, reparse_points: bool, filesystem: &str) -> Option<String> {
    if network {
        Some("no links are created at a network location".into())
    } else if !reparse_points {
        Some(format!("this drive cannot store links ({filesystem})"))
    } else {
        None
    }
}

impl Directory {
    /// Why this folder creates no links at all (`TransferDirectory::creates_no_links`).
    pub(super) fn no_links_reason(&self, cancel: &Cancellation) -> Result<Option<String>, FsError> {
        checked(cancel, &self.path)?;
        let volume = self.links.volume(self.identity.device, self)?;
        Ok(no_links_reason(
            volume.network,
            volume.reparse_points,
            &volume.filesystem,
        ))
    }

    /// The link this folder creates for `representation` (`TransferDirectory::plan_link`).
    pub(super) fn plan_link_representation(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferLinkPlan, FsError> {
        checked(cancel, &self.path)?;
        self.revalidate(cancel)?;
        // A transfer asks `creates_no_links` first and leaves such links out; this refusal
        // only guards other callers.
        if let Some(mut reason) = self.no_links_reason(cancel)? {
            reason[..1].make_ascii_uppercase();
            return Err(unsupported(
                &self.path,
                &format!("{reason}, so this link is left out"),
            ));
        }
        let (substitute, _) = representation
            .windows_names()
            .ok_or_else(|| unsupported(&self.path, "Windows recreates only Windows links"))?;
        let same = || TransferLinkPlan {
            representation: representation.clone(),
            note: None,
        };
        match representation.kind() {
            TransferLinkKind::WindowsJunction => {
                if !super::super::source_links::known_local_name(substitute, 0) {
                    return Err(unsupported(
                        &self.path,
                        "a junction can point only to a folder on a local drive",
                    ));
                }
                Ok(same())
            }
            kind @ (TransferLinkKind::WindowsDirectorySymbolicLink
            | TransferLinkKind::WindowsFileSymbolicLink) => {
                let target = symbolic_target(representation, &self.path)?;
                if self.links.symbolic_links_allowed() {
                    // `CreateSymbolicLinkW` writes the target as given for its print name.
                    let planned = TransferLinkRepresentation::windows_symbolic_link(
                        kind == TransferLinkKind::WindowsDirectorySymbolicLink,
                        representation.meaningful_flags(),
                        substitute,
                        &target,
                    )
                    .map_err(|mut failure| {
                        failure.path = self.path.clone();
                        failure
                    })?;
                    return Ok(TransferLinkPlan {
                        representation: planned,
                        note: None,
                    });
                }
                if kind == TransferLinkKind::WindowsFileSymbolicLink {
                    return Err(unsupported(
                        &self.path,
                        "Creating a symbolic link to a file needs Developer Mode or \
                         administrator rights, and a junction can point only to a folder",
                    ));
                }
                let junction = self.junction_for(representation, substitute)?;
                let (_, print) = junction.windows_names().unwrap_or_default();
                let note = format!(
                    "was created as a junction to {}, because creating a symbolic link needs \
                     Developer Mode or administrator rights{}",
                    String::from_utf16_lossy(print),
                    if representation.meaningful_flags() & 1 != 0 {
                        "; its relative target became this absolute one"
                    } else {
                        ""
                    }
                );
                Ok(TransferLinkPlan {
                    representation: junction,
                    note: Some(note),
                })
            }
            TransferLinkKind::LinuxSymbolicLink => Err(unsupported(
                &self.path,
                "Windows recreates only Windows links",
            )),
        }
    }

    /// The junction that stands for a directory symbolic link that may not be created:
    /// to the same local folder, a relative target resolved from this folder.
    fn junction_for(
        &self,
        representation: &TransferLinkRepresentation,
        substitute: &[u16],
    ) -> Result<TransferLinkRepresentation, FsError> {
        let refused = || {
            unsupported(
                &self.path,
                "Its target is not on a local drive: a junction cannot point there, and \
                 creating a symbolic link needs Developer Mode or administrator rights",
            )
        };
        let absolute: Vec<u16> = if representation.meaningful_flags() & 1 == 0 {
            if !super::super::source_links::known_local_name(substitute, 0) {
                return Err(refused());
            }
            substitute[NT_PREFIX.len()..].to_vec()
        } else {
            resolve_relative(&self.path, substitute).ok_or_else(refused)?
        };
        let nt: Vec<u16> = NT_PREFIX
            .iter()
            .copied()
            .chain(absolute.iter().copied())
            .collect();
        if !super::super::source_links::known_local_name(&nt, 0) {
            return Err(refused());
        }
        TransferLinkRepresentation::windows_junction(&nt, &absolute).map_err(|mut failure| {
            failure.path = self.path.clone();
            failure
        })
    }

    /// Creates the owned stage of the planned link `representation`.
    pub(super) fn create_link_stage(
        &self,
        representation: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<Box<dyn StagedTransferLink>, TransferLinkStageError> {
        let refused = |error| TransferLinkStageError { error, stage: None };
        if representation.windows_names().is_none() {
            return Err(refused(unsupported(
                &self.path,
                "Windows recreates only Windows links",
            )));
        }
        self.revalidate(cancel).map_err(refused)?;
        for _ in 0..16 {
            checked(cancel, &self.path).map_err(refused)?;
            let name = private_name(&self.path).map_err(refused)?;
            let path = self.path.join(&name);
            let created = super::super::namespace::change(cancel, |mutation| {
                let file = mutation
                    .create(
                        &self.file,
                        &self.path,
                        &name,
                        super::super::namespace::Creation::Private(EntryKind::Symlink),
                        cancel,
                        || create_native(&self.file, &self.path, &name, representation),
                    )
                    .map_err(|e| created_error(&path, e))?;
                let owner = super::super::namespace::owner_for(&file, &path);
                Ok((file, owner))
            });
            match created {
                Ok((file, owner)) => {
                    let identity = identity(&file, &path).map_err(refused)?;
                    return Ok(Box::new(LinkStage {
                        directory: self.clone(),
                        file: Some(file),
                        identity,
                        name,
                        path,
                        published: None,
                        refused: false,
                        _namespace_owner: owner,
                    }));
                }
                Err(failure) if failure.kind == io::ErrorKind::AlreadyExists => continue,
                Err(failure) => return Err(refused(failure)),
            }
        }
        Err(refused(error(
            &self.path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "exclusive link staging names collided repeatedly",
            ),
        )))
    }
}

/// A private stage name, `.me-` and sixteen random hex digits, as file stages have.
fn private_name(path: &Path) -> Result<OsString, FsError> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(|e| error(path, io::Error::other(e.to_string())))?;
    let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(OsString::from(format!(".me-{suffix}")))
}

/// A creation failure in words: a missing privilege is said so, not with the system's text.
fn created_error(path: &Path, failure: io::Error) -> FsError {
    if failure.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD as i32) {
        return unsupported(
            path,
            "Creating a symbolic link needs Developer Mode or administrator rights",
        );
    }
    error(path, failure)
}

/// Creates the link `name` in `parent` (at `parent_path`) and returns its no-follow handle
/// with DELETE access, denying others delete sharing.
fn create_native(
    parent: &File,
    parent_path: &Path,
    name: &OsStr,
    representation: &TransferLinkRepresentation,
) -> io::Result<File> {
    use windows_sys::Wdk::Storage::FileSystem::{FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE};
    let (substitute, print) = representation
        .windows_names()
        .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))?;
    match representation.kind() {
        TransferLinkKind::WindowsJunction => {
            let folder = native::open_relative(
                parent,
                name,
                FILE_READ_ATTRIBUTES
                    | FILE_WRITE_DATA
                    | FILE_WRITE_ATTRIBUTES
                    | DELETE
                    | SYNCHRONIZE,
                FILE_SHARE_READ,
                FILE_CREATE,
                FILE_DIRECTORY_FILE,
            )?;
            if let Err(failure) = set_junction(&folder, substitute, print) {
                // Nothing but the empty folder this call created: remove it again.
                let _ = native::remove(&folder);
                return Err(failure);
            }
            Ok(folder)
        }
        kind @ (TransferLinkKind::WindowsDirectorySymbolicLink
        | TransferLinkKind::WindowsFileSymbolicLink) => {
            let directory = kind == TransferLinkKind::WindowsDirectorySymbolicLink;
            let link = verbatim(&parent_path.join(name))?;
            let target: Vec<u16> = print.iter().copied().chain(Some(0)).collect();
            let flags = SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE
                | if directory {
                    SYMBOLIC_LINK_FLAG_DIRECTORY
                } else {
                    0
                };
            // SAFETY: both strings are NUL-terminated UTF-16 that live through the call;
            // the private name was checked absent by the namespace turn just before.
            if !unsafe { CreateSymbolicLinkW(link.as_ptr(), target.as_ptr(), flags) } {
                return Err(io::Error::last_os_error());
            }
            native::open_relative(
                parent,
                name,
                FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
                FILE_SHARE_READ,
                FILE_OPEN,
                if directory {
                    FILE_DIRECTORY_FILE
                } else {
                    FILE_NON_DIRECTORY_FILE
                },
            )
        }
        TransferLinkKind::LinuxSymbolicLink => Err(io::ErrorKind::Unsupported.into()),
    }
}

/// `path` as a NUL-terminated verbatim (`\\?\`) path, which has no length limit. The system
/// does not parse a verbatim path, so the `/` separators Windows accepts elsewhere become
/// `\` first (a `/` inside a verbatim name is invalid).
fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    match path.components().next() {
        Some(Component::Prefix(prefix)) if prefix.kind().is_verbatim() => {}
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => {
            for unit in &mut wide {
                if *unit == u16::from(b'/') {
                    *unit = SEPARATOR;
                }
            }
            wide.splice(0..0, r"\\?\".encode_utf16());
        }
        _ => return Err(io::ErrorKind::InvalidInput.into()),
    }
    wide.push(0);
    Ok(wide)
}

/// Writes mount-point reparse data (a junction) with these exact names on `folder`.
fn set_junction(folder: &File, substitute: &[u16], print: &[u16]) -> io::Result<()> {
    let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "junction target is too long");
    let substitute_bytes = u16::try_from(substitute.len() * 2).map_err(|_| too_long())?;
    let print_bytes = u16::try_from(print.len() * 2).map_err(|_| too_long())?;
    let names = u16::try_from((substitute.len() + print.len() + 2) * 2).map_err(|_| too_long())?;
    let length = names.checked_add(8).ok_or_else(too_long)?;
    let mut buffer = Vec::with_capacity(8 + usize::from(length));
    buffer.extend_from_slice(&super::super::source_links::MOUNT_POINT.to_le_bytes());
    buffer.extend_from_slice(&length.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&substitute_bytes.to_le_bytes());
    buffer.extend_from_slice(&(substitute_bytes + 2).to_le_bytes());
    buffer.extend_from_slice(&print_bytes.to_le_bytes());
    for unit in substitute.iter().chain(&[0]).chain(print).chain(&[0]) {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    let mut returned = 0u32;
    // SAFETY: a live synchronous handle with write access to the folder this call created;
    // the input buffer is initialized for its stated length and no output is requested.
    if unsafe {
        DeviceIoControl(
            folder.as_raw_handle(),
            FSCTL_SET_REPARSE_POINT,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The target text a symbolic link is created with: its relative text, a local drive's
/// path (`C:\...`) or a network share's (`\\server\share\...`).
fn symbolic_target(
    representation: &TransferLinkRepresentation,
    path: &Path,
) -> Result<Vec<u16>, FsError> {
    let (substitute, _) = representation
        .windows_names()
        .ok_or_else(|| unsupported(path, "Windows recreates only Windows links"))?;
    if representation.meaningful_flags() & 1 != 0 {
        return Ok(substitute.to_vec());
    }
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, SEPARATOR];
    match substitute.strip_prefix(&NT_PREFIX[..]) {
        Some(rest) if rest.starts_with(&UNC) => Ok([SEPARATOR, SEPARATOR]
            .into_iter()
            .chain(rest[UNC.len()..].iter().copied())
            .collect()),
        Some(rest) => Ok(rest.to_vec()),
        None => Err(unsupported(path, "the link's target is not a path")),
    }
}

/// A relative link text resolved name by name from the folder `base` (a local drive's
/// path), never above the drive's root; None when it cannot be.
fn resolve_relative(base: &Path, relative: &[u16]) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut components = base.components();
    let drive = match components.next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return None,
        },
        _ => return None,
    };
    if components.next()? != Component::RootDir {
        return None;
    }
    let mut names: Vec<Vec<u16>> = components
        .map(|component| match component {
            Component::Normal(name) => Some(name.encode_wide().collect()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    for name in relative.split(|unit| *unit == SEPARATOR || *unit == u16::from(b'/')) {
        match name {
            [] | [46] => {}
            [46, 46] => {
                names.pop()?;
            }
            name => names.push(name.to_vec()),
        }
    }
    let mut absolute = vec![u16::from(drive), u16::from(b':'), SEPARATOR];
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            absolute.push(SEPARATOR);
        }
        absolute.extend_from_slice(name);
    }
    Some(absolute)
}

/// Whether this process may create symbolic links: Developer Mode is on, or its token
/// holds the symbolic-link privilege (an elevated administrator's does). Read from the
/// system's settings and the process token, never by creating a link to find out.
fn symbolic_links_allowed() -> bool {
    developer_mode() || symbolic_link_privilege()
}

fn developer_mode() -> bool {
    let key: Vec<u16> = r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let value: Vec<u16> = "AllowDevelopmentWithoutDevLicense"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: NUL-terminated key and value names; a DWORD output of its stated size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    status == 0 && data == 1
}

fn symbolic_link_privilege() -> bool {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    let name: Vec<u16> = "SeCreateSymbolicLinkPrivilege"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut luid = LUID::default();
    // SAFETY: a NUL-terminated privilege name and a LUID output.
    if unsafe { LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) } == 0 {
        return false;
    }
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the current-process pseudo handle needs no closing; the token output is
    // owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    // SAFETY: a token handle this call opened and nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0u32;
    // SAFETY: a size query with no buffer; it fails with the needed size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenPrivileges,
            std::ptr::null_mut(),
            0,
            &mut size,
        )
    };
    let mut buffer = vec![0u8; size as usize];
    // SAFETY: a writable buffer of the size the token reported.
    if size == 0
        || unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenPrivileges,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
    {
        return false;
    }
    // TOKEN_PRIVILEGES: a count, then LUID_AND_ATTRIBUTES entries of 12 bytes each.
    let word = |offset: usize| {
        buffer
            .get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let count = word(0).unwrap_or(0) as usize;
    (0..count).any(|index| {
        let entry = 4 + index * 12;
        word(entry) == Some(luid.LowPart) && word(entry + 4) == Some(luid.HighPart as u32)
    })
}

/// An owned link stage (`StagedTransferLink`).
struct LinkStage {
    directory: Directory,
    /// The staged link, not followed, with DELETE access; None once it is removed.
    file: Option<File>,
    identity: TransferIdentity,
    name: OsString,
    path: PathBuf,
    /// The final name, once published.
    published: Option<OsString>,
    /// A cleanup failed: no further removal is attempted, by a call or by Drop.
    refused: bool,
    _namespace_owner: super::super::namespace::NativeOwner,
}

impl LinkStage {
    fn held(&self) -> Result<&File, FsError> {
        self.file
            .as_ref()
            .ok_or_else(|| invalid(&self.path, "the link stage was removed"))
    }

    /// The link visible at `name` in the destination folder, which must be this stage's
    /// object with exactly `expected` as its representation.
    fn check(
        &self,
        name: &OsStr,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        let path = self.directory.path.join(name);
        checked(cancel, &path)?;
        self.directory.revalidate(cancel)?;
        let directory = matches!(
            expected.kind(),
            TransferLinkKind::WindowsJunction | TransferLinkKind::WindowsDirectorySymbolicLink
        );
        let visible = super::super::source_links::open_link(&self.directory.file, name, directory)
            .map_err(|e| error(&path, e))?;
        if identity(&visible, &path)? != self.identity
            || super::super::source_links::read_link(&visible, &path)? != *expected
        {
            return Err(invalid(&path, "the staged link changed"));
        }
        Ok(self.identity)
    }
}

impl StagedTransferLink for LinkStage {
    fn link_representation(
        &self,
        cancel: &Cancellation,
    ) -> Result<TransferLinkRepresentation, FsError> {
        checked(cancel, &self.path)?;
        super::super::source_links::read_link(self.held()?, &self.path)
    }

    fn sync(&mut self, cancel: &Cancellation) -> Result<(), FsError> {
        // A link's reparse data is written with its creation; Windows offers no flush of
        // it through a handle without write access, as a symbolic link's is.
        checked(cancel, &self.path)
    }

    fn revalidate_stage(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        if self.published.is_some() {
            return Err(invalid(&self.path, "the link was already published"));
        }
        self.check(&self.name, expected, cancel)
    }

    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        if self.published.is_some() || self.refused {
            return Err(invalid(&self.path, "the link stage is no longer available"));
        }
        validate_name(name).map_err(|e| error(&self.path, e))?;
        self.directory.revalidate(cancel)?;
        let path = self.directory.path.join(name);
        checked(cancel, &path)?;
        let file = self.held()?;
        super::super::namespace::change(cancel, |mutation| {
            mutation
                .rename(file, &self.directory.file, name, cancel)
                .map_err(|e| error(&path, e))
        })?;
        self.published = Some(name.to_owned());
        Ok(TransferPublication::Published(path))
    }

    fn revalidate_publication(
        &self,
        expected: &TransferLinkRepresentation,
        cancel: &Cancellation,
    ) -> Result<TransferIdentity, FsError> {
        let name = self
            .published
            .as_ref()
            .ok_or_else(|| invalid(&self.path, "the link has not been put in place"))?;
        self.check(name, expected, cancel)
    }

    fn cleanup(&mut self) -> Result<(), TransferLinkCleanupError> {
        if self.published.is_some() || self.refused || self.file.is_none() {
            return Ok(());
        }
        let cancel = Cancellation::default();
        let removed = match self.file.as_ref() {
            Some(file) => super::super::namespace::change(&cancel, |mutation| {
                mutation
                    .remove(file, &cancel, || native::remove(file))
                    .map_err(|e| error(&self.path, e))
            }),
            None => Ok(()),
        };
        match removed {
            Ok(()) => {
                // The link's name goes when this, its deleting handle, closes.
                self.file = None;
                Ok(())
            }
            Err(failure) => {
                self.refused = true;
                Err(TransferLinkCleanupError {
                    error: failure,
                    recovery: vec![TransferRecovery {
                        path: self.path.clone(),
                        identity: Some(self.identity),
                    }],
                    cleanup_unconfirmed: false,
                })
            }
        }
    }
}

impl Drop for LinkStage {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
#[path = "transfer_link_stage_tests.rs"]
mod tests;
