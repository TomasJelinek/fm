//! Observe the native ABI boundary while exercising the real cache and public capture wrapper.
use super::*;
use crate::{Request, Response, Transport, test_path};
use midnight_filesystem_plugin_api::{ErrorCode, Kind, Metadata, Node, abi};
use std::collections::HashMap;

const BYTES: &[u8] = b"first-second";

#[derive(Debug, Default)]
struct State {
    requests: Vec<Request>,
    readers: HashMap<u64, (u64, Vec<u8>)>,
    next_reader: u64,
    revision: Vec<u8>,
    length: u64,
    change_after_read: bool,
}

#[derive(Debug, Default)]
struct ContentTransport(Mutex<State>);

impl Transport for ContentTransport {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        let mut state = lock(&self.0);
        state.requests.push(request.clone());
        match request {
            Request::Resolve { address } => Ok(Response::Resolved {
                mount: 1,
                node: Node {
                    id: 2,
                    address: address.clone(),
                    name_encoding: test_path::encoding(),
                    raw_name: test_path::name("archive.7z"),
                    display: "archive.7z".into(),
                    kind: Kind::File,
                    metadata: Metadata {
                        size: Some(state.length),
                        ..Metadata::default()
                    },
                },
            }),
            Request::OpenRead { mount: 1, node: 2 } => {
                state.next_reader += 1;
                let reader = state.next_reader;
                let length = state.length;
                let revision = state.revision.clone();
                state.readers.insert(reader, (length, revision.clone()));
                Ok(Response::Opened {
                    reader,
                    length,
                    revision,
                })
            }
            Request::ValidateRead { reader } => {
                validate(&state, *reader)?;
                Ok(Response::Unit)
            }
            Request::ReadAt {
                reader,
                offset,
                length,
            } => {
                validate(&state, *reader)?;
                let end = (*offset + u64::from(*length)).min(state.length);
                let bytes = BYTES[*offset as usize..end as usize].to_vec();
                let eof = end == state.length;
                // Model the required native SDK contract: native/content.rs validates
                // before and after reading, as documented in NATIVE_PLUGIN_PROTOCOL.md.
                if state.change_after_read {
                    state.revision = vec![2];
                }
                validate(&state, *reader)?;
                Ok(Response::Content { bytes, eof })
            }
            Request::CloseRead { reader } => {
                assert!(state.readers.remove(reader).is_some());
                Ok(Response::Unit)
            }
            Request::CloseMount { mount: 1 } => Ok(Response::Unit),
            _ => panic!("Unexpected native content request: {request:?}"),
        }
    }
}

fn validate(state: &State, reader: u64) -> Result<(), Failure> {
    if state.readers.get(&reader) != Some(&(state.length, state.revision.clone())) {
        return Err(Failure::new(
            ErrorCode::SourceChanged,
            "Native source changed",
        ));
    }
    Ok(())
}

fn fixture() -> (Arc<CaptureCache>, Arc<ContentTransport>, PathBuf) {
    let transport = Arc::new(ContentTransport(Mutex::new(State {
        length: BYTES.len() as u64,
        revision: vec![1],
        ..State::default()
    })));
    let cache = CaptureCache::new(NativeProvider {
        transport: transport.clone(),
        capabilities: abi::CAP_NATIVE_ADDRESS | abi::CAP_READ,
    });
    let path = std::env::temp_dir().join("cache-content-tests/archive.7z");
    (cache, transport, path)
}

#[test]
fn resident_reads_use_native_validation_without_extra_requests() {
    let (cache, transport, path) = fixture();
    let captured = cache.capture(&path, &|| false).unwrap();
    assert_eq!(captured.read_at(0, 4).unwrap(), b"firs");
    lock(&transport.0).requests.clear();
    for (offset, bytes) in [(4, b"t-se"), (8, b"cond"), (0, b"firs")] {
        assert_eq!(captured.read_at(offset, 4).unwrap(), bytes);
    }
    let state = lock(&transport.0);
    let validations = state
        .requests
        .iter()
        .filter(|request| matches!(request, Request::ValidateRead { .. }))
        .count();
    let reads = state
        .requests
        .iter()
        .filter(|request| matches!(request, Request::ReadAt { .. }))
        .count();
    eprintln!("three resident chunks: ValidateRead={validations}, ReadAt={reads}");
    assert_eq!(reads, 3);
    assert_eq!(
        validations, 0,
        "native ReadAt already validates the pinned source before and after reading"
    );
    assert!(!state.requests.iter().any(|request| matches!(
        request,
        Request::OpenRead { .. } | Request::CloseRead { .. }
    )));
}

#[test]
fn resident_change_is_rejected_by_the_native_read_contract() {
    for change_revision in [false, true] {
        let (cache, transport, path) = fixture();
        let captured = cache.capture(&path, &|| false).unwrap();
        captured.read_at(0, 4).unwrap();
        {
            let mut state = lock(&transport.0);
            state.requests.clear();
            if change_revision {
                state.revision = vec![2];
            } else {
                state.length = 11;
            }
        }
        assert!(captured.read_at(0, 4).is_err());
        assert!(matches!(
            lock(&transport.0).requests.as_slice(),
            [Request::ReadAt { .. }]
        ));
    }
}

