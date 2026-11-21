//! A worker that cannot start never waits on a system dialog, and its exit says why.
use super::*;
use crate::WorkerStopped;
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use windows_sys::Win32::System::Diagnostics::Debug::GetErrorMode;

/// This test's name in the library test binary, which runs it again in a child process.
const TEST_NAME: &str =
    "platform::startup_tests::a_worker_whose_library_is_missing_exits_without_a_system_dialog";
const CHILD: &str = "MIDNIGHT_WORKER_DIALOG_TEST";
const STATUS_DLL_NOT_FOUND: u32 = 0xC000_0135;
const POLICY: ResourcePolicy = ResourcePolicy {
    memory_bytes: 256 * 1024 * 1024,
    cpu: CpuBudget::Seconds(5),
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: u32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> NTSTATUS;
}

/// A minimal x64 console executable whose only import is a library that does not exist, so
/// the loader stops it before any of its code runs: an entry point that returns, one import
/// descriptor, its lookup and address tables, the two names and an empty relocation block
/// (nothing in it needs relocating), in one section.
fn missing_library_image() -> Vec<u8> {
    let mut image = vec![0u8; 0x400];
    let put = |image: &mut Vec<u8>, offset: usize, bytes: &[u8]| {
        image[offset..offset + bytes.len()].copy_from_slice(bytes);
    };
    put(&mut image, 0, b"MZ");
    put(&mut image, 0x3C, &0x40u32.to_le_bytes());
    put(&mut image, 0x40, b"PE\0\0");
    // COFF header: x64, one section, a PE32+ optional header; a large-address-aware
    // executable.
    let mut coff = Vec::new();
    coff.extend(0x8664u16.to_le_bytes());
    coff.extend(1u16.to_le_bytes());
    coff.extend([0; 12]);
    coff.extend(0xF0u16.to_le_bytes());
    coff.extend(0x0022u16.to_le_bytes());
    put(&mut image, 0x44, &coff);
    let mut optional = Vec::new();
    optional.extend(0x20Bu16.to_le_bytes());
    optional.extend([14, 0]);
    for value in [0x200u32, 0, 0, 0x1000, 0x1000] {
        optional.extend(value.to_le_bytes()); // code size .. entry point, base of code
    }
    optional.extend(0x1_4000_0000u64.to_le_bytes());
    for value in [0x1000u32, 0x200] {
        optional.extend(value.to_le_bytes()); // section and file alignment
    }
    for value in [6u16, 0, 0, 0, 6, 0] {
        optional.extend(value.to_le_bytes()); // OS, image and subsystem versions
    }
    for value in [0u32, 0x2000, 0x200, 0] {
        optional.extend(value.to_le_bytes()); // Win32 version, image and header sizes, checksum
    }
    optional.extend(3u16.to_le_bytes()); // console subsystem
    // Terminal server aware, NX compatible, dynamic base with high-entropy addresses.
    optional.extend(0x8160u16.to_le_bytes());
    for value in [0x10_0000u64, 0x1000, 0x10_0000, 0x1000] {
        optional.extend(value.to_le_bytes()); // stack and heap reserve and commit
    }
    optional.extend(0u32.to_le_bytes());
    optional.extend(16u32.to_le_bytes());
    let mut directories = [0u8; 128];
    directories[8..16].copy_from_slice(&[0x10, 0x10, 0, 0, 40, 0, 0, 0]); // imports
    directories[40..48].copy_from_slice(&[0xA0, 0x10, 0, 0, 12, 0, 0, 0]); // relocations
    directories[96..104].copy_from_slice(&[0x50, 0x10, 0, 0, 16, 0, 0, 0]); // address table
    optional.extend(directories);
    assert_eq!(optional.len(), 0xF0);
    put(&mut image, 0x58, &optional);
    let mut section = Vec::from(*b".text\0\0\0");
    for value in [0x200u32, 0x1000, 0x200, 0x200, 0, 0, 0] {
        section.extend(value.to_le_bytes());
    }
    section.extend(0xE000_0060u32.to_le_bytes()); // code, data, executable, readable, writable
    put(&mut image, 0x148, &section);
    // The section, at file offset 0x200 and address 0x1000.
    put(&mut image, 0x200, &[0xC3]);
    for (offset, value) in [(0x210, 0x1040u32), (0x21C, 0x1080), (0x220, 0x1050)] {
        put(&mut image, offset, &value.to_le_bytes());
    }
    put(&mut image, 0x240, &0x1060u64.to_le_bytes());
    put(&mut image, 0x250, &0x1060u64.to_le_bytes());
    put(&mut image, 0x262, b"Missing\0");
    put(&mut image, 0x280, b"midnight-missing-library.dll\0");
    // One relocation block for the section's page with two padding entries.
    put(&mut image, 0x2A0, &0x1000u32.to_le_bytes());
    put(&mut image, 0x2A4, &12u32.to_le_bytes());
    image
}

fn missing_library_worker(directory: &Path) -> PathBuf {
    let executable = directory.join("midnight-missing-library-worker.exe");
    std::fs::write(&executable, missing_library_image()).unwrap();
    executable
}

