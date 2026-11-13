#![cfg(windows)]
#![allow(unsafe_code)]
mod support;

use midnight_filesystem_plugin_api::{abi::CAP_PREPARED_RENAME, *};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, FileTimes, OpenOptions},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use support::{Client, Temp, address};
use windows_sys::Win32::{Storage::FileSystem::*, System::WindowsProgramming::DRIVE_FIXED};

fn text(name: &OsStr) -> NativeText {
    NativeText {
        encoding: NameEncoding::WindowsUtf16Le,
        bytes: name.encode_wide().flat_map(u16::to_le_bytes).collect(),
    }
}

struct Prepared<'a> {
    client: &'a Client,
    mount: u64,
    info: PreparedRename,
}

impl<'a> Prepared<'a> {
    fn new(client: &'a Client, source: &Path) -> Self {
        let (mount, node) = client.resolve(source);
        let response = client.call(Request::PrepareRename {
            mount,
            node: node.id,
        });
        let Response::RenamePrepared(info) = response else {
            panic!("Windows provider must implement prepared Rename: {response:?}");
        };
        assert_ne!(client.api.capabilities & CAP_PREPARED_RENAME, 0);
        info.validate().unwrap();
        assert_eq!(info.source.name, text(source.file_name().unwrap()));
        Self {
            client,
            mount,
            info,
        }
    }

    fn request(&self, attempt: u64, name: &OsStr, policy: RenamePolicy) -> Response {
        self.client.call(Request::Rename {
            prepared: self.info.handle,
            attempt,
            name: text(name),
            policy,
        })
    }

    fn attempt(&self, attempt: u64, name: &OsStr, policy: RenamePolicy) -> RenameState {
        let response = self.request(attempt, name, policy);
        let Response::RenameState(state) = response else {
            panic!("Expected retained Rename state for attempt {attempt}: {response:?}");
        };
        assert_eq!(state.prepared, self.info.handle);
        assert_eq!(state.attempt, attempt);
        state.validate().unwrap();
        state
    }

    fn query(&self) -> RenameState {
        let response = self.client.cancelled(
            Request::GetRenameState {
                prepared: self.info.handle,
            },
            1,
        );
        let Response::RenameState(state) = response else {
            panic!("State query must ignore user cancellation: {response:?}");
        };
        assert_eq!(state.prepared, self.info.handle);
        state
    }

    fn close(self) {
        assert_eq!(
            self.client.cancelled(
                Request::CloseRename {
                    prepared: self.info.handle,
                },
                1,
            ),
            Response::Unit
        );
        assert_eq!(
            self.client.call(Request::CloseMount { mount: self.mount }),
            Response::Unit
        );
    }
}

fn conflict(state: &RenameState) -> &RenameConflict {
    let RenameOutcome::Conflict(value) = &state.outcome else {
        panic!("Expected a collision, never an implicit overwrite: {state:?}");
    };
    assert_ne!(value.token, 0);
    value
}

fn refused(response: Response, expected: ErrorCode) {
    let Response::Failed(error) = response else {
        panic!("Expected protocol refusal {expected:?}: {response:?}");
    };
    assert_eq!(error.code, expected, "{error:?}");
}

fn source_changed(state: &RenameState) {
    assert!(
        matches!(
            &state.outcome,
            RenameOutcome::Unchanged(Failure {
                code: ErrorCode::SourceChanged,
                ..
            })
        ),
        "A prepared source/parent must never be replaced by a new pin: {state:?}"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: [u8; 16],
}

fn native_identity(path: &Path) -> Identity {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut id = FILE_ID_INFO::default();
    // SAFETY: this owned no-follow fixture handle and exact output buffer stay
    // live throughout the synchronous metadata query; no namespace is changed.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&mut id as *mut FILE_ID_INFO).cast(),
                size_of::<FILE_ID_INFO>() as u32,
            )
        },
        0,
        "Fixture identity query failed for {path:?}: {}",
        std::io::Error::last_os_error()
    );
    Identity {
        volume: id.VolumeSerialNumber,
        file: id.FileId.Identifier,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    raw_relative: Vec<u16>,
    identity: Identity,
    attributes: u32,
    contents: Option<Vec<u8>>,
}

