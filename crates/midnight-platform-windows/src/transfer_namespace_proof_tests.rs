//! The handle proof of a qualified folder's ancestry (`Parent::proven`) against the
//! per-level check that it replaces (`Parent::validate_levels`), kept here as the oracle.
//! NTFS refuses to rename a directory while anything below it is open, so no ancestor of a
//! held folder changes while the folder stays below it. The held folder itself can be
//! renamed, replaced, moved or deleted, and moved out so that its former ancestors can
//! change before it comes back. Red runs leave parts of the proof out through
//! `MIDNIGHT_TEST_NAMESPACE_PROOF_SKIP` (`TEST_PROOF_SKIP_ENV`): `links` for the moves out
//! and back, `final-path,links` for the other changes, `delete-pending` for a pending
//! delete.
use super::*;
use std::{cell::Cell, fs};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE,
    CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetLengthSid, InitializeAcl,
    InitializeSecurityDescriptor, OBJECT_INHERIT_ACE, SE_DACL_PROTECTED, SECURITY_DESCRIPTOR,
    SECURITY_MAX_SID_SIZE, SetKernelObjectSecurity, SetSecurityDescriptorControl,
    SetSecurityDescriptorDacl, WinCreatorOwnerRightsSid,
};

/// `top\middle 文档\parent\held` in a fresh fixture folder.
struct Tree {
    temp: tempfile::TempDir,
    top: PathBuf,
    middle: PathBuf,
    parent: PathBuf,
    held: PathBuf,
}
impl Tree {
    fn new() -> Self {
        Self::with_parent(|path| fs::create_dir(path)).unwrap()
    }
    /// The tree, with `parent` made by `make` (a case-sensitive folder, say).
    fn with_parent(make: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<Self> {
        let temp = crate::test_fixtures::build_temp("namespace-proof-");
        let top = temp.path().join("top");
        let middle = top.join("middle 文档");
        let parent = middle.join("parent");
        let held = parent.join("held");
        fs::create_dir_all(&middle)?;
        make(&parent)?;
        fs::create_dir(&held)?;
        Ok(Self {
            temp,
            top,
            middle,
            parent,
            held,
        })
    }
}

/// The folder's pooled parent authority, as a binding in it captures it.
fn pin(path: &Path) -> Arc<Parent> {
    capture(
        &native::open_parent(path).unwrap(),
        path,
        &Cancellation::default(),
    )
    .unwrap()
}

/// Whether `parent` proves its ancestry by handle. A qualified folder always does; on an
/// unqualified volume the test is skipped with the reason printed.
fn proves(parent: &Parent) -> bool {
    match parent.mode {
        LookupMode::ShortNamesDisabled { .. } => {
            assert!(
                parent.pinned.is_some(),
                "a qualified folder proves its ancestry by handle"
            );
            true
        }
        LookupMode::Unqualified => {
            eprintln!(
                "skipped: unqualified lookup rules here (8.3 names, or not NTFS); the handle \
                 proof needs qualified ones ({TEST_LOOKUP_ENV}=short-names-disabled forces them)"
            );
            false
        }
    }
}

/// Neither check reports a change, and the proof itself holds.
fn assert_unchanged(parent: &Parent) {
    let cancel = Cancellation::default();
    parent.validate(&cancel).unwrap();
    parent.validate_levels(&cancel).unwrap();
    assert!(parent.proven(&cancel).unwrap(), "the proof holds");
}

/// `validate` reports the change exactly as the per-level check does (kind, message and
/// path), and the proof itself sees it.
fn assert_reported_as_by_levels(parent: &Parent, kind: io::ErrorKind) {
    let cancel = Cancellation::default();
    let reported = parent
        .validate(&cancel)
        .expect_err("validate reports the change");
    let levels = parent
        .validate_levels(&cancel)
        .expect_err("the per-level check reports the change");
    assert_eq!(
        (reported.kind, &reported.message, &reported.path),
        (levels.kind, &levels.message, &levels.path)
    );
    assert_eq!(levels.kind, kind, "{levels}");
    assert!(
        !parent.proven(&cancel).unwrap(),
        "the proof itself sees the change"
    );
}

/// Finding 1 (WIN-8): NTFS refuses to rename a directory while anything below it is open,
/// whatever its sharing, so no ancestor of a held folder can be renamed, nor therefore
/// replaced, while the folder stays below it. Both checks keep holding.
#[test]
fn no_ancestor_of_a_held_folder_can_be_renamed_or_replaced() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    for level in [&tree.top, &tree.middle, &tree.parent] {
        let refused = fs::rename(level, level.with_file_name("away"))
            .expect_err("NTFS refuses to rename an ancestor of a held folder");
        // ERROR_ACCESS_DENIED
        assert_eq!(refused.raw_os_error(), Some(5), "{level:?}: {refused}");
        assert_unchanged(&parent);
    }
}

