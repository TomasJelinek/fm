//! A Copy job's private staging folder (WIN-8): stages live in one `.midnight-stage-*` folder
//! per destination folder, take no namespace turn of their own, are published with the
//! security a file created in the destination gets, and the empty folder goes at the end.
use super::*;
use midnight_core::{TransferNamespacePin, TransferNamespaceProbe};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use windows_sys::Win32::Security::*;

/// A job's host: grants every turn and reservation (unless told to refuse turns, as a
/// latched peer makes a real host refuse changes), counts them, and refreshes the private pins it
/// keeps when a turn finishes, retiring those that certify their removal.
#[derive(Default)]
struct Turns {
    begun: AtomicUsize,
    private: AtomicUsize,
    pins: Mutex<Vec<Arc<dyn TransferNamespacePin>>>,
    refuse: AtomicBool,
}
impl Turns {
    fn refuse(&self, refuse: bool) {
        self.refuse.store(refuse, Ordering::SeqCst);
    }
    fn begun(&self) -> usize {
        self.begun.load(Ordering::SeqCst)
    }
    fn private(&self) -> usize {
        self.private.load(Ordering::SeqCst)
    }
    fn pins(&self) -> usize {
        self.pins.lock().unwrap().len()
    }
}
struct Host(Arc<Turns>);
struct Turn(Arc<Turns>);
impl TransferNamespaceChanges for Host {
    fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        // These tests register no job root, so no change reserves anything: a refusal that
        // the engine's reservation reports (`WouldBlock`, nothing changed) comes here.
        if self.0.refuse.load(Ordering::SeqCst) {
            return Err(error(Path::new(""), io::ErrorKind::WouldBlock.into()));
        }
        self.0.begun.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Turn(self.0.clone())))
    }
}
impl TransferNamespaceChange for Turn {
    fn reserve_private_child(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        if let Some(pin) = pin {
            pin.refresh(cancel)?;
            let mut pins = self.0.pins.lock().unwrap();
            if !pins.iter().any(|old| Arc::ptr_eq(old, &pin)) {
                self.0.private.fetch_add(1, Ordering::SeqCst);
                pins.push(pin);
            }
        }
        Ok(true)
    }
    fn finish(self: Box<Self>) -> Result<(), FsError> {
        let pins = std::mem::take(&mut *self.0.pins.lock().unwrap());
        let mut live = Vec::with_capacity(pins.len());
        let mut failure = None;
        for pin in pins {
            match pin.refresh(&Cancellation::default()) {
                Ok(TransferNamespaceProbe::Known(scopes)) if scopes.is_empty() => drop(pin),
                Ok(_) => live.push(pin),
                Err(error) => {
                    failure.get_or_insert(error);
                    live.push(pin);
                }
            }
        }
        self.0.pins.lock().unwrap().extend(live);
        failure.map_or(Ok(()), Err)
    }
}

/// Run `test` as a Copy job does: with its namespace host installed on this thread.
fn with_job<R>(test: impl FnOnce(&Turns) -> R) -> R {
    let turns = Arc::new(Turns::default());
    let host: Arc<dyn TransferNamespaceChanges> = Arc::new(Host(turns.clone()));
    with_transfer_namespace_changes(host, || test(&turns))
}

