use super::*;
use crate::api::abi::Cancellation;
use crate::ffi_tests::{create_instance, descriptor, destroy, invoke, read, release};
struct Adapter;
struct Directory(usize, bool);
struct Reader(bool);
static TINY_STATS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
impl NativeReader for Reader {
    fn length(&self) -> u64 {
        3
    }
    fn revision(&self) -> &[u8] {
        if self.0 { &[7; 4096] } else { b"revision1" }
    }
    fn validate(&self, _context: &CallContext<'_>) -> Result<(), Failure> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32, _: &CallContext<'_>) -> Result<Vec<u8>, Failure> {
        Ok(b"abc"[offset as usize..(offset + u64::from(length)).min(3) as usize].to_vec())
    }
}
impl NativeAdapter for Adapter {
    type Directory = Directory;
    type Reader = Reader;
    fn decode(value: &Address) -> Result<PathBuf, Failure> {
        Ok(PathBuf::from(std::str::from_utf8(&value.bytes).unwrap()))
    }
    fn encode(path: &Path) -> Address {
        Address {
            encoding: AddressEncoding::Opaque,
            bytes: path.to_str().unwrap().as_bytes().to_vec(),
        }
    }
    fn name(name: &OsStr) -> (NameEncoding, Vec<u8>) {
        (
            NameEncoding::Utf8,
            name.to_str().unwrap().as_bytes().to_vec(),
        )
    }
    fn stat(path: &Path, _context: &CallContext<'_>) -> Result<NativeInfo, Failure> {
        if path.file_name().is_some_and(|name| name == "tiny-reply") {
            TINY_STATS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let file = path
            .file_name()
            .is_some_and(|name| name.to_str().unwrap().starts_with("entry"));
        Ok(NativeInfo {
            kind: if file { Kind::File } else { Kind::Directory },
            metadata: Metadata {
                size: file.then_some(3),
                ..Default::default()
            },
        })
    }
    fn list(path: &Path, _context: &CallContext<'_>) -> Result<Directory, Failure> {
        Ok(Directory(
            0,
            path.file_name().is_some_and(|name| name == "bad-page"),
        ))
    }
    fn next(
        directory: &mut Directory,
        _: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure> {
        if directory.0 == 10000 {
            return Ok(None);
        }
        let name = OsString::from(format!("entry{}", directory.0));
        directory.0 += 1;
        Ok(Some(NativeEntry {
            name,
            info: NativeInfo {
                kind: Kind::File,
                metadata: Metadata {
                    size: Some(3),
                    modified: (directory.1 && directory.0 == 2).then_some(Timestamp {
                        seconds: 0,
                        nanos: 1_000_000_000,
                    }),
                    ..Default::default()
                },
            },
        }))
    }
    fn open_reader(path: &Path, _context: &CallContext<'_>) -> Result<Reader, Failure> {
        Ok(Reader(
            path.file_name().is_some_and(|name| name == "entry-large"),
        ))
    }
    fn volumes(_: &CallContext<'_>) -> Result<Vec<Volume>, Failure> {
        Ok(vec![])
    }
    fn storage_kind(_: &Path, _context: &CallContext<'_>) -> Result<StorageKind, Failure> {
        Ok(StorageKind::Unknown)
    }
    fn space_info(_: &Path, _context: &CallContext<'_>) -> Result<(u64, u64), Failure> {
        Ok((1, 2))
    }
}

#[test]
fn unpublished_resolve_does_not_exhaust_mounts() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(
        &api,
        Config {
            reply_bytes: 10,
            ..Config::default()
        },
    );
    TINY_STATS.store(0, std::sync::atomic::Ordering::Relaxed);
    for _ in 0..70 {
        let (status, buffer) = invoke(
            &api,
            instance,
            Request::Resolve {
                address: Adapter::encode(&std::env::current_dir().unwrap().join("tiny-reply")),
            },
            Cancellation::default(),
        );
        assert_eq!(status, crate::api::abi::RESOURCE_LIMIT);
        assert_eq!(buffer.id, 0);
    }
    assert_eq!(TINY_STATS.load(std::sync::atomic::Ordering::Relaxed), 70);
    assert_eq!(destroy(&api, instance), 0);
}
fn call(api: &crate::api::abi::Api, instance: u64, request: Request) -> Response {
    let (status, buffer) = invoke(api, instance, request, Cancellation::default());
    assert_eq!(status, crate::api::abi::OK);
    let value = read(buffer);
    assert_eq!(release(api, instance, buffer.id), crate::api::abi::OK);
    value
}

#[test]
fn existing_native_adapters_decline_optional_details_without_breaking_storage() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    let address = Adapter::encode(&std::env::current_dir().unwrap());
    for request in [
        Request::VolumeLabel {
            address: address.clone(),
        },
        Request::VolumeInformation {
            address: address.clone(),
        },
    ] {
        assert!(matches!(
            call(&api, instance, request),
            Response::Failed(Failure {
                code: ErrorCode::Unsupported,
                ..
            })
        ));
    }
    assert_eq!(
        call(&api, instance, Request::SpaceInfo { address }),
        Response::SpaceInfo {
            available: 1,
            total: 2
        }
    );
    assert_eq!(destroy(&api, instance), crate::api::abi::OK);
}
fn resolve(api: &crate::api::abi::Api, instance: u64, file: bool) -> (u64, u64) {
    let base = std::env::current_dir().unwrap();
    let path = if file { base.join("entry-file") } else { base };
    let Response::Resolved { mount, node } = call(
        api,
        instance,
        Request::Resolve {
            address: Adapter::encode(&path),
        },
    ) else {
        panic!("Resolve failed")
    };
    (mount, node.id)
}
#[test]
fn first_page_streams_and_wrong_ordinal_cannot_advance_or_close_an_owned_cursor() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    let (mount, node) = resolve(&api, instance, false);
    let Response::Page {
        cursor,
        entries,
        done,
        ..
    } = call(
        &api,
        instance,
        Request::ListBegin {
            mount,
            directory: node,
            max_entries: 2,
        },
    )
    else {
        panic!("Page failed")
    };
    assert_eq!(entries.len(), 2);
    assert!(!done);
    assert_eq!(entries[0].display, "entry0");
    assert!(matches!(
        call(
            &api,
            instance,
            Request::ListNext {
                cursor,
                expected_page: 7,
                max_entries: 2
            }
        ),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
    let Response::Page { entries, .. } = call(
        &api,
        instance,
        Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 2,
        },
    ) else {
        panic!("Next failed")
    };
    assert_eq!(entries[0].display, "entry2");
    assert!(matches!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Failed(Failure {
            code: ErrorCode::Busy,
            ..
        })
    ));
    assert_eq!(
        call(&api, instance, Request::CloseList { cursor }),
        Response::Unit
    );
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}
#[test]
fn independent_reader_handles_keep_mount_busy_and_follow_exact_eof() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    let (mount, node) = resolve(&api, instance, true);
    let Response::Opened { reader: first, .. } =
        call(&api, instance, Request::OpenRead { mount, node })
    else {
        panic!("Open failed")
    };
    let Response::Opened { reader: second, .. } =
        call(&api, instance, Request::OpenRead { mount, node })
    else {
        panic!("Open failed")
    };
    assert_ne!(first, second);
    assert_eq!(
        call(
            &api,
            instance,
            Request::ReadAt {
                reader: first,
                offset: 1,
                length: 3
            }
        ),
        Response::Content {
            bytes: b"bc".to_vec(),
            eof: true
        }
    );
    assert_eq!(
        call(
            &api,
            instance,
            Request::ReadAt {
                reader: second,
                offset: 0,
                length: 1
            }
        ),
        Response::Content {
            bytes: b"a".to_vec(),
            eof: false
        }
    );
    assert_eq!(
        call(
            &api,
            instance,
            Request::ReadAt {
                reader: first,
                offset: 3,
                length: 1
            }
        ),
        Response::Content {
            bytes: vec![],
            eof: true
        }
    );
    assert!(matches!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Failed(Failure {
            code: ErrorCode::Busy,
            ..
        })
    ));
    assert_eq!(
        call(&api, instance, Request::CloseRead { reader: first }),
        Response::Unit
    );
    assert_eq!(
        call(&api, instance, Request::CloseRead { reader: second }),
        Response::Unit
    );
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}