fn snapshot(root: &Path) -> Vec<Entry> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<Entry>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let attributes = metadata.file_attributes();
        let ordinary = attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0;
        entries.push(Entry {
            raw_relative: path
                .strip_prefix(root)
                .unwrap()
                .as_os_str()
                .encode_wide()
                .collect(),
            identity: native_identity(path),
            attributes,
            contents: (ordinary && metadata.is_file()).then(|| fs::read(path).unwrap()),
        });
        if ordinary && metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|left, right| left.raw_relative.cmp(&right.raw_relative));
    entries
}

fn decode_address(value: &Address) -> PathBuf {
    assert_eq!(value.encoding, AddressEncoding::WindowsUtf16Le);
    let (pairs, remainder) = value.bytes.as_chunks::<2>();
    assert!(remainder.is_empty());
    let units: Vec<_> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    assert!(!units.contains(&0));
    PathBuf::from(OsString::from_wide(&units))
}

fn require_fixed_local_ntfs(client: &Client, path: &Path) {
    let response = client.call(Request::VolumeInformation {
        address: address(path),
    });
    let Response::VolumeInformation(info) = response else {
        panic!("Cannot qualify replacement fixture volume: {response:?}");
    };
    let filesystem = info.filesystem.as_ref().map(|value| {
        assert_eq!(value.encoding, NameEncoding::WindowsUtf16Le);
        let (pairs, remainder) = value.bytes.as_chunks::<2>();
        assert!(remainder.is_empty());
        let units: Vec<_> = pairs.iter().copied().map(u16::from_le_bytes).collect();
        String::from_utf16(&units).unwrap()
    });
    let root = decode_address(
        info.root
            .as_ref()
            .expect("Cannot qualify replacement without an exact volume root"),
    );
    let terminated: Vec<_> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: this is the provider-reported volume root in terminated UTF-16;
    // GetDriveTypeW only queries its drive type.
    let drive = unsafe { GetDriveTypeW(terminated.as_ptr()) };
    assert!(
        filesystem.as_deref() == Some("NTFS")
            && drive == DRIVE_FIXED
            && info.storage != StorageKind::Network
            && info.read_only == Some(false),
        "Fixture limitation: recoverable replacement requires writable local fixed NTFS; \
         no replacement test was skipped or qualified here. drive={drive}; info={info:?}"
    );
}

struct FilesBefore {
    source: PathBuf,
    destination: PathBuf,
    source_id: Identity,
    target_id: Identity,
    source_bytes: Vec<u8>,
    target_bytes: Vec<u8>,
}

impl FilesBefore {
    fn capture(source: &Path, destination: &Path) -> Self {
        let value = Self {
            source: source.to_owned(),
            destination: destination.to_owned(),
            source_id: native_identity(source),
            target_id: native_identity(destination),
            source_bytes: fs::read(source).unwrap(),
            target_bytes: fs::read(destination).unwrap(),
        };
        assert_ne!(value.source_id, value.target_id);
        value
    }