#[test]
fn a_renamed_held_folder_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    assert_unchanged(&parent);
    fs::rename(&tree.held, tree.parent.join("renamed")).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

#[test]
fn a_replaced_held_folder_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    fs::rename(&tree.held, tree.parent.join("away")).unwrap();
    fs::create_dir(&tree.held).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::InvalidInput);
}

#[test]
fn a_moved_held_folder_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    fs::rename(&tree.held, tree.middle.join("held")).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

#[test]
fn a_case_only_rename_in_a_case_sensitive_folder_is_reported_as_by_the_per_level_check() {
    let tree = match Tree::with_parent(case_sensitive_directory) {
        Ok(tree) => tree,
        Err(error) => {
            eprintln!("skipped: this machine refuses a case-sensitive folder: {error}");
            return;
        }
    };
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    assert_unchanged(&parent);
    fs::rename(&tree.held, tree.parent.join("HELD")).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

/// The accepted difference, in the safe direction: after a case-only rename in a
/// case-insensitive folder every level still opens by its pinned name, which the per-level
/// check accepts, but the held folder's path has changed, which the proof reports.
#[test]
fn a_case_only_rename_in_a_case_insensitive_folder_is_reported_though_each_level_opens() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    fs::rename(&tree.held, tree.parent.join("HELD")).unwrap();
    let cancel = Cancellation::default();
    parent.validate_levels(&cancel).unwrap();
    assert!(
        !parent.proven(&cancel).unwrap(),
        "the proof sees the change"
    );
    let reported = parent
        .validate(&cancel)
        .expect_err("validate reports the change");
    assert_eq!(
        (reported.kind, reported.message.as_str()),
        (
            io::ErrorKind::InvalidInput,
            "original namespace ancestor binding changed"
        )
    );
}

/// A delete on last close leaves the name in place while this test holds the folder.
#[test]
fn a_held_folder_with_a_pending_delete_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    assert_unchanged(&parent);
    dispose(&tree.held, false).unwrap();
    assert!(
        fs::read_dir(&tree.parent)
            .unwrap()
            .any(|entry| entry.unwrap().file_name() == "held"),
        "the folder keeps its name until its last handle closes"
    );
    assert_reported_as_by_levels(&parent, io::ErrorKind::PermissionDenied);
}

/// A POSIX delete removes the name at once, while this test still holds the folder.
#[test]
fn a_posix_deleted_held_folder_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    dispose(&tree.held, true).unwrap();
    assert_eq!(fs::read_dir(&tree.parent).unwrap().count(), 0);
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

/// The held folder moved out, its parent renamed away and created again, and the folder
/// moved back: its path, identity and delete state are the pinned ones again, and only
/// its link names another parent.
#[test]
fn a_held_folder_moved_back_into_a_replaced_parent_is_reported_as_by_the_per_level_check() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    assert_unchanged(&parent);
    let outside = tree.temp.path().join("outside");
    fs::rename(&tree.held, &outside).unwrap();
    fs::rename(&tree.parent, tree.middle.join("parent away")).unwrap();
    fs::create_dir(&tree.parent).unwrap();
    fs::rename(&outside, &tree.held).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

