//! Archive container captures (`capture_matching`) under `ARCHIVE_REVISIONS`, through the
//! installed provider's capture cache. The emulated plugin readers check every request
//! against their own revision, change time included, as the native plugins' readers do.
use super::*;
use crate::{Request, Response, Transport, test_path};
use midnight_filesystem_plugin_api::{ErrorCode, Kind, Metadata, Node, abi};
use midnight_plugin_host::CapturedFile;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(windows)]
#[path = "installed_archive_capture_windows_tests.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "installed_archive_capture_linux_tests.rs"]
mod platform;
use platform::{LAYOUT, tag};

const BYTES: &[u8] = b"archive bytes";

fn revision(prefix: &[u8], write: u8, change: u8) -> Vec<u8> {
    let mut revision = [prefix, &vec![1; LAYOUT.2]].concat();
    revision[prefix.len() + 32] = write;
    *revision.last_mut().unwrap() = change;
    revision
}

fn node(address: &midnight_filesystem_plugin_api::Address, length: u64) -> Node {
    Node {
        id: 2,
        address: address.clone(),
        name_encoding: test_path::encoding(),
        raw_name: test_path::name("archive.tar"),
        display: "archive.tar".into(),
        kind: Kind::File,
        metadata: Metadata {
            size: Some(length),
            ..Metadata::default()
        },
    }
}

fn changed() -> Response {
    Response::Failed(Failure::new(
        ErrorCode::SourceChanged,
        "Captured native content changed.",
    ))
}

fn provider(transport: Arc<dyn Transport>) -> InstalledProvider {
    let installed = InstalledProvider::default();
    installed
        .provider
        .set(Ok(crate::NativeProvider {
            transport,
            capabilities: abi::CAP_NATIVE_ADDRESS | abi::CAP_READ,
        }))
        .unwrap();
    installed
}

/// The file an emulated plugin serves: its current length and revision, and whether its
/// path can be resolved at all. Each reader keeps the length and revision it was opened with.
#[derive(Debug, Default)]
struct State {
    length: u64,
    revision: Vec<u8>,
    unreachable: bool,
    readers: HashMap<u64, (u64, Vec<u8>)>,
    next: u64,
}

#[derive(Debug)]
struct Plugin(Mutex<State>);
impl Plugin {
    fn change(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.0.lock().unwrap());
    }
}
impl Transport for Plugin {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        let mut state = self.0.lock().unwrap();
        let current = (state.length, state.revision.clone());
        Ok(match request {
            Request::Resolve { .. } if state.unreachable => {
                Response::Failed(Failure::new(ErrorCode::Io, "The path is unreachable."))
            }
            Request::Resolve { address } => Response::Resolved {
                mount: 1,
                node: node(address, state.length),
            },
            Request::OpenRead { mount: 1, node: 2 } => {
                state.next += 1;
                let reader = state.next;
                state.readers.insert(reader, current.clone());
                Response::Opened {
                    reader,
                    length: current.0,
                    revision: current.1,
                }
            }
            Request::ValidateRead { reader } if state.readers[reader] == current => Response::Unit,
            Request::ReadAt {
                reader,
                offset,
                length,
            } if state.readers[reader] == current => {
                let end = (offset + u64::from(*length)).min(current.0);
                Response::Content {
                    bytes: BYTES[*offset as usize..end as usize].to_vec(),
                    eof: end == current.0,
                }
            }
            Request::ValidateRead { .. } | Request::ReadAt { .. } => changed(),
            Request::CloseRead { reader } => {
                assert!(state.readers.remove(reader).is_some());
                Response::Unit
            }
            Request::CloseMount { mount: 1 } => Response::Unit,
            _ => panic!("Unexpected native content request: {request:?}"),
        })
    }
}