#[test]
fn failed_reader_encoding_releases_native_reader_and_mount_lease() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(
        &api,
        Config {
            reply_bytes: 1024,
            ..Config::default()
        },
    );
    let Response::Resolved { mount, node } = call(
        &api,
        instance,
        Request::Resolve {
            address: Adapter::encode(&std::env::current_dir().unwrap().join("entry-large")),
        },
    ) else {
        panic!("Resolve");
    };
    for _ in 0..70 {
        let (status, output) = invoke(
            &api,
            instance,
            Request::OpenRead {
                mount,
                node: node.id,
            },
            Cancellation::default(),
        );
        assert_eq!(status, crate::api::abi::RESOURCE_LIMIT);
        assert_eq!(output.id, 0);
    }
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}
#[test]
fn failed_page_encoding_invalidates_cursor_instead_of_silently_skipping_entries() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    let Response::Resolved { mount, node } = call(
        &api,
        instance,
        Request::Resolve {
            address: Adapter::encode(&std::env::current_dir().unwrap().join("bad-page")),
        },
    ) else {
        panic!("Resolve");
    };
    let Response::Page { cursor, .. } = call(
        &api,
        instance,
        Request::ListBegin {
            mount,
            directory: node.id,
            max_entries: 1,
        },
    ) else {
        panic!("List");
    };
    let (status, output) = invoke(
        &api,
        instance,
        Request::ListNext {
            cursor,
            expected_page: 1,
            max_entries: 1,
        },
        Cancellation::default(),
    );
    assert_eq!(status, crate::api::abi::INVALID);
    assert_eq!(output.id, 0);
    assert!(matches!(
        call(
            &api,
            instance,
            Request::ListNext {
                cursor,
                expected_page: 2,
                max_entries: 1
            }
        ),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}
#[test]
fn buffer_publication_failure_rolls_back_new_mount_before_next_admission() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    for _ in 0..70 {
        crate::runtime::fail_next_publish::<NativeFilesystem<Adapter>>(instance);
        let (status, output) = invoke(
            &api,
            instance,
            Request::Resolve {
                address: Adapter::encode(&std::env::current_dir().unwrap()),
            },
            Cancellation::default(),
        );
        assert_eq!(status, crate::api::abi::RESOURCE_LIMIT);
        assert_eq!(output.id, 0);
    }
    let (mount, _) = resolve(&api, instance, false);
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}

