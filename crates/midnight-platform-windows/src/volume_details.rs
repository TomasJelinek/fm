use midnight_core::{FsError, SpaceInfo};
use std::{
    borrow::Cow,
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNameW},
    System::Diagnostics::Debug::{GetThreadErrorMode, SEM_FAILCRITICALERRORS, SetThreadErrorMode},
};

/// `GetVolumeInformationW` documents `MAX_PATH + 1` as the largest name buffer.
pub(crate) const NAME_UNITS: usize = 261;

/// A junction, symbolic link or mount point in the panel path can lead to another
/// volume, which a lexical root lookup would miss: resolve the final object first.
/// Volumes without mount-manager registration (RAM disks, encrypted containers, some
/// redirectors) have no DOS final path (`ERROR_INVALID_FUNCTION`); for them, and any
/// other failure except a missing path, the lexical root lookup still answers.
fn final_path<'a>(
    path: &'a Path,
    operation: &'static str,
    canonicalize: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> Result<Cow<'a, Path>, FsError> {
    match canonicalize(path) {
        Ok(resolved) => Ok(Cow::Owned(resolved)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(FsError::from_io(operation, path, error))
        }
        Err(_) => Ok(Cow::Borrowed(path)),
    }
}

fn canonical(path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

fn with_path(mut error: FsError, path: &Path) -> FsError {
    error.path = path.to_owned();
    error
}

pub(crate) fn label(path: &Path) -> Result<Option<OsString>, FsError> {
    label_resolved(path, canonical)
}

fn label_resolved(
    path: &Path,
    canonicalize: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> Result<Option<OsString>, FsError> {
    let _mode = ErrorMode::new()
        .map_err(|error| FsError::from_io("suppress volume media dialogs", path, error))?;
    let resolved = final_path(path, "read volume label", canonicalize)?;
    query_label(&resolved, native_root, |root, output| {
        let length = native_length(output.len())?;
        // SAFETY: root is the NUL-terminated native volume root, output has
        // length writable units, and all unused optional outputs are null.
        if unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                output.as_mut_ptr(),
                length,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .map_err(|error| with_path(error, path))
}

pub(crate) fn space(path: &Path) -> Result<SpaceInfo, FsError> {
    space_resolved(path, canonical)
}

fn space_resolved(
    path: &Path,
    canonicalize: impl FnOnce(&Path) -> io::Result<PathBuf>,
) -> Result<SpaceInfo, FsError> {
    let _mode = ErrorMode::new()
        .map_err(|error| FsError::from_io("suppress volume media dialogs", path, error))?;
    // GetVolumePathNameW can resolve missing descendants to an existing volume.
    // Preserve StorageService's missing-path error contract before that lookup.
    std::fs::symlink_metadata(path)
        .map_err(|error| FsError::from_io("read volume capacity", path, error))?;
    let resolved = final_path(path, "read volume capacity", canonicalize)?;
    query_space(&resolved, native_root, |root| {
        let (mut available, mut total) = (0, 0);
        // SAFETY: root is NUL-terminated and the two output pointers reference
        // writable u64 values. The unused total-free-bytes output may be null.
        // Available bytes and total bytes both honor the calling user's quota.
        if unsafe {
            GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, ptr::null_mut())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(SpaceInfo { available, total })
    })
    .map_err(|error| with_path(error, path))
}

pub(crate) fn native_root(input: &[u16], output: &mut [u16]) -> io::Result<()> {
    let length = native_length(output.len())?;
    // SAFETY: input is NUL-terminated, output is writable for exactly
    // length UTF-16 units, and both buffers live through the call.
    if unsafe { GetVolumePathNameW(input.as_ptr(), output.as_mut_ptr(), length) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn query_space(
    path: &Path,
    root_query: impl FnMut(&[u16], &mut [u16]) -> io::Result<()>,
    mut space_query: impl FnMut(&[u16]) -> io::Result<SpaceInfo>,
) -> Result<SpaceInfo, FsError> {
    let root = resolve_root(path, root_query)?;
    space_query(&root).map_err(|error| FsError::from_io("read volume capacity", path, error))
}

fn query_label(
    path: &Path,
    root_query: impl FnMut(&[u16], &mut [u16]) -> io::Result<()>,
    mut label_query: impl FnMut(&[u16], &mut [u16]) -> io::Result<()>,
) -> Result<Option<OsString>, FsError> {
    let root = resolve_root(path, root_query)?;
    let name = wide_buffer(NAME_UNITS, |output| label_query(&root, output))
        .map_err(|error| FsError::from_io("read volume label", path, error))?;
    Ok((!name.is_empty()).then(|| OsString::from_wide(&name)))
}

pub(crate) fn resolve_root(
    path: &Path,
    mut root_query: impl FnMut(&[u16], &mut [u16]) -> io::Result<()>,
) -> Result<Vec<u16>, FsError> {
    let mut input: Vec<_> = path.as_os_str().encode_wide().collect();
    if input.is_empty() || input.contains(&0) {
        return Err(FsError::from_io(
            "resolve volume root",
            path,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "A volume path must be nonempty and cannot contain NUL.",
            ),
        ));
    }
    input.push(0);
    // The volume root is a prefix of the input plus at most a trailing separator, so
    // the input length (including its NUL) plus one bounds every valid answer. A long
    // path also maps to ERROR_FILENAME_EXCED_RANGE, which must not keep doubling.
    let limit = input.len() + 1;
    let mut root = wide_buffer(limit, |output| root_query(&input, output))
        .map_err(|error| FsError::from_io("resolve volume root", path, error))?;
    root.push(0);
    Ok(root)
}

/// Buffer growth follows native insufficient-buffer errors up to the largest valid
/// answer (`limit` UTF-16 units including the NUL); at that size the error is real.
pub(crate) fn wide_buffer(
    limit: usize,
    mut query: impl FnMut(&mut [u16]) -> io::Result<()>,
) -> io::Result<Vec<u16>> {
    let mut buffer = vec![0; limit.clamp(1, 64)];
    loop {
        match query(&mut buffer) {
            Ok(()) => {
                let end = buffer.iter().position(|unit| *unit == 0).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unterminated native volume string.",
                    )
                })?;
                buffer.truncate(end);
                return Ok(buffer);
            }
            Err(error)
                if matches!(error.raw_os_error(), Some(122 | 206 | 234))
                    && buffer.len() < limit =>
            {
                let length = buffer.len().saturating_mul(2).min(limit);
                native_length(length)?;
                buffer
                    .try_reserve_exact(length - buffer.len())
                    .map_err(|_| io::Error::other("Cannot allocate the native volume buffer."))?;
                buffer.resize(length, 0);
            }
            Err(error) => return Err(error),
        }
    }
}

fn native_length(length: usize) -> io::Result<u32> {
    u32::try_from(length).map_err(|_| buffer_overflow())
}

fn buffer_overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "Native volume buffer length exceeds DWORD.",
    )
}