/// As above one level higher: the held folder and its held parent moved out (a directory
/// is renamed only while nothing below it is open), the grandparent renamed away and
/// created again, both moved back. The held folder's own link still holds; its parent's
/// names another directory.
#[test]
fn a_held_folder_and_its_parent_moved_back_into_a_replaced_grandparent_are_reported() {
    let tree = Tree::new();
    let parent = pin(&tree.held);
    if !proves(&parent) {
        return;
    }
    assert_unchanged(&parent);
    let held_outside = tree.temp.path().join("held outside");
    let parent_outside = tree.temp.path().join("parent outside");
    fs::rename(&tree.held, &held_outside).unwrap();
    fs::rename(&tree.parent, &parent_outside).unwrap();
    fs::rename(&tree.middle, tree.top.join("middle away")).unwrap();
    fs::create_dir(&tree.middle).unwrap();
    fs::rename(&parent_outside, &tree.parent).unwrap();
    fs::rename(&held_outside, &tree.held).unwrap();
    assert_reported_as_by_levels(&parent, io::ErrorKind::NotFound);
}

/// A folder pinned through a junction holds the junction's target levels; retargeting the
/// junction changes none of them, and only the display path's check (`check_path`, which
/// the proof leaves as it was) reports that the path now leads elsewhere.
#[test]
fn a_junction_in_the_display_path_changes_no_held_level_and_check_path_reports_its_target() {
    let temp = crate::test_fixtures::build_temp("namespace-proof-");
    let real = temp.path().join("real");
    let other = temp.path().join("other");
    let empty = temp.path().join("empty");
    for folder in [real.join("held"), other.join("held"), empty.clone()] {
        fs::create_dir_all(folder).unwrap();
    }
    let link = temp.path().join("link");
    fs::create_dir(&link).unwrap();
    crate::test_fixtures::create_junction(&link, &real).unwrap();
    let display = link.join("held");
    let parent = pin(&display);
    if !proves(&parent) {
        return;
    }
    let names: Vec<_> = parent
        .chain
        .iter()
        .filter_map(|level| level.physical.file_name())
        .take(2)
        .collect();
    assert_eq!(names, ["held", "real"], "the held levels are the target's");
    assert_unchanged(&parent);
    parent.check_path(&display).unwrap();
    retarget(&link, &other);
    assert_unchanged(&parent);
    let elsewhere = parent.check_path(&display).unwrap_err();
    assert_eq!(elsewhere.kind, io::ErrorKind::InvalidInput, "{elsewhere}");
    retarget(&link, &empty);
    assert_unchanged(&parent);
    let missing = parent.check_path(&display).unwrap_err();
    assert_eq!(missing.kind, io::ErrorKind::NotFound, "{missing}");
}

/// Within a refresh batch each held folder's proof and each link are read once, also for
/// a folder above it whose links the first proof already read.
#[test]
fn a_batch_proves_each_held_folder_and_reads_each_link_once() {
    let tree = Tree::new();
    let below = pin(&tree.held);
    let above = pin(&tree.parent);
    if !proves(&below) || !proves(&above) {
        return;
    }
    let cancel = Cancellation::default();
    let counted = |run: &dyn Fn()| {
        let folders = FOLDER_PROOFS.with(Cell::get);
        let links = EDGE_PROBES.with(Cell::get);
        run();
        (
            FOLDER_PROOFS.with(Cell::get) - folders,
            EDGE_PROBES.with(Cell::get) - links,
        )
    };
    let checks = || {
        below.validate(&cancel).unwrap();
        below.validate(&cancel).unwrap();
        above.validate(&cancel).unwrap();
    };
    let edges = below.edges();
    assert_eq!(counted(&checks), (3, 3 * edges - 1));
    let batched = counted(&|| {
        let _batch = TransferNamespaceRefreshBatch::begin();
        checks();
    });
    assert_eq!(batched, (2, edges));
}