static CANCEL_CHECKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
extern "C" fn cancel_after_one(_: u64) -> u32 {
    u32::from(CANCEL_CHECKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 2 == 1)
}
#[test]
fn cancellation_after_resolve_rolls_back_mounts_and_cleanup_ignores_cancellation() {
    let api = descriptor::<NativeFilesystem<Adapter>>();
    let instance = create_instance(&api, Config::default());
    CANCEL_CHECKS.store(0, std::sync::atomic::Ordering::Relaxed);
    for _ in 0..70 {
        let (status, buffer) = invoke(
            &api,
            instance,
            Request::Resolve {
                address: Adapter::encode(&std::env::current_dir().unwrap()),
            },
            Cancellation {
                context: 0,
                cancelled: Some(cancel_after_one),
            },
        );
        assert_eq!(status, crate::api::abi::OK);
        assert!(matches!(
            read(buffer),
            Response::Failed(Failure {
                code: ErrorCode::Cancelled,
                ..
            })
        ));
        assert_eq!(release(&api, instance, buffer.id), 0);
    }
    let (mount, _) = resolve(&api, instance, false);
    assert_eq!(
        call(&api, instance, Request::CloseMount { mount }),
        Response::Unit
    );
    assert_eq!(destroy(&api, instance), 0);
}