/// A missing optical/removable medium must return an error without opening a
/// modal native dialog. Only this blocking thread's mode changes, then restores.
pub(crate) struct ErrorMode(u32);

impl ErrorMode {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: GetThreadErrorMode has no pointer arguments; SetThreadErrorMode
        // receives valid storage for the previous mode and only affects this thread.
        unsafe {
            let previous = GetThreadErrorMode();
            let mut actual_previous = 0;
            if SetThreadErrorMode(previous | SEM_FAILCRITICALERRORS, &mut actual_previous) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(actual_previous))
        }
    }
}

impl Drop for ErrorMode {
    fn drop(&mut self) {
        // SAFETY: This guard never leaves the synchronous caller's thread. The
        // saved mode came from SetThreadErrorMode; the unused output may be null.
        unsafe {
            SetThreadErrorMode(self.0, ptr::null_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StorageService, WindowsFileSystem};

    #[test]
    fn native_system_volume_root_and_directory_have_the_same_label() {
        let directory = std::env::current_dir().unwrap();
        let filesystem = WindowsFileSystem::new(None);
        let root = directory.ancestors().last().unwrap();
        let root_label = label(root).expect("the running system volume is available");
        assert_eq!(label(&directory).unwrap(), root_label);
        assert!(filesystem.space_info(&directory).is_ok());
    }

    #[test]
    fn unavailable_volume_reports_error_instead_of_an_absent_label() {
        let path = Path::new(r"\\?\Volume{00000000-0000-0000-0000-000000000000}\");
        assert!(label(path).is_err());
        assert!(space(path).is_err());
    }

    #[test]
    fn native_capacity_is_positive_and_resolves_long_directory_to_same_volume() {
        let directory = std::env::current_dir().unwrap();
        let root = directory.ancestors().last().unwrap();
        let capacity = space(&directory).unwrap();
        assert!(capacity.total > 0);
        assert!(capacity.available <= capacity.total);
        assert_eq!(capacity.total, space(root).unwrap().total);
    }

    #[test]
    fn missing_directory_cannot_report_its_existing_parents_capacity() {
        let fixture = crate::test_fixtures::build_temp("fixture-");
        let missing = fixture.path().join("missing_磁碟_🦀");
        let error = space(&missing).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::NotFound);
        assert_eq!(error.path, missing);
    }

    #[test]
    fn capacity_passes_the_dynamic_raw_native_root_and_preserves_zero_space() {
        let path = Path::new(r"\\?\C:\").join("磁碟_🦀_".repeat(100));
        let input: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let root: Vec<_> = r"\\?\C:\mounted_磁碟".repeat(30).encode_utf16().collect();
        let result = query_space(
            &path,
            |passed, output| {
                assert_eq!(passed, input);
                write_wide(output, &root)
            },
            |passed| {
                assert_eq!(&passed[..passed.len() - 1], root);
                assert_eq!(passed.last(), Some(&0));
                Ok(SpaceInfo {
                    available: 0,
                    total: 123,
                })
            },
        )
        .unwrap();
        assert_eq!(result.available, 0);
        assert_eq!(result.total, 123);
        let error = query_space(
            &path,
            |_, output| write_wide(output, &root),
            |_| Err(io::ErrorKind::PermissionDenied.into()),
        )
        .unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        assert_eq!(error.operation, "read volume capacity");
        assert_eq!(error.path, path);
    }

    fn write_wide(output: &mut [u16], value: &[u16]) -> io::Result<()> {
        if output.len() <= value.len() {
            return Err(io::Error::from_raw_os_error(234));
        }
        output[..value.len()].copy_from_slice(value);
        output[value.len()] = 0;
        Ok(())
    }

    #[test]
    fn growing_utf16_buffers_preserve_long_paths_raw_labels_and_native_resolved_root() {
        let path = Path::new(r"\\?\C:\").join("Žluťoučký_磁碟_🦀".repeat(100));
        let input: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let root: Vec<_> = r"\\?\C:\mounted_磁碟".repeat(30).encode_utf16().collect();
        // Labels fit MAX_PATH + 1 units; still larger than the first 64-unit buffer.
        let mut raw_name: Vec<_> = "Žluťoučký 磁碟 🦀".repeat(15).encode_utf16().collect();
        raw_name.push(0xd800);
        assert!(raw_name.len() > 64 && raw_name.len() < NAME_UNITS);
        let mut root_calls = 0;
        let mut label_calls = 0;
        let result = query_label(
            &path,
            |passed, output| {
                assert_eq!(passed, input);
                root_calls += 1;
                write_wide(output, &root)
            },
            |passed, output| {
                assert_eq!(&passed[..passed.len() - 1], root);
                assert_eq!(passed.last(), Some(&0));
                label_calls += 1;
                write_wide(output, &raw_name)
            },
        )
        .unwrap();
        assert!(root_calls > 1);
        assert!(label_calls > 1);
        assert_eq!(result, Some(OsString::from_wide(&raw_name)));
    }

    #[test]
    fn a_junction_to_another_volume_reports_that_volumes_label_and_capacity() {
        let system = std::path::PathBuf::from(
            std::env::var_os("SystemRoot").expect("Windows provides SystemRoot"),
        );
        let fixture = crate::test_fixtures::build_temp("volume-junction-");
        let filesystem = WindowsFileSystem::new(None);
        let fixture_space = filesystem.space_info(fixture.path()).unwrap();
        let system_space = filesystem.space_info(&system).unwrap();
        let link = fixture.path().join("to-system-volume");
        std::fs::create_dir(&link).unwrap();
        crate::test_fixtures::create_junction(&link, &system).unwrap();
        let linked_space = filesystem.space_info(&link);
        let linked_label = label(&link);
        std::fs::remove_dir(&link).unwrap();
        assert_eq!(linked_space.unwrap().total, system_space.total);
        assert_eq!(linked_label.unwrap(), label(&system).unwrap());
        if fixture_space.total == system_space.total {
            eprintln!("the fixture and system directories share a volume; weak check");
        }
    }

    /// RAM disks, encrypted containers and some redirectors are not registered with the
    /// mount manager, so the final DOS path that canonicalization needs fails with
    /// ERROR_INVALID_FUNCTION. Their label and free space still resolve lexically.
    #[test]
    fn a_volume_without_a_final_dos_path_still_reports_its_label_and_capacity() {
        let fixture = crate::test_fixtures::build_temp("volume-lexical-");
        let invalid_function =
            |_: &Path| -> io::Result<PathBuf> { Err(io::Error::from_raw_os_error(1)) };
        assert_eq!(
            label_resolved(fixture.path(), invalid_function).unwrap(),
            label(fixture.path()).unwrap()
        );
        assert_eq!(
            space_resolved(fixture.path(), invalid_function)
                .unwrap()
                .total,
            space(fixture.path()).unwrap().total
        );
        // A missing path stays an error, and a successful resolution is used as is.
        let missing = |_: &Path| -> io::Result<PathBuf> { Err(io::ErrorKind::NotFound.into()) };
        let error = label_resolved(fixture.path(), missing).unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::NotFound);
        assert_eq!(error.path, fixture.path());
        let resolved = final_path(Path::new(r"C:\link"), "read", |_| {
            Ok(PathBuf::from(r"D:\target"))
        })
        .unwrap();
        assert_eq!(resolved, Path::new(r"D:\target"));
        let lexical = final_path(Path::new(r"R:\ram disk"), "read", |_| {
            Err(io::ErrorKind::PermissionDenied.into())
        })
        .unwrap();
        assert_eq!(lexical, Path::new(r"R:\ram disk"));
    }

    #[test]
    fn a_too_long_error_at_the_largest_valid_size_is_returned_without_doubling_further() {
        // ERROR_FILENAME_EXCED_RANGE also reports an over-long input path; the buffer
        // must stop at the input length plus a separator instead of growing to 4 GiB.
        let path = Path::new(r"C:\").join("x".repeat(400));
        let input_units = path.as_os_str().encode_wide().count() + 1;
        let mut sizes = Vec::new();
        let error = resolve_root(&path, |_, output| {
            sizes.push(output.len());
            Err(io::Error::from_raw_os_error(206))
        })
        .unwrap_err();
        assert_eq!(error.operation, "resolve volume root");
        assert_eq!(sizes.last(), Some(&(input_units + 1)));
        assert!(sizes.iter().all(|size| *size <= input_units + 1));
        assert!(sizes.len() <= 5, "{sizes:?}");
        let mut calls = 0;
        assert!(
            wide_buffer(NAME_UNITS, |_| {
                calls += 1;
                Err(io::Error::from_raw_os_error(122))
            })
            .is_err()
        );
        assert_eq!(calls, 4, "64, 128, 256 and the 261-unit maximum");
    }

    #[test]
    fn empty_label_is_absent_and_native_errors_keep_original_path() {
        let path = Path::new(r"C:\folder\磁碟");
        assert_eq!(
            query_label(
                path,
                |_, output| write_wide(output, &[b'C' as u16, b':' as u16, b'\\' as u16]),
                |_, output| write_wide(output, &[]),
            )
            .unwrap(),
            None
        );
        let error = query_label(
            path,
            |_, output| write_wide(output, &[b'C' as u16]),
            |_, _| Err(io::Error::from_raw_os_error(5)),
        )
        .unwrap_err();
        assert_eq!(error.operation, "read volume label");
        assert_eq!(error.path, path);
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
        let error = query_label(
            path,
            |_, _| Err(io::ErrorKind::NotFound.into()),
            |_, _| panic!("root resolution failed"),
        )
        .unwrap_err();
        assert_eq!(error.operation, "resolve volume root");
        assert_eq!(error.kind, io::ErrorKind::NotFound);
    }

    #[test]
    fn embedded_nul_and_empty_paths_never_reach_the_native_query() {
        for path in [
            OsString::new(),
            OsString::from_wide(&[b'C' as u16, 0, b'X' as u16]),
        ] {
            let error = query_label(
                Path::new(&path),
                |_, _| panic!("invalid path reached native root query"),
                |_, _| panic!("invalid path reached native label query"),
            )
            .unwrap_err();
            assert_eq!(error.kind, io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn media_dialog_error_mode_is_restored_after_success_and_failure() {
        // SAFETY: GetThreadErrorMode takes no pointer arguments.
        let before = unsafe { GetThreadErrorMode() };
        {
            let _mode = ErrorMode::new().unwrap();
            // SAFETY: GetThreadErrorMode takes no pointer arguments.
            assert_eq!(unsafe { GetThreadErrorMode() } & SEM_FAILCRITICALERRORS, 1);
        }
        // SAFETY: GetThreadErrorMode takes no pointer arguments.
        assert_eq!(unsafe { GetThreadErrorMode() }, before);
        let _ = label(Path::new(""));
        // SAFETY: GetThreadErrorMode takes no pointer arguments.
        assert_eq!(unsafe { GetThreadErrorMode() }, before);
    }

    #[test]
    fn native_unicode_directory_beyond_260_utf16_units_resolves_its_volume() {
        let fixture = crate::test_fixtures::build_temp("fixture-");
        let directory = fixture
            .path()
            .join("磁碟_Žluťoučký_🦀".repeat(8))
            .join("磁碟_Žluťoučký_🦀".repeat(8))
            .join("磁碟_Žluťoučký_🦀".repeat(8));
        assert!(directory.as_os_str().encode_wide().count() > 260);
        std::fs::create_dir_all(&directory).unwrap();
        assert_eq!(label(&directory).unwrap(), label(fixture.path()).unwrap());
        assert_eq!(
            space(&directory).unwrap().total,
            space(fixture.path()).unwrap().total
        );
    }
}