fn directory(path: &Path) -> Directory {
    Directory::from_file(native::open_parent(path).unwrap(), path.to_owned()).unwrap()
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The one staging folder in `folder`; fails unless it holds exactly that.
fn staging_folder(folder: &Path) -> PathBuf {
    let names = names(folder);
    let staging: Vec<_> = names
        .iter()
        .filter(|name| name.starts_with(".midnight-stage-"))
        .collect();
    assert_eq!(staging.len(), 1, "one staging folder expected in {names:?}");
    let name = staging[0];
    assert_eq!(name.len(), ".midnight-stage-".len() + 32, "{name}");
    assert!(
        name[".midnight-stage-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    folder.join(name)
}

/// Stage `bytes` for the job, then publish them as `name` after their readback.
fn publish(directory: &dyn TransferDirectory, name: &str, bytes: &[u8]) {
    let cancel = Cancellation::default();
    let mut stage = directory.stage_job_file(&cancel).unwrap();
    stage.write_all(bytes).unwrap();
    stage.sync(&cancel).unwrap();
    stage.publish_no_replace(OsStr::new(name), &cancel).unwrap();
}

#[test]
fn no_staged_file_is_visible_under_its_final_name_before_verification() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|_| {
        let directory = directory(temp.path());
        let cancel = Cancellation::default();
        let mut stage = directory.stage_job_file(&cancel).unwrap();
        stage.write_all(b"verified bytes").unwrap();
        stage.sync(&cancel).unwrap();
        // The destination shows the job's staging folder only; the stage is inside it.
        let folder = staging_folder(temp.path());
        assert_eq!(names(temp.path()).len(), 1);
        let staged = names(&folder);
        assert_eq!(staged.len(), 1);
        assert!(staged[0].starts_with(".me-"), "{staged:?}");
        let guard = stage.rewind_for_validation(&cancel).unwrap();
        let mut readback = Vec::new();
        stage.read_to_end(&mut readback).unwrap();
        guard.revalidate(&cancel).unwrap();
        assert_eq!(readback, b"verified bytes");
        assert!(!temp.path().join("final").exists());
        stage
            .publish_no_replace(OsStr::new("final"), &cancel)
            .unwrap();
        stage.revalidate_publication(&cancel).unwrap();
        assert_eq!(
            fs::read(temp.path().join("final")).unwrap(),
            b"verified bytes"
        );
        assert!(names(&folder).is_empty());
        drop((guard, stage));
        directory.finish_staging().unwrap();
        assert_eq!(names(temp.path()), ["final"]);
    });
}

#[test]
fn only_the_first_stage_of_a_folder_takes_a_namespace_turn() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|turns| {
        let directory = directory(temp.path());
        let cancel = Cancellation::default();
        let first = directory.stage_job_file(&cancel).unwrap();
        // The folder's creation: one turn and its one private reservation.
        assert_eq!((turns.begun(), turns.private()), (1, 1));
        let folder = staging_folder(temp.path());
        let mut later = Vec::new();
        for _ in 0..8 {
            later.push(directory.stage_job_file(&cancel).unwrap());
        }
        assert_eq!((turns.begun(), turns.private()), (1, 1));
        assert_eq!(names(&folder).len(), 9);
        // The clones of the folder's directory share it.
        let clone = directory.clone();
        later.push(clone.stage_job_file(&cancel).unwrap());
        assert_eq!(staging_folder(temp.path()), folder);
        assert_eq!(turns.private(), 1);
        drop((first, later));
        directory.finish_staging().unwrap();
        assert!(names(temp.path()).is_empty());
    });
}

#[test]
fn cancel_leaves_neither_stages_nor_the_staging_folder() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|turns| {
        let directory = directory(temp.path());
        let cancel = Cancellation::default();
        publish(&directory, "completed", b"kept");
        let mut first = directory.stage_job_file(&cancel).unwrap();
        first.write_all(b"partially written").unwrap();
        let mut second = directory.stage_job_file(&cancel).unwrap();
        second.write_all(b"also partial").unwrap();
        let folder = staging_folder(temp.path());
        assert_eq!(names(&folder).len(), 2);
        // Cancel: each unpublished stage through its handle, whose name goes once that
        // handle closes (as the job drops its work), then the empty folder.
        first.cleanup().unwrap();
        second.cleanup().unwrap();
        drop((first, second));
        assert!(names(&folder).is_empty());
        directory.finish_staging().unwrap();
        assert_eq!(names(temp.path()), ["completed"]);
        assert_eq!(fs::read(temp.path().join("completed")).unwrap(), b"kept");
        // The folder's pin certified its removal and retired.
        assert_eq!(turns.pins(), 0);
        // A finished folder has nothing more to remove; the next stage makes a new one.
        directory.finish_staging().unwrap();
        let again = directory.stage_job_file(&cancel).unwrap();
        assert_ne!(staging_folder(temp.path()), folder);
        drop(again);
        directory.finish_staging().unwrap();
        assert_eq!(names(temp.path()), ["completed"]);
    });
}

#[test]
fn a_foreign_entry_keeps_the_staging_folder_and_is_reported_by_its_path() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|_| {
        let directory = directory(temp.path());
        let cancel = Cancellation::default();
        let mut stage = directory.stage_job_file(&cancel).unwrap();
        stage.write_all(b"partial").unwrap();
        let folder = staging_folder(temp.path());
        fs::write(folder.join("foreign"), b"another program's file").unwrap();
        stage.cleanup().unwrap();
        drop(stage);
        let failure = directory
            .finish_staging()
            .expect_err("a folder holding an entry is kept");
        assert_eq!(failure.path, folder);
        assert_eq!(failure.kind, io::ErrorKind::DirectoryNotEmpty, "{failure}");
        assert_eq!(names(&folder), ["foreign"]);
        assert_eq!(
            fs::read(folder.join("foreign")).unwrap(),
            b"another program's file"
        );
        // Reported once: the job no longer holds the kept folder.
        directory.finish_staging().unwrap();
        assert_eq!(staging_folder(temp.path()), folder);
    });
}