    fn assert_replaced(
        &self,
        fixture: &Temp,
        state: &RenameState,
        prepared: &PreparedRename,
        consent: &RenameConflict,
    ) -> PathBuf {
        let RenameOutcome::Renamed {
            name,
            same_entry,
            recovery,
            ..
        } = &state.outcome
        else {
            panic!("Qualified regular-file replacement must commit: {state:?}");
        };
        assert_eq!(name, &text(self.destination.file_name().unwrap()));
        assert!(!same_entry);
        assert_eq!(prepared.source.kind, Kind::File);
        assert_ne!(prepared.source.identity, consent.target.identity);
        assert_eq!(
            fs::symlink_metadata(&self.source).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(native_identity(&self.destination), self.source_id);
        assert_eq!(fs::read(&self.destination).unwrap(), self.source_bytes);
        assert_eq!(recovery.len(), 1, "Every residual must be reported exactly");
        let displaced = &recovery[0];
        assert_eq!(displaced.role, RenameRecoveryRole::DisplacedTarget);
        assert_eq!(displaced.identity, consent.target.identity);
        let path = decode_address(&displaced.address);
        assert!(path.is_absolute(), "Recovery locator must be absolute");
        assert_eq!(address(&path), displaced.address, "Lossless raw locator");
        assert!(
            path.canonicalize()
                .unwrap()
                .starts_with(fixture.path.canonicalize().unwrap()),
            "Recovery must remain inside the owned fixture: {path:?}"
        );
        assert_ne!(path, self.source);
        assert_ne!(path, self.destination);
        assert_eq!(native_identity(&path), self.target_id);
        assert_eq!(fs::read(&path).unwrap(), self.target_bytes);
        path
    }
}

fn make_change_time_older(path: &Path) {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .unwrap();
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: the fixture handle and complete FILE_BASIC_INFO output remain live.
    assert_ne!(
        unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
    // Ensure later changes cannot coalesce with the captured change timestamp.
    // Preserve size, LastWriteTime and every other publicly previewed field.
    basic.ChangeTime -= 100_000_000;
    // SAFETY: changes only basic timestamps of this freshly owned fixture file;
    // no privilege, ACL manipulation or global clock change is involved.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileBasicInfo,
                (&basic as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
}

fn junction(path: &Path, target: &Path) {
    // Match the existing native tests: junction creation needs no symlink
    // privilege. Names are fresh owned fixture paths, and Rust owns cleanup.
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(path.to_str().unwrap().replace('/', "\\"))
        .arg(target.to_str().unwrap().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Nonprivileged junction fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn no_replace_reports_existing_file_and_preserves_both_objects() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&target, b"existing target").unwrap();
    let before = snapshot(&fixture.path);
    let client = Client::new();
    let prepared = Prepared::new(&client, &source);
    let state = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let value = conflict(&state);
    assert_eq!(value.target.name, text(OsStr::new("target.bin")));
    assert_eq!(value.target.kind, Kind::File);
    assert_eq!(value.target.metadata.size, Some(15));
    assert_ne!(value.target.identity, prepared.info.source.identity);
    assert_eq!(prepared.query(), state);
    assert_eq!(snapshot(&fixture.path), before);
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn distinct_hard_link_aliases_are_conflicts_and_never_replaceable() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let alias = fixture.path.join("alias.bin");
    fs::write(&source, b"same object under two names").unwrap();
    fs::hard_link(&source, &alias).unwrap();
    assert_eq!(native_identity(&source), native_identity(&alias));
    let before = snapshot(&fixture.path);
    let client = Client::new();
    let prepared = Prepared::new(&client, &source);
    for (index, name) in ["alias.bin", "ALIAS.BIN"].into_iter().enumerate() {
        let attempt = index as u64 + 1;
        let state = prepared.attempt(attempt, OsStr::new(name), RenamePolicy::NoReplace);
        let value = conflict(&state);
        assert_eq!(value.target.identity, prepared.info.source.identity);
        assert!(!value.can_replace);
        refused(
            prepared.request(
                attempt + 1,
                OsStr::new(name),
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: value.token,
                },
            ),
            ErrorCode::Invalid,
        );
        assert_eq!(prepared.query(), state);
        assert_eq!(snapshot(&fixture.path), before);
    }
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn directory_and_file_type_collisions_never_enable_replacement() {
    for (source_directory, target_directory) in [(false, true), (true, false), (true, true)] {
        let fixture = Temp::new();
        let source = fixture.path.join("source");
        let target = fixture.path.join("target");
        if source_directory {
            fs::create_dir(&source).unwrap();
            fs::write(source.join("selected-child"), b"keep selected tree").unwrap();
        } else {
            fs::write(&source, b"selected source").unwrap();
        }
        if target_directory {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("existing-child"), b"keep existing tree").unwrap();
        } else {
            fs::write(&target, b"existing target").unwrap();
        }
        let before = snapshot(&fixture.path);
        let client = Client::new();
        let prepared = Prepared::new(&client, &source);
        if source_directory {
            assert!(!prepared.info.recoverable_replace);
        }
        let state = prepared.attempt(1, OsStr::new("target"), RenamePolicy::NoReplace);
        let value = conflict(&state);
        assert_eq!(
            value.target.kind,
            if target_directory {
                Kind::Directory
            } else {
                Kind::File
            }
        );
        assert!(!value.can_replace);
        refused(
            prepared.request(
                2,
                OsStr::new("target"),
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: value.token,
                },
            ),
            ErrorCode::Invalid,
        );
        assert_eq!(prepared.query(), state);
        assert_eq!(snapshot(&fixture.path), before);
        prepared.close();
        assert_eq!(snapshot(&fixture.path), before);
    }
}

#[test]
fn junction_collisions_cannot_enable_replacement_or_touch_the_referent() {
    for source_is_link in [false, true] {
        let fixture = Temp::new();
        let referent = fixture.path.join("referent");
        let source = fixture.path.join("source");
        let target = fixture.path.join("target");
        fs::create_dir(&referent).unwrap();
        fs::write(referent.join("sentinel.bin"), b"external referent survives").unwrap();
        if source_is_link {
            junction(&source, &referent);
            fs::write(&target, b"existing ordinary target").unwrap();
        } else {
            fs::write(&source, b"selected ordinary source").unwrap();
            junction(&target, &referent);
        }
        let before = snapshot(&fixture.path);
        let client = Client::new();
        let prepared = Prepared::new(&client, &source);
        if source_is_link {
            assert!(matches!(
                prepared.info.source.kind,
                Kind::Symlink | Kind::DirectoryLink
            ));
            assert!(!prepared.info.recoverable_replace);
        }
        let state = prepared.attempt(1, OsStr::new("target"), RenamePolicy::NoReplace);
        let value = conflict(&state);
        if !source_is_link {
            assert!(matches!(
                value.target.kind,
                Kind::Symlink | Kind::DirectoryLink
            ));
        }
        assert!(!value.can_replace);
        refused(
            prepared.request(
                2,
                OsStr::new("target"),
                RenamePolicy::RecoverableCurrentEntry {
                    conflict: value.token,
                },
            ),
            ErrorCode::Invalid,
        );
        assert_eq!(prepared.query(), state);
        assert_eq!(snapshot(&fixture.path), before);
        prepared.close();
        assert_eq!(snapshot(&fixture.path), before);
    }
}

#[test]
fn qualified_replacement_reports_actual_raw_destination_and_exact_displaced_object() {
    let fixture = Temp::new();
    let source = fixture.path.join("source 文档.bin");
    let raw_target = OsString::from_wide(&[
        0x65, 0x78, 0x69, 0x73, 0x74, 0x69, 0x6e, 0x67, 0x20, 0x6587, 0xd800, 0x2e, 0x62, 0x69,
        0x6e,
    ]);
    let destination = fixture.path.join(&raw_target);
    fs::write(&source, b"selected native bytes").unwrap();
    fs::write(&destination, b"displaced native bytes").unwrap();
    let before = FilesBefore::capture(&source, &destination);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    assert!(
        prepared.info.recoverable_replace,
        "Qualified local fixed NTFS must retain recoverable file replacement"
    );
    let first = prepared.attempt(1, &raw_target, RenamePolicy::NoReplace);
    let consent = conflict(&first).clone();
    assert!(consent.can_replace);
    assert_eq!(consent.target.name, text(&raw_target));
    assert_eq!(consent.target.metadata.size, Some(22));
    let state = prepared.attempt(
        2,
        &raw_target,
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    let recovery = before.assert_replaced(&fixture, &state, &prepared.info, &consent);
    assert_eq!(prepared.query(), state);
    prepared.close();
    assert_eq!(native_identity(&destination), before.source_id);
    assert_eq!(native_identity(&recovery), before.target_id);
    assert_eq!(fs::read(&recovery).unwrap(), b"displaced native bytes");
}

#[test]
fn changed_target_identity_requires_fresh_consent_and_preserves_every_original() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    let retained = fixture.path.join("prior-target.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&target, b"first target").unwrap();
    let original_target = native_identity(&target);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    assert!(prepared.info.recoverable_replace);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let old_consent = conflict(&first).clone();
    assert!(old_consent.can_replace);
    fs::rename(&target, &retained).unwrap();
    fs::write(&target, b"new target after consent").unwrap();
    assert_ne!(native_identity(&target), original_target);
    let before = snapshot(&fixture.path);
    let expected = FilesBefore::capture(&source, &target);
    let refreshed = prepared.attempt(
        2,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: old_consent.token,
        },
    );
    let consent = conflict(&refreshed).clone();
    assert_ne!(consent.token, old_consent.token);
    assert_ne!(consent.target.identity, old_consent.target.identity);
    assert_eq!(consent.target.metadata.size, Some(24));
    assert!(consent.can_replace);
    assert_eq!(prepared.query(), refreshed);
    assert_eq!(snapshot(&fixture.path), before);
    refused(
        prepared.request(
            3,
            OsStr::new("target.bin"),
            RenamePolicy::RecoverableCurrentEntry {
                conflict: old_consent.token,
            },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(prepared.query(), refreshed);
    assert_eq!(snapshot(&fixture.path), before);
    let state = prepared.attempt(
        3,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    expected.assert_replaced(&fixture, &state, &prepared.info, &consent);
    assert_eq!(native_identity(&retained), original_target);
    assert_eq!(fs::read(&retained).unwrap(), b"first target");
    prepared.close();
    assert_eq!(native_identity(&retained), original_target);
}

#[test]
fn same_object_target_revision_requires_fresh_consent_even_with_identical_preview_metadata() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&target, b"old target bytes").unwrap();
    make_change_time_older(&target);
    let target_id = native_identity(&target);
    let modified = fs::metadata(&target).unwrap().modified().unwrap();
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    assert!(prepared.info.recoverable_replace);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let old_consent = conflict(&first).clone();
    assert!(old_consent.can_replace);
    fs::write(&target, b"new target bytes").unwrap();
    File::options()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(native_identity(&target), target_id);
    assert_eq!(fs::metadata(&target).unwrap().len(), 16);
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), modified);
    let before = snapshot(&fixture.path);
    let expected = FilesBefore::capture(&source, &target);
    let refreshed = prepared.attempt(
        2,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: old_consent.token,
        },
    );
    let consent = conflict(&refreshed).clone();
    assert_ne!(consent.token, old_consent.token);
    assert_eq!(consent.target.identity, old_consent.target.identity);
    assert_ne!(consent.target.revision, old_consent.target.revision);
    assert_eq!(consent.target.metadata, old_consent.target.metadata);
    assert!(consent.can_replace);
    assert_eq!(snapshot(&fixture.path), before);
    refused(
        prepared.request(
            3,
            OsStr::new("target.bin"),
            RenamePolicy::RecoverableCurrentEntry {
                conflict: old_consent.token,
            },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(prepared.query(), refreshed);
    assert_eq!(snapshot(&fixture.path), before);
    let state = prepared.attempt(
        3,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    let recovery = expected.assert_replaced(&fixture, &state, &prepared.info, &consent);
    prepared.close();
    assert_eq!(native_identity(&recovery), target_id);
    assert_eq!(fs::read(&recovery).unwrap(), b"new target bytes");
}

#[test]
fn source_replacement_after_collision_never_repins_the_new_source() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    let retained = fixture.path.join("prepared-source.bin");
    fs::write(&source, b"prepared source").unwrap();
    fs::write(&target, b"consented target").unwrap();
    let pinned = native_identity(&source);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let consent = conflict(&first);
    assert!(consent.can_replace);
    fs::rename(&source, &retained).unwrap();
    fs::write(&source, b"new source winner").unwrap();
    assert_ne!(native_identity(&source), pinned);
    let before = snapshot(&fixture.path);
    let state = prepared.attempt(
        2,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    source_changed(&state);
    assert_eq!(prepared.query(), state);
    assert_eq!(native_identity(&retained), pinned);
    assert_eq!(snapshot(&fixture.path), before);
    refused(
        prepared.request(3, OsStr::new("another.bin"), RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn parent_replacement_after_collision_never_repins_the_new_parent() {
    let fixture = Temp::new();
    let parent = fixture.path.join("parent");
    let retained = fixture.path.join("prepared-parent");
    fs::create_dir(&retained).unwrap();
    junction(&parent, &retained);
    let source = parent.join("source.bin");
    fs::write(&source, b"prepared source").unwrap();
    fs::write(parent.join("target.bin"), b"consented target").unwrap();
    // Capture the actual pinned directory, not the no-follow junction identity.
    let pinned_parent = native_identity(&retained);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &parent);
    let prepared = Prepared::new(&client, &source);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let consent = conflict(&first);
    assert!(consent.can_replace);
    // Source and consent pins keep the real directory open; swap only its alias.
    fs::remove_dir(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source.bin"), b"new source winner").unwrap();
    fs::write(parent.join("target.bin"), b"new target winner").unwrap();
    assert_ne!(native_identity(&parent), pinned_parent);
    let before = snapshot(&fixture.path);
    let state = prepared.attempt(
        2,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    source_changed(&state);
    assert_eq!(native_identity(&retained), pinned_parent);
    assert_eq!(prepared.query(), state);
    assert_eq!(snapshot(&fixture.path), before);
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn conflict_replay_and_consent_refusals_preserve_the_exact_recorded_attempt() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&target, b"existing target").unwrap();
    let before = snapshot(&fixture.path);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let consent = conflict(&first);
    assert!(consent.can_replace);
    assert_eq!(
        prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace),
        first
    );
    for (attempt, name, policy) in [
        (1, "other.bin", RenamePolicy::NoReplace),
        (
            1,
            "target.bin",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: consent.token,
            },
        ),
        (
            2,
            "target.bin",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: consent.token.checked_add(1).unwrap(),
            },
        ),
        (
            2,
            "other.bin",
            RenamePolicy::RecoverableCurrentEntry {
                conflict: consent.token,
            },
        ),
    ] {
        refused(
            prepared.request(attempt, OsStr::new(name), policy),
            ErrorCode::Invalid,
        );
        assert_eq!(prepared.query(), first);
        assert_eq!(snapshot(&fixture.path), before);
    }
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn committed_attempt_replay_never_renames_a_new_entry_at_the_old_source_name() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let destination = fixture.path.join("destination.bin");
    fs::write(&source, b"prepared source").unwrap();
    let pinned = native_identity(&source);
    let client = Client::new();
    let prepared = Prepared::new(&client, &source);
    let state = prepared.attempt(1, OsStr::new("destination.bin"), RenamePolicy::NoReplace);
    assert!(
        matches!(
            &state.outcome,
            RenameOutcome::Renamed {
                name,
                same_entry: false,
                recovery,
                ..
            } if *name == text(OsStr::new("destination.bin")) && recovery.is_empty()
        ),
        "{state:?}"
    );
    assert_eq!(native_identity(&destination), pinned);
    fs::write(&source, b"new independent winner").unwrap();
    assert_ne!(native_identity(&source), pinned);
    let before_replay = snapshot(&fixture.path);
    assert_eq!(
        prepared.attempt(1, OsStr::new("destination.bin"), RenamePolicy::NoReplace),
        state
    );
    assert_eq!(prepared.query(), state);
    refused(
        prepared.request(1, OsStr::new("other.bin"), RenamePolicy::NoReplace),
        ErrorCode::Invalid,
    );
    refused(
        prepared.request(2, OsStr::new("other.bin"), RenamePolicy::NoReplace),
        ErrorCode::StaleReference,
    );
    assert_eq!(snapshot(&fixture.path), before_replay);
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before_replay);
}

#[test]
fn another_name_abandons_old_consent_and_requires_the_new_conflict_token() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let first_target = fixture.path.join("first.bin");
    let second_target = fixture.path.join("second.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&first_target, b"first untouched target").unwrap();
    fs::write(&second_target, b"second displaced target").unwrap();
    let first_id = native_identity(&first_target);
    let before = snapshot(&fixture.path);
    let expected = FilesBefore::capture(&source, &second_target);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    let first = prepared.attempt(1, OsStr::new("first.bin"), RenamePolicy::NoReplace);
    let old_token = conflict(&first).token;
    let second = prepared.attempt(2, OsStr::new("second.bin"), RenamePolicy::NoReplace);
    let consent = conflict(&second).clone();
    assert_ne!(consent.token, old_token);
    assert!(consent.can_replace);
    assert_eq!(snapshot(&fixture.path), before);
    refused(
        prepared.request(
            3,
            OsStr::new("second.bin"),
            RenamePolicy::RecoverableCurrentEntry {
                conflict: old_token,
            },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(prepared.query(), second);
    assert_eq!(snapshot(&fixture.path), before);
    let state = prepared.attempt(
        3,
        OsStr::new("second.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: consent.token,
        },
    );
    expected.assert_replaced(&fixture, &state, &prepared.info, &consent);
    assert_eq!(native_identity(&first_target), first_id);
    assert_eq!(fs::read(&first_target).unwrap(), b"first untouched target");
    prepared.close();
}

#[test]
fn closing_a_conflict_is_skip_and_leaves_the_complete_namespace_unchanged() {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&target, b"existing target").unwrap();
    let before = snapshot(&fixture.path);
    let client = Client::new();
    let prepared = Prepared::new(&client, &source);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    conflict(&first);
    let handle = prepared.info.handle;
    prepared.close();
    refused(
        client.call(Request::GetRenameState { prepared: handle }),
        ErrorCode::StaleReference,
    );
    assert_eq!(snapshot(&fixture.path), before);
}

fn assert_changed_target_kind_refreshes_conflict(target_is_link: bool) {
    let fixture = Temp::new();
    let source = fixture.path.join("source.bin");
    let target = fixture.path.join("target.bin");
    let retained = fixture.path.join("prior-target.bin");
    let referent = fixture.path.join("referent");
    fs::write(&source, b"selected source remains").unwrap();
    fs::write(&target, b"previous ordinary target remains").unwrap();
    fs::create_dir(&referent).unwrap();
    fs::write(referent.join("sentinel.bin"), b"referent remains").unwrap();
    let source_id = native_identity(&source);
    let old_target_id = native_identity(&target);
    let referent_id = native_identity(&referent);
    let client = Client::new();
    require_fixed_local_ntfs(&client, &fixture.path);
    let prepared = Prepared::new(&client, &source);
    assert!(prepared.info.recoverable_replace);
    let first = prepared.attempt(1, OsStr::new("target.bin"), RenamePolicy::NoReplace);
    let old_consent = conflict(&first).clone();
    assert_eq!(old_consent.target.kind, Kind::File);
    assert!(old_consent.can_replace);

    fs::rename(&target, &retained).unwrap();
    let expected_kind = if target_is_link {
        // The existing junction fixture fails explicitly if ordinary-user
        // creation is unavailable; this regression never skips link coverage.
        junction(&target, &referent);
        Kind::DirectoryLink
    } else {
        fs::create_dir(&target).unwrap();
        fs::write(target.join("new-child.bin"), b"new directory child remains").unwrap();
        Kind::Directory
    };
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert_ne!(metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY, 0);
    assert_eq!(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        target_is_link
    );
    let new_target_id = native_identity(&target);
    assert_ne!(new_target_id, source_id);
    assert_ne!(new_target_id, old_target_id);
    assert_ne!(new_target_id, referent_id);
    let before = snapshot(&fixture.path);

    // A separate public preparation observes the new object's opaque identity.
    // Its independent native file ID above proves this is the replacement entry,
    // including the junction object rather than its referent.
    let observed = Prepared::new(&client, &target);
    assert_eq!(observed.info.source.kind, expected_kind);
    let expected_identity = observed.info.source.identity.clone();
    assert_ne!(expected_identity, old_consent.target.identity);
    observed.close();
    assert_eq!(snapshot(&fixture.path), before);

    let refreshed = prepared.attempt(
        2,
        OsStr::new("target.bin"),
        RenamePolicy::RecoverableCurrentEntry {
            conflict: old_consent.token,
        },
    );
    let current = conflict(&refreshed);
    assert_ne!(current.token, old_consent.token);
    assert_eq!(current.target.kind, expected_kind);
    assert_eq!(current.target.identity, expected_identity);
    assert_eq!(current.target.name, text(OsStr::new("target.bin")));
    assert_eq!(current.target.metadata.size, None);
    assert!(!current.can_replace);
    assert_eq!(prepared.query(), refreshed);
    assert_eq!(snapshot(&fixture.path), before);

    refused(
        prepared.request(
            3,
            OsStr::new("target.bin"),
            RenamePolicy::RecoverableCurrentEntry {
                conflict: current.token,
            },
        ),
        ErrorCode::Invalid,
    );
    assert_eq!(prepared.query(), refreshed);
    assert_eq!(native_identity(&source), source_id);
    assert_eq!(native_identity(&target), new_target_id);
    assert_eq!(native_identity(&retained), old_target_id);
    assert_eq!(native_identity(&referent), referent_id);
    assert_eq!(fs::read(&source).unwrap(), b"selected source remains");
    assert_eq!(
        fs::read(&retained).unwrap(),
        b"previous ordinary target remains"
    );
    assert_eq!(
        fs::read(referent.join("sentinel.bin")).unwrap(),
        b"referent remains"
    );
    if target_is_link {
        assert_eq!(
            fs::read(target.join("sentinel.bin")).unwrap(),
            b"referent remains"
        );
    } else {
        assert_eq!(
            fs::read(target.join("new-child.bin")).unwrap(),
            b"new directory child remains"
        );
    }
    assert_eq!(snapshot(&fixture.path), before);
    prepared.close();
    assert_eq!(snapshot(&fixture.path), before);
}

#[test]
fn changed_target_directory_returns_fresh_nonreplaceable_conflict() {
    assert_changed_target_kind_refreshes_conflict(false);
}

#[test]
fn changed_target_junction_returns_fresh_nonreplaceable_conflict() {
    assert_changed_target_kind_refreshes_conflict(true);
}