#[test]
fn change_during_native_read_prevents_byte_publication() {
    let (cache, transport, path) = fixture();
    let captured = cache.capture(&path, &|| false).unwrap();
    lock(&transport.0).change_after_read = true;
    assert!(captured.read_at(0, 4).is_err());
    let state = lock(&transport.0);
    assert!(matches!(
        state.requests.last(),
        Some(Request::ReadAt { .. })
    ));
    assert!(
        state
            .requests
            .iter()
            .any(|request| matches!(request, Request::ReadAt { .. }))
    );
}

#[test]
fn explicit_revalidation_still_calls_the_native_validator() {
    let (cache, transport, path) = fixture();
    let captured = cache.capture(&path, &|| false).unwrap();
    captured.read_at(0, 4).unwrap();
    lock(&transport.0).requests.clear();
    captured.revalidate().unwrap();
    {
        let mut state = lock(&transport.0);
        assert!(matches!(
            state.requests.as_slice(),
            [Request::ValidateRead { .. }]
        ));
        state.requests.clear();
        state.revision = vec![2];
    }
    assert!(captured.revalidate().is_err());
    assert!(matches!(
        lock(&transport.0).requests.as_slice(),
        [Request::ValidateRead { .. }]
    ));
}

#[test]
fn eof_reads_validate_without_issuing_native_reads() {
    for (length, offset) in [(12, 12), (12, 13), (0, 0), (0, 1)] {
        let (cache, transport, path) = fixture();
        lock(&transport.0).length = length;
        let captured = cache.capture(&path, &|| false).unwrap();
        captured.revalidate().unwrap();
        lock(&transport.0).requests.clear();
        assert!(captured.read_at(offset, 4).unwrap().is_empty());
        {
            let mut state = lock(&transport.0);
            assert!(matches!(
                state.requests.as_slice(),
                [Request::ValidateRead { .. }]
            ));
            state.requests.clear();
            state.revision = vec![2];
        }
        assert!(captured.read_at(offset, 4).is_err());
        assert!(matches!(
            lock(&transport.0).requests.as_slice(),
            [Request::ValidateRead { .. }]
        ));
    }
}

#[test]
fn native_checked_reads_reject_metadata_different_from_the_original_lease() {
    let (cache, transport, path) = fixture();
    let source = cache.provider.capture_source(&path, &|| false).unwrap();
    lock(&transport.0).requests.clear();
    for (expected_length, expected_version) in [(11, &[1][..]), (12, &[2][..])] {
        assert!(
            source
                .read_checked(0, 4, expected_length, expected_version)
                .is_err()
        );
    }
    assert!(lock(&transport.0).requests.is_empty());
}

#[test]
fn snapshot_checked_reads_forward_original_expected_metadata() {
    let (cache, transport, path) = fixture();
    let source = Snapshot {
        cache,
        id: 1,
        path,
        length: 12,
        revision: vec![1],
        name: "archive.7z".into(),
        initial: Mutex::new(None),
    };
    for (expected_length, expected_version) in [(11, &[1][..]), (12, &[2][..])] {
        assert!(
            source
                .read_checked(0, 4, expected_length, expected_version)
                .is_err()
        );
    }
    assert!(!lock(&transport.0).requests.iter().any(|request| matches!(
        request,
        Request::ValidateRead { .. } | Request::ReadAt { .. }
    )));
}

#[test]
fn evicted_readers_must_match_original_revision_and_length() {
    for change_revision in [false, true] {
        let (cache, transport, path) = fixture();
        let snapshots: Vec<_> = (0..17)
            .map(|_| cache.capture(&path, &|| false).unwrap())
            .collect();
        for captured in &snapshots {
            assert_eq!(captured.read_at(0, 4).unwrap(), b"firs");
            assert!(lock(&transport.0).readers.len() <= 16);
        }
        {
            let mut state = lock(&transport.0);
            assert_eq!(state.readers.len(), 16);
            state.requests.clear();
            if change_revision {
                state.revision = vec![2];
            } else {
                state.length = 11;
            }
        }
        assert!(snapshots[0].read_at(0, 4).is_err());
        {
            let state = lock(&transport.0);
            assert!(
                state
                    .requests
                    .iter()
                    .any(|request| matches!(request, Request::OpenRead { .. }))
            );
            assert!(
                !state
                    .requests
                    .iter()
                    .any(|request| matches!(request, Request::ReadAt { .. }))
            );
            assert!(state.requests.ends_with(&[
                Request::CloseRead { reader: 35 },
                Request::CloseMount { mount: 1 },
            ]));
            assert_eq!(state.readers.len(), 16);
        }
        drop(snapshots);
        assert!(lock(&transport.0).readers.is_empty());
    }
}