#[test]
fn a_refused_removal_is_retried_by_a_later_finish_of_the_job() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|turns| {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        let finished = parent
            .create_directory(OsStr::new("finished"), &cancel)
            .unwrap();
        let dropped = parent
            .create_directory(OsStr::new("dropped"), &cancel)
            .unwrap();
        publish(finished.as_ref(), "file", b"finished");
        publish(dropped.as_ref(), "file", b"dropped");
        let folders = [
            staging_folder(&temp.path().join("finished")),
            staging_folder(&temp.path().join("dropped")),
        ];
        // A latched peer refuses every change of the job: neither removal is a failure of
        // the job, and neither folder is lost to it.
        turns.refuse(true);
        finished.finish_staging().unwrap();
        drop((finished, dropped));
        assert!(folders.iter().all(|folder| folder.exists()));
        // The job's next finish removes both.
        turns.refuse(false);
        parent.finish_staging().unwrap();
        assert!(folders.iter().all(|folder| !folder.exists()));
        assert_eq!(names(&temp.path().join("finished")), ["file"]);
        assert_eq!(names(&temp.path().join("dropped")), ["file"]);
    });
}

#[test]
fn a_removal_still_refused_at_the_job_end_is_reported_and_tried_once_more() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|turns| {
        let directory = directory(temp.path());
        publish(&directory, "file", b"bytes");
        let folder = staging_folder(temp.path());
        turns.refuse(true);
        // The job's own folder finishes last, at the job's end: what is still refused then
        // is reported by its path.
        let failure = directory
            .finish_staging()
            .expect_err("a refusal at the job's end is reported");
        assert_eq!(failure.kind, io::ErrorKind::WouldBlock, "{failure}");
        assert_eq!(failure.path, folder);
        assert!(folder.exists());
        // The last folder of the job to go tries once more.
        turns.refuse(false);
        drop(directory);
        assert_eq!(names(temp.path()), ["file"]);
    });
}

#[test]
fn sibling_destination_folders_stage_in_folders_of_their_own() {
    let temp = crate::test_fixtures::build_temp("staging-");
    with_job(|turns| {
        let parent = directory(temp.path());
        let cancel = Cancellation::default();
        let left = parent
            .create_directory(OsStr::new("left"), &cancel)
            .unwrap();
        let right = parent
            .create_directory(OsStr::new("right"), &cancel)
            .unwrap();
        let mut left_stage = left.stage_job_file(&cancel).unwrap();
        let mut right_stage = right.stage_job_file(&cancel).unwrap();
        left_stage.write_all(b"left").unwrap();
        right_stage.write_all(b"right").unwrap();
        assert_eq!(names(temp.path()), ["left", "right"]);
        let left_folder = staging_folder(&temp.path().join("left"));
        let right_folder = staging_folder(&temp.path().join("right"));
        assert_ne!(left_folder.file_name(), right_folder.file_name());
        assert_eq!(names(&left_folder).len(), 1);
        assert_eq!(names(&right_folder).len(), 1);
        assert_eq!(turns.private(), 2);
        for (stage, folder) in [
            (&mut left_stage, &left_folder),
            (&mut right_stage, &right_folder),
        ] {
            stage
                .publish_no_replace(OsStr::new("file"), &cancel)
                .unwrap();
            assert!(names(folder).is_empty());
        }
        drop((left_stage, right_stage));
        left.finish_staging().unwrap();
        // The other folder's staging is its own.
        assert_eq!(staging_folder(&temp.path().join("right")), right_folder);
        right.finish_staging().unwrap();
        assert_eq!(names(&temp.path().join("left")), ["file"]);
        assert_eq!(names(&temp.path().join("right")), ["file"]);
        assert_eq!(fs::read(temp.path().join("left/file")).unwrap(), b"left");
        assert_eq!(fs::read(temp.path().join("right/file")).unwrap(), b"right");
    });
}

#[test]
fn a_folder_that_refuses_new_entries_fails_the_stage_with_todays_words() {
    let temp = crate::test_fixtures::build_temp("staging-");
    let refusing = temp.path().join("refusing");
    fs::create_dir(&refusing).unwrap();
    let saved =
        crate::test_fixtures::deny_to_everyone(&refusing, FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY)
            .unwrap();
    let (today, refused, left) = with_job(|_| {
        let directory = directory(&refusing);
        let cancel = Cancellation::default();
        let today = directory
            .stage_file(&cancel)
            .err()
            .expect("a stage refused");
        let refused = directory
            .stage_job_file(&cancel)
            .err()
            .expect("a job's stage refused");
        (today, refused, names(&refusing))
    });
    crate::test_fixtures::restore_dacl(&refusing, &saved).unwrap();
    // No staging folder: the refused stage is one of its own, refused as today's is.
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(refused.operation, today.operation);
    assert_eq!(refused.kind, io::ErrorKind::PermissionDenied);
    assert_eq!(refused.kind, today.kind);
    assert_eq!(refused.message, today.message);
    assert_eq!(refused.path.parent(), Some(refusing.as_path()));
    assert_eq!(today.path.parent(), Some(refusing.as_path()));
    // The next job stages in a staging folder again.
    with_job(|_| {
        let directory = directory(&refusing);
        publish(&directory, "after", b"allowed again");
        staging_folder(&refusing);
        directory.finish_staging().unwrap();
    });
    assert_eq!(names(&refusing), ["after"]);
}