/// The native error mode of a process: bit 0 set means "show critical-error dialogs".
fn native_error_mode(process: &Process) -> io::Result<u32> {
    let mut mode = 0u32;
    // SAFETY: a live process handle; the output is a writable u32.
    let status = unsafe {
        NtQueryInformationProcess(
            process.process.as_raw_handle(),
            PROCESS_DEFAULT_HARD_ERROR_MODE,
            (&mut mode as *mut u32).cast(),
            size_of::<u32>() as u32,
            null_mut(),
        )
    };
    if status < 0 {
        return Err(io::Error::other(format!("NTSTATUS {status:#x}")));
    }
    Ok(mode)
}

/// Wait at most `milliseconds` for the process to exit.
fn exited_within(process: &Process, milliseconds: u32) -> bool {
    // SAFETY: a live process handle; the wait is bounded.
    unsafe { WaitForSingleObject(process.process.as_raw_handle(), milliseconds) == WAIT_OBJECT_0 }
}

#[test]
fn a_worker_whose_library_is_missing_exits_without_a_system_dialog() {
    if std::env::var_os(CHILD).is_some() {
        return missing_library_child();
    }
    // Run again in a process with the default error mode, in which a worker that inherited
    // its parent's mode would show the loader's "System Error" dialog.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .creation_flags(CREATE_DEFAULT_ERROR_MODE)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // SAFETY: the owned child's live handle; the wait is bounded.
    let exited = unsafe { WaitForSingleObject(child.as_raw_handle(), 60_000) } == WAIT_OBJECT_0;
    if !exited {
        let _ = child.kill();
    }
    let output = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(exited, "the test process did not finish: {text}");
    assert!(output.status.success(), "{text}");
}

fn missing_library_child() {
    // SAFETY: GetErrorMode only reads this process's mode.
    let mode = unsafe { GetErrorMode() };
    assert_eq!(
        mode & SEM_FAILCRITICALERRORS,
        0,
        "this process must allow dialogs"
    );
    let directory = tempfile::tempdir().unwrap();
    let executable = missing_library_worker(directory.path());
    // Before it runs, the worker has its own error mode, whatever this process's is. It is
    // never resumed otherwise, so a missing fix fails here without showing a dialog.
    let mut checked = |process: &Process| {
        let native = native_error_mode(process)?;
        if native != CHILD_ERROR_MODE ^ SEM_FAILCRITICALERRORS {
            return Err(io::Error::other(format!(
                "the suspended worker would show dialogs: native error mode {native:#x}"
            )));
        }
        Ok(())
    };
    let mut worker = spawn_impl(&executable, POLICY, Some(&mut checked)).unwrap();
    assert!(
        exited_within(&worker.process, 30_000),
        "the worker kept running"
    );
    let status = worker.process.wait_status().unwrap();
    assert_eq!(status, ExitStatus::Code(STATUS_DLL_NOT_FOUND));
    assert_eq!(
        status.reason(),
        "a required library is missing or cannot be read (STATUS_DLL_NOT_FOUND, 0xC0000135)"
    );
    // The public launch reports the same: either its policy write finds the worker gone,
    // or the worker's Ready read ends and its exit status says why.
    match crate::spawn_with_policy(&executable, POLICY) {
        Err(error) => {
            let stopped = error
                .get_ref()
                .and_then(|error| error.downcast_ref::<WorkerStopped>());
            assert_eq!(stopped, Some(&WorkerStopped(status)), "{error}");
            assert_eq!(
                error.to_string(),
                "The worker stopped: a required library is missing or cannot be read \
                 (STATUS_DLL_NOT_FOUND, 0xC0000135)"
            );
        }
        Ok(mut worker) => {
            let ready = midnight_worker_sdk::read_policy_ready(&mut worker.output);
            assert_eq!(ready.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
            assert!(
                exited_within(&worker.process, 30_000),
                "the worker kept running"
            );
            assert_eq!(worker.process.wait_status().unwrap(), status);
        }
    }
    assert!(status.library_not_loaded());
    assert_eq!(
        crate::unloadable_library(&crate::worker_libraries(&executable)),
        None,
        "no library lies beside the worker yet"
    );
    // The library present beside the worker but not granted to the workers' AppContainer:
    // the loader is denied it and reports it as not found. The host names it.
    let library = directory.path().join("midnight-missing-library.dll");
    std::fs::write(&library, b"not an image").unwrap();
    let denied = exit_of(&executable);
    assert_eq!(denied, ExitStatus::Code(STATUS_DLL_NOT_FOUND));
    let unloadable = crate::unloadable_library(&crate::worker_libraries(&executable));
    assert_eq!(
        unloadable,
        Some(crate::UnloadableLibrary {
            path: library.clone(),
            missing: false
        })
    );
    assert_eq!(
        unloadable.unwrap().to_string(),
        format!(
            "could not load its libraries (access denied): {}",
            library.display()
        )
    );
    // Granted as the build and deployment scripts grant FFmpeg, the loader opens it and
    // refuses its content instead, and the host finds nothing to blame on access.
    grant_workers(&library);
    let opened = exit_of(&executable);
    assert!(
        !opened.success() && !opened.library_not_loaded(),
        "{opened:?}"
    );
    assert_eq!(
        crate::unloadable_library(&crate::worker_libraries(&executable)),
        None
    );
}

/// Start `executable` as a worker and return how it ended (this process allows dialogs, the
/// worker never does).
fn exit_of(executable: &Path) -> ExitStatus {
    let mut worker = spawn_impl(executable, POLICY, None).unwrap();
    assert!(
        exited_within(&worker.process, 30_000),
        "the worker kept running"
    );
    worker.process.wait_status().unwrap()
}

/// Grant the workers' AppContainer read and execute on `path` alone, as
/// `scripts/deploy-private.ps1` grants the FFmpeg libraries.
fn grant_workers(path: &Path) {
    use windows_sys::Win32::{
        Security::{ACL, Authorization::*, DACL_SECURITY_INFORMATION, NO_INHERITANCE},
        Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ},
    };
    let sid = worker_profile_sid().unwrap();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: a terminated path; Windows allocates the descriptor that owns `acl`.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, 0);
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.0.cast(),
        },
    };
    let mut updated = null_mut();
    // SAFETY: one explicit entry and the file's current ACL; Windows allocates the result.
    assert_eq!(
        unsafe { SetEntriesInAclW(1, &access, acl, &mut updated) },
        0
    );
    // SAFETY: a terminated path and the ACL built above.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            updated,
            null_mut(),
        )
    };
    // SAFETY: both were allocated by Windows for this caller.
    unsafe {
        LocalFree(updated.cast());
        LocalFree(descriptor);
    }
    assert_eq!(status, 0);
}