fn fake(revision: Vec<u8>) -> (InstalledProvider, Arc<Plugin>, PathBuf) {
    let plugin = Arc::new(Plugin(Mutex::new(State {
        length: BYTES.len() as u64,
        revision,
        ..State::default()
    })));
    let installed = provider(plugin.clone());
    let path = std::env::temp_dir().join("archive capture tests/archive.tar");
    (installed, plugin, path)
}

/// A restored tab or Hot Path: its saved revision still names an archive whose
/// change time alone moved since (both revision layouts), and nothing else.
#[test]
fn a_saved_archive_revision_matches_after_only_its_change_time_moved() {
    let (current, legacy, _, length) = LAYOUT;
    for prefix in [current, legacy] {
        let (installed, _, path) = fake(revision(current, 1, 2));
        let archive = installed
            .capture_matching(&path, Some(&revision(prefix, 1, 1)), &|| false)
            .unwrap();
        assert_eq!(archive.stable_version(), revision(current, 1, 2));
        assert_eq!(archive.read_at(0, 7).unwrap(), b"archive");
        let mut identity = revision(prefix, 1, 1);
        identity[prefix.len()] ^= 1;
        let mut size = revision(prefix, 1, 1);
        size[prefix.len() + length] ^= 1;
        for saved in [revision(prefix, 2, 2), identity, size] {
            let failure = installed
                .capture_matching(&path, Some(&saved), &|| false)
                .unwrap_err();
            assert_eq!(failure.code, ErrorCode::SourceChanged);
        }
    }
}

/// The reported failure: a copy out of an archive, or its listing, after only the change
/// time of the archive moved. An exact capture fails; the archive capture reopens its
/// reader and reads on, also after a failed reopen of an unreachable path, which is not a
/// lasting change. A new write time, length or identity is one, and fails for good.
#[test]
fn an_archive_capture_reads_on_after_only_the_change_time_moved() {
    let current = LAYOUT.0;
    let (installed, plugin, path) = fake(revision(current, 1, 1));
    let archive = installed.capture_matching(&path, None, &|| false).unwrap();
    let exact = installed.capture(&path, &|| false).unwrap();
    assert_eq!(archive.read_at(0, 7).unwrap(), b"archive");
    assert_eq!(exact.read_at(0, 7).unwrap(), b"archive");
    plugin.change(|state| state.revision = revision(current, 1, 2));
    assert!(exact.revalidate().is_err());
    assert!(exact.read_at(0, 7).is_err());
    archive.revalidate().unwrap();
    assert_eq!(archive.read_at(8, 5).unwrap(), b"bytes");
    assert_eq!(archive.stable_version(), revision(current, 1, 1));
    plugin.change(|state| {
        state.revision = revision(current, 1, 3);
        state.unreachable = true;
    });
    assert!(archive.revalidate().is_err());
    assert!(
        !archive.changed_since_capture(),
        "an unreachable file is no lasting change"
    );
    plugin.change(|state| state.unreachable = false);
    archive.revalidate().unwrap();
    assert!(!archive.changed_since_capture());
    for change in 0..3 {
        let (installed, plugin, path) = fake(revision(current, 1, 1));
        let archive = installed.capture_matching(&path, None, &|| false).unwrap();
        assert_eq!(archive.read_at(0, 7).unwrap(), b"archive");
        plugin.change(|state| match change {
            0 => state.revision = revision(current, 2, 2),
            1 => state.length -= 1,
            _ => state.revision[current.len()] ^= 1,
        });
        assert!(archive.read_at(0, 7).is_err(), "change {change}");
        assert!(archive.revalidate().is_err(), "change {change}");
        assert!(archive.changed_since_capture(), "change {change}");
    }
}