/// Publish one file for a job in `folder`, whose DACL `deny` changed (`restore` undoes it),
/// and return what the folder holds then and the turns the job took.
fn publish_in_refusing(
    folder: &Path,
    deny: impl FnOnce(&Path),
    restore: impl FnOnce(&Path),
) -> (Vec<String>, usize) {
    deny(folder);
    let begun = with_job(|turns| {
        let directory = directory(folder);
        for name in ["first", "second"] {
            publish(&directory, name, name.as_bytes());
        }
        directory.finish_staging().unwrap();
        turns.begun()
    });
    restore(folder);
    (names(folder), begun)
}

#[test]
fn a_folder_that_refuses_new_folders_gets_stages_of_their_own() {
    let temp = crate::test_fixtures::build_temp("staging-");
    let saved = std::cell::OnceCell::new();
    let (left, begun) = publish_in_refusing(
        temp.path(),
        |folder| {
            let denied = crate::test_fixtures::deny_to_everyone(folder, FILE_ADD_SUBDIRECTORY);
            assert!(saved.set(denied.unwrap()).is_ok());
        },
        |folder| crate::test_fixtures::restore_dacl(folder, saved.get().unwrap()).unwrap(),
    );
    // A write-only drop folder: no staging folder, each file staged beside its final name.
    assert_eq!(left, ["first", "second"]);
    assert_eq!(fs::read(temp.path().join("second")).unwrap(), b"second");
    // The refused creation's turn, then each stage's own and each publication's.
    assert!(begun >= 3, "{begun}");
}

#[test]
fn a_folder_whose_security_cannot_be_read_gets_stages_of_their_own() {
    let temp = crate::test_fixtures::build_temp("staging-");
    let user = Sid::user();
    // OWNER RIGHTS replaces the owner's implicit READ_CONTROL, so the deny applies to the
    // test's own folder too.
    let owner = Sid::well_known(WinCreatorOwnerRightsSid);
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let (left, _) = publish_in_refusing(
        temp.path(),
        |folder| {
            set_dacl(
                folder,
                &[
                    Entry(false, 0, READ_CONTROL, &owner),
                    Entry(true, tree, FILE_ALL_ACCESS, &user),
                ],
                true,
            );
            assert!(
                OpenOptions::new()
                    .access_mode(READ_CONTROL)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(folder)
                    .is_err()
            );
        },
        |folder| set_dacl(folder, &[Entry(true, tree, FILE_ALL_ACCESS, &user)], true),
    );
    assert_eq!(left, ["first", "second"]);
    assert_eq!(fs::read(temp.path().join("first")).unwrap(), b"first");
}

/// The reverse case: a folder that takes new folders but no new files never gets a staging
/// folder, so its files are refused before they are copied, not at their publication.
#[test]
fn a_folder_that_refuses_new_files_refuses_the_stage_before_the_copy() {
    let temp = crate::test_fixtures::build_temp("staging-");
    let saved = crate::test_fixtures::deny_to_everyone(temp.path(), FILE_ADD_FILE).unwrap();
    let (refused, left) = with_job(|_| {
        let directory = directory(temp.path());
        let refused = directory
            .stage_job_file(&Cancellation::default())
            .err()
            .expect("a stage refused");
        directory.finish_staging().unwrap();
        (refused, names(temp.path()))
    });
    crate::test_fixtures::restore_dacl(temp.path(), &saved).unwrap();
    assert_eq!(refused.kind, io::ErrorKind::PermissionDenied, "{refused}");
    // The stage it could not create beside the final names.
    assert_eq!(refused.path.parent(), Some(temp.path()));
    assert!(left.is_empty(), "{left:?}");
}