#[test]
fn the_workers_may_load_only_libraries_granted_to_their_appcontainer() {
    let directory = tempfile::tempdir().unwrap();
    let granted = directory.path().join("granted.dll");
    let denied = directory.path().join("Denied.DLL");
    let missing = directory.path().join("missing.dll");
    std::fs::write(&granted, b"library").unwrap();
    std::fs::write(&denied, b"library").unwrap();
    std::fs::write(directory.path().join("notes.txt"), b"not a library").unwrap();
    grant_workers(&granted);
    assert!(worker_may_load(&granted).unwrap());
    assert!(!worker_may_load(&denied).unwrap());
    assert_eq!(
        crate::worker_libraries(&directory.path().join("worker.exe")),
        [denied.clone(), granted.clone()]
    );
    assert_eq!(
        crate::unloadable_library(std::slice::from_ref(&granted)),
        None
    );
    assert_eq!(
        crate::unloadable_library(&[granted.clone(), denied.clone(), missing.clone()]),
        Some(crate::UnloadableLibrary {
            path: denied,
            missing: false
        })
    );
    let unloadable = crate::unloadable_library(&[granted, missing.clone()]);
    assert_eq!(
        unloadable.as_ref().map(ToString::to_string),
        Some(format!(
            "could not load its libraries: {} is missing",
            missing.display()
        ))
    );
}

#[test]
fn workers_run_as_the_appcontainer_that_the_build_scripts_grant_their_libraries() {
    // scripts/build-thumbnailers.ps1 and scripts/deploy-private.ps1 grant the FFmpeg
    // libraries to this SID (scripts/tests/test_restricted_worker_identity.py); a worker
    // running as any other AppContainer cannot load them.
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    let sid = worker_profile_sid().unwrap();
    let mut text = null_mut();
    // SAFETY: a valid SID owned by `sid`; Windows allocates the terminated string.
    assert_ne!(unsafe { ConvertSidToStringSidW(sid.0, &mut text) }, 0);
    // SAFETY: the string is terminated; it is freed once below.
    let value = unsafe {
        let length = (0..).take_while(|&index| *text.add(index) != 0).count();
        let value = String::from_utf16(std::slice::from_raw_parts(text, length));
        LocalFree(text.cast());
        value
    };
    assert_eq!(
        value.unwrap(),
        "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998"
    );
}

#[test]
fn exit_reasons_name_the_loader_crash_and_resource_statuses() {
    for (code, reason) in [
        (
            0xC000_0135,
            "a required library is missing or cannot be read (STATUS_DLL_NOT_FOUND, 0xC0000135)",
        ),
        (
            0xC000_0139,
            "a required library does not provide a function it needs \
             (STATUS_ENTRYPOINT_NOT_FOUND, 0xC0000139)",
        ),
        (
            0xC000_0005,
            "it crashed (STATUS_ACCESS_VIOLATION, 0xC0000005)",
        ),
        (
            0xC000_0409,
            "it stopped on an internal error (STATUS_STACK_BUFFER_OVERRUN, 0xC0000409)",
        ),
        (
            0xC000_012D,
            "it ran out of memory (STATUS_COMMITMENT_LIMIT, 0xC000012D)",
        ),
        // The termination code of a killed worker job, and any other code.
        (1, "it exited with code 0x00000001"),
        (0x65, "it exited with code 0x00000065"),
    ] {
        assert_eq!(ExitStatus::Code(code).reason(), reason);
    }
}