/// Readers over a real file: each pins an exact in-process capture taken when it is
/// opened and reports that revision in the plugin's layout, which has the same payload.
#[derive(Debug)]
struct Files {
    path: PathBuf,
    readers: Mutex<HashMap<u64, CapturedFile>>,
    next: AtomicU64,
}
impl Transport for Files {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        let mut readers = self.readers.lock().unwrap();
        Ok(match request {
            Request::Resolve { address } => Response::Resolved {
                mount: 1,
                node: node(address, std::fs::metadata(&self.path).unwrap().len()),
            },
            Request::OpenRead { mount: 1, node: 2 } => {
                let file = CapturedFile::capture(&self.path).unwrap();
                let reader = self.next.fetch_add(1, Ordering::Relaxed) + 1;
                let response = Response::Opened {
                    reader,
                    length: file.length(),
                    revision: plugin_revision(&file),
                };
                readers.insert(reader, file);
                response
            }
            Request::ValidateRead { reader } => match readers[reader].revalidate() {
                Ok(()) => Response::Unit,
                Err(_) => changed(),
            },
            Request::ReadAt {
                reader,
                offset,
                length,
            } => match readers[reader].read_at(*offset, *length) {
                Ok(bytes) => Response::Content {
                    eof: offset + bytes.len() as u64 == readers[reader].length(),
                    bytes,
                },
                Err(_) => changed(),
            },
            Request::CloseRead { reader } => {
                assert!(readers.remove(reader).is_some());
                Response::Unit
            }
            Request::CloseMount { mount: 1 } => Response::Unit,
            _ => panic!("Unexpected native content request: {request:?}"),
        })
    }
}

fn plugin_revision(file: &CapturedFile) -> Vec<u8> {
    let (current, legacy, _, _) = LAYOUT;
    let version = file.stable_version();
    [current, version.strip_prefix(legacy).unwrap()].concat()
}

/// Tag the file (`platform::tag`) until its change time has moved, which happens at the
/// file system's clock tick at the latest; each pass is a real metadata change, without
/// sleeping.
fn move_change_time(path: &Path) {
    let before = CapturedFile::capture(path).unwrap().stable_version();
    let deadline = Instant::now() + Duration::from_secs(10);
    while CapturedFile::capture(path).unwrap().stable_version() == before {
        assert!(Instant::now() < deadline, "the change time did not move");
        tag(path);
    }
}

/// The reported failure on this machine's file system: after the archive was tagged
/// (only its change time moved), its running capture reads on and its saved revision
/// still restores it, while an exact capture fails. A rewrite of the same length, with a
/// new write time, still fails both, for good.
#[test]
fn an_archive_capture_reads_on_after_the_archive_was_tagged() {
    let root = tempfile::tempdir().unwrap();
    // No extension: endpoint protection tags some fresh file types on its own.
    let path = root.path().join("archive");
    std::fs::write(&path, BYTES).unwrap();
    let installed = provider(Arc::new(Files {
        path: path.clone(),
        readers: Mutex::default(),
        next: AtomicU64::new(0),
    }));
    let archive = installed.capture_matching(&path, None, &|| false).unwrap();
    let saved = archive.stable_version();
    let exact = installed.capture(&path, &|| false).unwrap();
    assert_eq!(archive.read_at(0, 7).unwrap(), b"archive");
    assert_eq!(exact.read_at(0, 7).unwrap(), b"archive");
    move_change_time(&path);
    assert!(exact.revalidate().is_err());
    assert!(exact.read_at(0, 7).is_err());
    archive.revalidate().unwrap();
    assert_eq!(archive.read_at(8, 5).unwrap(), b"bytes");
    assert_eq!(archive.stable_version(), saved);
    let restored = installed
        .capture_matching(&path, Some(&saved), &|| false)
        .unwrap();
    assert_ne!(
        restored.stable_version(),
        saved,
        "the current revision is kept"
    );
    assert_eq!(restored.read_at(0, 7).unwrap(), b"archive");
    std::fs::write(&path, b"changed bytes").unwrap();
    assert!(archive.revalidate().is_err());
    assert!(archive.read_at(0, 7).is_err());
    assert!(archive.changed_since_capture());
    let failure = installed
        .capture_matching(&path, Some(&saved), &|| false)
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::SourceChanged);
}