/// The process token's user, a well-known SID, each in an aligned buffer.
struct Sid(Vec<u64>);
impl Sid {
    fn well_known(kind: WELL_KNOWN_SID_TYPE) -> Self {
        let mut sid = vec![0u64; (SECURITY_MAX_SID_SIZE as usize).div_ceil(8)];
        let mut size = SECURITY_MAX_SID_SIZE;
        // SAFETY: an aligned writable buffer of SECURITY_MAX_SID_SIZE bytes.
        let created = unsafe {
            CreateWellKnownSid(
                kind,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut size,
            )
        };
        assert_ne!(created, 0, "{}", io::Error::last_os_error());
        Self(sid)
    }
    fn user() -> Self {
        let token = security::process_token().unwrap();
        let mut user = vec![0u64; 64];
        let mut needed = 0;
        // SAFETY: a live token handle and an aligned writable 512-byte buffer.
        let read = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                user.as_mut_ptr().cast(),
                512,
                &mut needed,
            )
        };
        assert_ne!(read, 0, "{}", io::Error::last_os_error());
        // SAFETY: the successful read initialized TOKEN_USER and its SID in `user`.
        let sid = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        // SAFETY: the valid SID above; the copy covers its exact length.
        let length = unsafe { GetLengthSid(sid) } as usize;
        let mut copy = vec![0u64; length.div_ceil(8)];
        // SAFETY: both ranges are `length` bytes long and do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(sid.cast::<u8>(), copy.as_mut_ptr().cast::<u8>(), length)
        };
        Self(copy)
    }
    fn psid(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast()
    }
}

/// One entry of a test DACL: allowed (or denied) `mask` for `sid` with ACE `flags`.
struct Entry<'a>(bool, u32, u32, &'a Sid);

fn open_security(path: &Path, access: u32) -> File {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap()
}

/// Give `folder` a protected DACL of `entries`, auto-inherited (as Explorer and
/// `SetSecurityInfo` leave a folder) when `auto`.
fn set_dacl(folder: &Path, entries: &[Entry<'_>], auto: bool) {
    let file = open_security(folder, WRITE_DAC);
    let mut acl = vec![0u64; 256];
    let pointer = acl.as_mut_ptr().cast::<ACL>();
    // SAFETY: the aligned writable allocation covers the 2,048 bytes given; every SID is
    // valid, and the descriptor and ACL outlive the SetKernelObjectSecurity call.
    unsafe {
        assert_ne!(InitializeAcl(pointer, 2048, ACL_REVISION), 0);
        for Entry(allow, flags, mask, sid) in entries {
            let added = if *allow {
                AddAccessAllowedAceEx(pointer, ACL_REVISION, *flags, *mask, sid.psid())
            } else {
                AddAccessDeniedAceEx(pointer, ACL_REVISION, *flags, *mask, sid.psid())
            };
            assert_ne!(added, 0, "{}", io::Error::last_os_error());
        }
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let descriptor = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        assert_ne!(InitializeSecurityDescriptor(descriptor, 1), 0);
        assert_ne!(SetSecurityDescriptorDacl(descriptor, 1, pointer, 0), 0);
        let bits = SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED | SE_DACL_AUTO_INHERIT_REQ;
        let set = if auto { bits } else { SE_DACL_PROTECTED };
        assert_ne!(SetSecurityDescriptorControl(descriptor, bits, set), 0);
        assert_ne!(
            SetKernelObjectSecurity(file.as_raw_handle(), DACL_SECURITY_INFORMATION, descriptor),
            0,
            "{}",
            io::Error::last_os_error()
        );
    }
}

/// Give `folder` a low mandatory label (no write up) inherited with ACE `flags`.
fn set_low_label(folder: &Path, flags: u32) {
    let file = open_security(folder, READ_CONTROL | WRITE_OWNER);
    let low = Sid::well_known(WinLowLabelSid);
    let mut acl = vec![0u64; 32];
    let pointer = acl.as_mut_ptr().cast::<ACL>();
    // SAFETY: as in `set_dacl`; 1 is SYSTEM_MANDATORY_LABEL_NO_WRITE_UP.
    unsafe {
        assert_ne!(InitializeAcl(pointer, 256, ACL_REVISION), 0);
        assert_ne!(
            AddMandatoryAce(pointer, ACL_REVISION, flags, 1, low.psid()),
            0
        );
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let descriptor = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        assert_ne!(InitializeSecurityDescriptor(descriptor, 1), 0);
        assert_ne!(SetSecurityDescriptorSacl(descriptor, 1, pointer, 0), 0);
        assert_ne!(
            SetKernelObjectSecurity(file.as_raw_handle(), LABEL_SECURITY_INFORMATION, descriptor),
            0,
            "{}",
            io::Error::last_os_error()
        );
    }
}

/// An entry's whole descriptor as far as a process without SeSecurityPrivilege reads it:
/// owner, group, the inheritance control bits, the DACL's entries in order (with their
/// flags) and the mandatory label.
#[derive(Debug, PartialEq, Eq)]
struct Security {
    control: u16,
    owner: Vec<u8>,
    group: Vec<u8>,
    dacl: Option<Vec<Vec<u8>>>,
    label: Option<Vec<Vec<u8>>>,
}
fn security_of(path: &Path) -> Security {
    let file = open_security(path, READ_CONTROL);
    let information = OWNER_SECURITY_INFORMATION
        | GROUP_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | LABEL_SECURITY_INFORMATION;
    let mut needed = 0;
    // SAFETY: a live READ_CONTROL handle and a zero-length size probe.
    unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
    let descriptor: PSECURITY_DESCRIPTOR = storage.as_mut_ptr().cast();
    // SAFETY: the aligned writable allocation covers `needed` bytes.
    let read = unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            descriptor,
            needed,
            &mut needed,
        )
    };
    assert_ne!(read, 0, "{}", io::Error::last_os_error());
    let sid_bytes = |sid: PSID| {
        // SAFETY: a valid SID inside the live descriptor.
        let length = unsafe { GetLengthSid(sid) } as usize;
        // SAFETY: `length` readable bytes at `sid`.
        unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), length) }.to_vec()
    };
    let entries = |acl: *mut ACL| {
        (!acl.is_null()).then(|| {
            // SAFETY: a valid ACL inside the live descriptor; each entry is read through
            // its own header's size.
            (0..unsafe { (*acl).AceCount })
                .map(|index| unsafe {
                    let mut ace = std::ptr::null_mut();
                    assert_ne!(GetAce(acl, u32::from(index), &mut ace), 0);
                    let size = (*ace.cast::<ACE_HEADER>()).AceSize as usize;
                    std::slice::from_raw_parts(ace.cast::<u8>(), size).to_vec()
                })
                .collect()
        })
    };
    let (mut owner, mut group, mut dacl, mut sacl) = (
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    );
    let (mut present, mut defaulted, mut control, mut revision) = (0, 0, 0u16, 0u32);
    // SAFETY: the self-relative descriptor read above and writable outputs.
    unsafe {
        assert_ne!(
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted),
            0
        );
        assert_ne!(
            GetSecurityDescriptorGroup(descriptor, &mut group, &mut defaulted),
            0
        );
        assert_ne!(
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted),
            0
        );
        assert_ne!(
            GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted),
            0
        );
        assert_ne!(
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
            0
        );
    }
    Security {
        control: control
            & (SE_DACL_PRESENT
                | SE_DACL_AUTO_INHERITED
                | SE_DACL_PROTECTED
                | SE_SACL_PRESENT
                | SE_SACL_AUTO_INHERITED
                | SE_SACL_PROTECTED),
        owner: sid_bytes(owner),
        group: sid_bytes(group),
        dacl: entries(dacl),
        label: entries(sacl),
    }
}