/// Points the existing junction `link` at `target` instead.
fn retarget(link: &Path, target: &Path) {
    // Removes the junction itself, never its target's contents.
    fs::remove_dir(link).unwrap();
    fs::create_dir(link).unwrap();
    crate::test_fixtures::create_junction(link, target).unwrap();
}

/// Deletes the empty directory `path` through a handle of its own, as another program
/// would. A POSIX delete unlinks it at once; otherwise it stays, its delete pending, until
/// its last handle closes.
fn dispose(path: &Path, posix: bool) -> io::Result<()> {
    let file = OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let posix_information = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    };
    let information = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: a live handle opened with DELETE; each fixed-size structure outlives the call.
    let disposed = unsafe {
        if posix {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfoEx,
                (&posix_information as *const FILE_DISPOSITION_INFO_EX).cast(),
                size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        } else {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&information as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        }
    };
    succeeded(disposed)
}

fn succeeded(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Creates the directory `path` case-sensitive. Setting that flag needs the right to delete
/// children, which the rights that fixture folders inherit (Modify) lack: the new directory
/// first gets a protected DACL that gives its owner, this test, full access to it and to
/// what it will contain.
fn case_sensitive_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
    let mut owner = [0u64; (SECURITY_MAX_SID_SIZE as usize).div_ceil(8)];
    let mut owner_size = SECURITY_MAX_SID_SIZE;
    let sid = owner.as_mut_ptr().cast();
    // SAFETY: an aligned writable buffer of SECURITY_MAX_SID_SIZE bytes.
    succeeded(unsafe {
        CreateWellKnownSid(
            WinCreatorOwnerRightsSid,
            std::ptr::null_mut(),
            sid,
            &mut owner_size,
        )
    })?;
    // SAFETY: `sid` is the valid SID created above.
    let sid_length = unsafe { GetLengthSid(sid) } as usize;
    let length = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_length;
    let mut buffer = vec![0u64; length.div_ceil(8)];
    let acl = buffer.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let pointer = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    let open = |access| {
        OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    };
    // The owner may always change the DACL.
    let owned = open(WRITE_DAC)?;
    // SAFETY: `acl` covers `length` writable bytes in `buffer`, and `descriptor` refers to
    // it; both outlive the SetKernelObjectSecurity call on the live directory handle. The
    // descriptor is initialized in place (SECURITY_DESCRIPTOR_REVISION = 1).
    unsafe {
        succeeded(InitializeAcl(acl, length as u32, ACL_REVISION))?;
        succeeded(AddAccessAllowedAceEx(
            acl,
            ACL_REVISION,
            OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
            FILE_ALL_ACCESS,
            sid,
        ))?;
        succeeded(InitializeSecurityDescriptor(pointer, 1))?;
        succeeded(SetSecurityDescriptorDacl(pointer, 1, acl, 0))?;
        succeeded(SetSecurityDescriptorControl(
            pointer,
            SE_DACL_PROTECTED,
            SE_DACL_PROTECTED,
        ))?;
        succeeded(SetKernelObjectSecurity(
            owned.as_raw_handle(),
            DACL_SECURITY_INFORMATION,
            pointer,
        ))?;
    }
    let directory = open(FILE_WRITE_ATTRIBUTES | FILE_DELETE_CHILD)?;
    // FILE_CS_FLAG_CASE_SENSITIVE_DIR
    let information = FILE_CASE_SENSITIVE_INFO { Flags: 1 };
    // SAFETY: a live handle with FILE_WRITE_ATTRIBUTES and a structure of its exact size.
    succeeded(unsafe {
        SetFileInformationByHandle(
            directory.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&information as *const FILE_CASE_SENSITIVE_INFO).cast(),
            size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    })
}