/// Publish a staged file and create one directly beside it; their descriptors must match.
fn assert_published_like_direct(directory: &Directory, case: &str, index: usize) -> Security {
    let published = format!("published-{index}");
    let direct = format!("direct-{index}");
    publish(directory, &published, case.as_bytes());
    fs::write(directory.path.join(&direct), case.as_bytes()).unwrap();
    let staged = security_of(&directory.path.join(&published));
    assert_eq!(
        staged,
        security_of(&directory.path.join(&direct)),
        "{case}: the published file's security differs from a file created in the folder"
    );
    staged
}

#[test]
fn published_stages_get_the_security_of_files_created_in_the_destination() {
    let user = Sid::user();
    let everyone = Sid::well_known(WinWorldSid);
    let users = Sid::well_known(WinBuiltinUsersSid);
    let guests = Sid::well_known(WinBuiltinGuestsSid);
    let creator = Sid::well_known(WinCreatorOwnerSid);
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let read = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
    let plain = [
        Entry(true, tree, FILE_ALL_ACCESS, &user),
        Entry(false, OBJECT_INHERIT_ACE, FILE_GENERIC_WRITE, &guests),
        Entry(true, tree, read, &users),
    ];
    let no_propagate = [
        Entry(true, tree, FILE_ALL_ACCESS, &user),
        Entry(
            true,
            OBJECT_INHERIT_ACE | NO_PROPAGATE_INHERIT_ACE,
            read,
            &everyone,
        ),
    ];
    // The user may write into the folder itself only; files get CREATOR OWNER's entry.
    let creator_owner = [
        Entry(true, 0, FILE_ALL_ACCESS, &user),
        Entry(true, tree | INHERIT_ONLY_ACE, FILE_ALL_ACCESS, &creator),
        Entry(true, tree, read, &users),
    ];
    let cases: [(&str, Option<&[Entry<'_>]>, bool); 7] = [
        ("inherited from the volume", None, false),
        ("plain, auto-inherited", Some(&plain), true),
        ("plain, classic inheritance", Some(&plain), false),
        ("no-propagate, auto-inherited", Some(&no_propagate), true),
        (
            "no-propagate, classic inheritance",
            Some(&no_propagate),
            false,
        ),
        ("creator owner, auto-inherited", Some(&creator_owner), true),
        (
            "creator owner, classic inheritance",
            Some(&creator_owner),
            false,
        ),
    ];
    for (case, entries, auto) in cases {
        let temp = crate::test_fixtures::build_temp("security-");
        let folder = temp.path().join("destination");
        fs::create_dir(&folder).unwrap();
        if let Some(entries) = entries {
            set_dacl(&folder, entries, auto);
        }
        if entries.is_some_and(|entries| entries.len() == 3) && auto {
            set_low_label(&folder, tree);
        }
        with_job(|_| {
            let directory = directory(&folder);
            let staged = assert_published_like_direct(&directory, case, 0);
            if entries.is_some() {
                // The entries really came from the folder, not from a default DACL.
                assert!(
                    staged.dacl.as_ref().is_some_and(|dacl| dacl.len() >= 2),
                    "{case}"
                );
            }
            directory.finish_staging().unwrap();
        });
    }
}

#[test]
fn a_folder_acl_changed_during_the_job_applies_to_the_files_staged_after_it() {
    let user = Sid::user();
    let users = Sid::well_known(WinBuiltinUsersSid);
    let guests = Sid::well_known(WinBuiltinGuestsSid);
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    for auto in [true, false] {
        let temp = crate::test_fixtures::build_temp("security-");
        let folder = temp.path().join("destination");
        fs::create_dir(&folder).unwrap();
        set_dacl(&folder, &[Entry(true, tree, FILE_ALL_ACCESS, &user)], auto);
        with_job(|_| {
            let directory = directory(&folder);
            let before = assert_published_like_direct(&directory, "before the change", 0);
            let staging = staging_folder(&folder);
            set_dacl(
                &folder,
                &[
                    Entry(true, tree, FILE_ALL_ACCESS, &user),
                    Entry(false, tree, FILE_WRITE_DATA, &guests),
                    Entry(true, tree, FILE_GENERIC_READ, &users),
                ],
                auto,
            );
            let after = assert_published_like_direct(&directory, "after the change", 1);
            assert_ne!(before, after, "the second file follows the changed ACL");
            assert_eq!(after.dacl.as_ref().map(Vec::len), Some(3));
            // Both files were staged in the job's one staging folder.
            assert_eq!(staging_folder(&folder), staging);
            directory.finish_staging().unwrap();
        });
    }
}

/// The documented residual (transfers.md): an inheritable SACL entry marked
/// NO_PROPAGATE_INHERIT_ACE reaches the staging folder but not the files staged in it.
#[test]
fn a_no_propagate_label_reaches_the_staging_folder_but_not_its_files() {
    let user = Sid::user();
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let temp = crate::test_fixtures::build_temp("security-");
    let folder = temp.path().join("destination");
    fs::create_dir(&folder).unwrap();
    set_dacl(&folder, &[Entry(true, tree, FILE_ALL_ACCESS, &user)], true);
    set_low_label(&folder, tree | NO_PROPAGATE_INHERIT_ACE);
    with_job(|_| {
        let directory = directory(&folder);
        publish(&directory, "published", b"bytes");
        fs::write(folder.join("direct"), b"bytes").unwrap();
        let staging = security_of(&staging_folder(&folder));
        let published = security_of(&folder.join("published"));
        let direct = security_of(&folder.join("direct"));
        assert!(staging.label.as_ref().is_some_and(|label| label.len() == 1));
        assert!(direct.label.as_ref().is_some_and(|label| label.len() == 1));
        assert!(published.label.as_ref().is_none_or(Vec::is_empty));
        assert_eq!(published.dacl, direct.dacl);
        directory.finish_staging().unwrap();
    });
}

#[test]
fn the_descriptor_guard_frees_its_descriptor_on_every_error_path() {
    let temp = crate::test_fixtures::build_temp("security-");
    let user = Sid::user();
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    // Auto-inherited, so `complete` has a DACL to set again.
    set_dacl(
        temp.path(),
        &[Entry(true, tree, FILE_ALL_ACCESS, &user)],
        true,
    );
    fs::write(temp.path().join("taken"), b"existing").unwrap();
    let folder = native::open_parent(temp.path()).unwrap();
    let readable = native::reopen(
        &folder,
        READ_CONTROL | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )
    .unwrap();
    let token = security::process_token().unwrap();
    let live = || security::LIVE.with(std::cell::Cell::get);
    assert_eq!(live(), 0);
    // The creation fails: the name exists.
    {
        let descriptor = security::FileDescriptor::inherited_from(&readable, &token).unwrap();
        assert_eq!(live(), 1);
        let failure = descriptor
            .create(
                &folder,
                OsStr::new("taken"),
                FILE_READ_DATA,
                FILE_SHARE_READ,
            )
            .unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
    }
    assert_eq!(live(), 0);
    // Setting the DACL again fails: the file was created without WRITE_DAC.
    {
        let mut descriptor = security::FileDescriptor::inherited_from(&readable, &token).unwrap();
        assert_ne!(descriptor.access(), 0);
        let file = descriptor
            .create(
                &folder,
                OsStr::new("created"),
                FILE_READ_DATA,
                FILE_SHARE_READ,
            )
            .unwrap();
        let failure = descriptor.complete(&file).unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(live(), 1);
    }
    assert_eq!(live(), 0);
    // Reading the folder's descriptor fails (no READ_CONTROL): nothing was allocated.
    let failure = security::FileDescriptor::inherited_from(&folder, &token)
        .err()
        .expect("READ_CONTROL is required");
    assert_eq!(failure.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(live(), 0);
    // A stage of a job frees its descriptor once it is created.
    with_job(|_| {
        let directory = directory(temp.path());
        let stage = directory.stage_job_file(&Cancellation::default()).unwrap();
        staging_folder(temp.path());
        assert_eq!(live(), 0);
        drop(stage);
        directory.finish_staging().unwrap();
    });
}

/// Manual measurement (WIN-8): what each stage's descriptor costs, in this build. Medians of
/// each step; a plain creation alternates before and after the described one.
/// `cargo test -p midnight-platform-windows --lib descriptor_cost -- --ignored --nocapture`
#[test]
#[ignore = "manual measurement: creates 4,000 files per folder kind"]
fn descriptor_cost_per_staged_file() {
    use std::time::{Duration, Instant};
    const COUNT: usize = 2_000;
    let user = Sid::user();
    let users = Sid::well_known(WinBuiltinUsersSid);
    let tree = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    let median = |mut times: Vec<Duration>| {
        times.sort();
        times[times.len() / 2].as_secs_f64() * 1e6
    };
    for (kind, auto) in [("classic", false), ("auto-inherited", true)] {
        let temp = crate::test_fixtures::build_temp("security-");
        set_dacl(
            temp.path(),
            &[
                Entry(true, tree, FILE_ALL_ACCESS, &user),
                Entry(true, tree, FILE_GENERIC_READ, &users),
            ],
            auto,
        );
        let folder = native::open_parent(temp.path()).unwrap();
        let readable = native::reopen(
            &folder,
            READ_CONTROL | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
        .unwrap();
        let token = security::process_token().unwrap();
        let access = FILE_READ_DATA | FILE_WRITE_DATA | DELETE;
        let plain = |index: usize| {
            let name = OsString::from(format!("plain-{index}"));
            let started = Instant::now();
            let file = native::open_relative(
                &folder,
                &name,
                access,
                FILE_SHARE_READ,
                FILE_CREATE,
                FILE_NON_DIRECTORY_FILE,
            )
            .unwrap();
            let elapsed = started.elapsed();
            native::remove(&file).unwrap();
            elapsed
        };
        let mut times: [Vec<Duration>; 4] = Default::default();
        for index in 0..COUNT {
            if index % 2 == 1 {
                times[3].push(plain(index));
            }
            let name = OsString::from(format!("described-{index}"));
            let started = Instant::now();
            let mut descriptor =
                security::FileDescriptor::inherited_from(&readable, &token).unwrap();
            times[0].push(started.elapsed());
            let started = Instant::now();
            let file = descriptor
                .create(
                    &folder,
                    &name,
                    access | descriptor.access(),
                    FILE_SHARE_READ,
                )
                .unwrap();
            times[1].push(started.elapsed());
            let started = Instant::now();
            descriptor.complete(&file).unwrap();
            drop(descriptor);
            times[2].push(started.elapsed());
            native::remove(&file).unwrap();
            if index % 2 == 0 {
                times[3].push(plain(index));
            }
        }
        let [computed, created, completed, plain] = times.map(median);
        println!(
            "PERF descriptor {kind}: median computation {computed:.1} us, creation with it \
             {created:.1} us, completion and release {completed:.1} us; plain creation \
             {plain:.1} us; so {:.1} us per staged file",
            computed + (created - plain) + completed
        );
    }
}
