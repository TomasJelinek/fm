use super::*;
use std::time::{Duration, Instant};
fn context(cancelled: &(dyn Fn() -> bool + Sync)) -> Context<'_> {
    Context {
        cancelled,
        deadline: Instant::now() + Duration::from_secs(2),
    }
}
#[test]
fn accepts_discovery_server_and_share_addresses_without_changing_identity() {
    for (uri, kind) in [
        ("network:///", NodeKind::Root),
        ("dns-sd://local/", NodeKind::Container),
        ("smb://NAS/", NodeKind::Server),
        ("smb://nas/My%20Share/", NodeKind::Share),
    ] {
        let record = Linux::manual(uri).unwrap();
        assert_eq!(record.target.key, uri.as_bytes());
        assert_eq!(record.target.kind, kind);
        Linux::validate(&record.target).unwrap();
    }
}

#[test]
fn wsdd_discovery_namespace_accepts_only_the_exact_root_container() {
    let root = Linux::manual("wsdd:///").unwrap();
    assert_eq!(root.target.key, b"wsdd:///");
    assert_eq!(root.target.kind, NodeKind::Container);
    Linux::validate(&root.target).unwrap();
    for invalid in [
        "wsdd://",
        "wsdd:////",
        "wsdd://host/",
        "wsdd:///device",
        "wsdd://user:secret@host/",
        "wsdd://%75ser%40host/",
        "wsdd:///%00",
        "wsdd:///%2f",
        "wsdd:///%2e%2e",
        "wsdd:///?query=value",
        "wsdd:///#fragment",
        "wsdd:///\n",
        "WSDD:///",
    ] {
        assert_eq!(
            Linux::manual(invalid).unwrap_err().code,
            ErrorCode::Invalid,
            "{invalid:?}",
        );
    }
}
#[test]
fn rejects_credentials_unsupported_schemes_and_ambiguous_addresses() {
    for uri in [
        "smb://user:secret@host/share",
        "smb://user%40host/share",
        "file:///tmp",
        "https://host/",
        "smb://host/share?password=x",
        "smb://host/share#x",
        "smb://host/%00",
        "smb://host/%2f",
        "smb://host/../share",
        "smb://host/%2e%2e/share",
        "network://host/",
        "dns-sd://local/%",
        "smb://host/\n",
        "smb:host",
        "smb://",
    ] {
        assert_eq!(
            Linux::manual(uri).unwrap_err().code,
            ErrorCode::Invalid,
            "{uri:?}"
        );
    }
}
#[test]
fn kind_must_agree_with_uri() {
    let target = Target {
        kind: NodeKind::Share,
        key: b"smb://host/".to_vec(),
    };
    assert_eq!(
        Linux::validate(&target).unwrap_err().code,
        ErrorCode::Invalid
    );
}
#[test]
fn cancellation_and_deadline_are_checked_before_loading_backend() {
    let cancelled = || true;
    assert!(matches!(
        Linux::begin(&Linux::root().target, &context(&cancelled)),
        Err(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    let not_cancelled = || false;
    let expired = Context {
        cancelled: &not_cancelled,
        deadline: Instant::now() - Duration::from_secs(1),
    };
    assert!(matches!(
        Linux::begin(&Linux::root().target, &expired),
        Err(Failure {
            code: ErrorCode::TimedOut,
            ..
        })
    ));
}
#[test]
fn bare_host_is_an_explicit_smb_server_location() {
    let record = Linux::manual("My-NAS").unwrap();
    assert_eq!(record.target.key, b"smb://My-NAS/");
    assert_eq!(record.target.kind, NodeKind::Server);
}

#[test]
fn accepts_authority_only_smb_uri_and_long_share_names_without_truncation() {
    let server = Linux::manual("smb://nas.example").unwrap();
    assert_eq!(server.target.key, b"smb://nas.example");
    assert_eq!(server.target.kind, NodeKind::Server);
    let long = format!("smb://host/{}/", "x".repeat(4096));
    assert_eq!(Linux::manual(&long).unwrap().target.key, long.as_bytes());
}

#[test]
fn enforces_admission_limits_and_rejects_native_path_nul() {
    let huge = format!("smb://host/{}/", "x".repeat(65536));
    assert_eq!(
        Linux::manual(&huge).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
    let target = Linux::manual("smb://host/share/").unwrap().target;
    let fake = Fake {
        path: Some(b"/run/a\0b".to_vec()),
        ..Default::default()
    };
    assert_eq!(
        backend::activate(&fake, &target, &context(&|| false))
            .unwrap_err()
            .code,
        ErrorCode::Invalid
    );
}

#[test]
fn cancellation_during_iteration_preserves_owned_cursor_for_cleanup() {
    let fake = Fake::default();
    let mut cursor = backend::begin(&fake, &Linux::root().target, &context(&|| false)).unwrap();
    assert_eq!(
        backend::next(&fake, &mut cursor, &context(&|| true))
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(fake.closes.get(), 0);
    drop(cursor);
    assert_eq!(fake.closes.get(), 1);
}

use backend::{Backend, Entry};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};
struct Fake {
    entries: RefCell<VecDeque<Entry>>,
    error: Option<ErrorCode>,
    mounts: Cell<usize>,
    path: Option<Vec<u8>>,
    closes: Rc<Cell<usize>>,
}
struct FakeCursor {
    entries: VecDeque<Entry>,
    closes: Rc<Cell<usize>>,
}
impl Drop for FakeCursor {
    fn drop(&mut self) {
        self.closes.set(self.closes.get() + 1);
    }
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            entries: RefCell::default(),
            error: None,
            mounts: Cell::new(0),
            path: None,
            closes: Rc::new(Cell::new(0)),
        }
    }
}
impl Backend for Fake {
    type Cursor = FakeCursor;
    fn begin(&self, _uri: &[u8], _: &Context<'_>) -> Result<FakeCursor, Failure> {
        if let Some(code) = self.error {
            return Err(Failure::new(code, "Fixture failure."));
        }
        Ok(FakeCursor {
            entries: std::mem::take(&mut *self.entries.borrow_mut()),
            closes: self.closes.clone(),
        })
    }
    fn next(&self, cursor: &mut FakeCursor, _: &Context<'_>) -> Result<Option<Entry>, Failure> {
        Ok(cursor.entries.pop_front())
    }
    fn activate(&self, _: &Target, _: &Context<'_>) -> Result<Option<Vec<u8>>, Failure> {
        self.mounts.set(self.mounts.get() + 1);
        if let Some(code) = self.error {
            Err(Failure::new(code, "Fixture failure."))
        } else {
            Ok(self.path.clone())
        }
    }
}
#[test]
fn listing_uses_exact_target_uri_and_preserves_unicode_display() {
    let fake = Fake::default();
    fake.entries.borrow_mut().push_back(Entry {
        uri: b"smb://NAS/R%26D/".to_vec(),
        display: "Vývoj — 研究".into(),
    });
    let mut cursor = backend::begin(&fake, &Linux::root().target, &context(&|| false)).unwrap();
    let row = backend::next(&fake, &mut cursor, &context(&|| false))
        .unwrap()
        .unwrap();
    assert_eq!(row.target.key, b"smb://NAS/R%26D/");
    assert_eq!(row.display, "Vývoj — 研究");
    assert_eq!(row.target.kind, NodeKind::Share);
    assert!(
        backend::next(&fake, &mut cursor, &context(&|| false))
            .unwrap()
            .is_none()
    );
    assert_eq!(fake.mounts.get(), 0);
    drop(cursor);
    assert_eq!(fake.closes.get(), 1);
}
#[test]
fn unmounted_listing_never_activates_or_turns_into_empty_success() {
    let fake = Fake {
        error: Some(ErrorCode::MountRequired),
        ..Default::default()
    };
    assert!(matches!(
        backend::begin(&fake, &Linux::root().target, &context(&|| false)),
        Err(Failure {
            code: ErrorCode::MountRequired,
            ..
        })
    ));
    assert_eq!(fake.mounts.get(), 0);
    assert_eq!(fake.closes.get(), 0);
}
#[test]
fn discovery_filters_credential_and_unsupported_targets_without_exposing_them() {
    let fake = Fake::default();
    for uri in ["smb://secret@host/share/", "https://host/", "smb://safe/"] {
        fake.entries.borrow_mut().push_back(Entry {
            uri: uri.as_bytes().to_vec(),
            display: "Row".into(),
        });
    }
    let mut cursor = backend::begin(&fake, &Linux::root().target, &context(&|| false)).unwrap();
    assert_eq!(
        backend::next(&fake, &mut cursor, &context(&|| false))
            .unwrap()
            .unwrap()
            .target
            .key,
        b"smb://safe/"
    );
}

#[test]
fn discovery_keeps_wsdd_root_while_filtering_other_wsdd_addresses() {
    let fake = Fake::default();
    for uri in [
        "wsdd://user:secret@host/",
        "wsdd:///device",
        "wsdd:////",
        "wsdd:///",
    ] {
        fake.entries.borrow_mut().push_back(Entry {
            uri: uri.as_bytes().to_vec(),
            display: "WS-Discovery Network".into(),
        });
    }
    let mut cursor = backend::begin(&fake, &Linux::root().target, &context(&|| false)).unwrap();
    let root = backend::next(&fake, &mut cursor, &context(&|| false))
        .unwrap()
        .expect("the supported WS-Discovery root must survive discovery filtering");
    assert_eq!(root.target.key, b"wsdd:///");
    assert_eq!(root.target.kind, NodeKind::Container);
    assert_eq!(root.display, "WS-Discovery Network");
    assert!(
        backend::next(&fake, &mut cursor, &context(&|| false))
            .unwrap()
            .is_none()
    );
    assert_eq!(fake.mounts.get(), 0);
}
#[test]
fn explicit_namespace_activation_returns_only_provider_ready_marker() {
    let fake = Fake::default();
    let target = Linux::root().target;
    assert_eq!(
        backend::activate(&fake, &target, &context(&|| false)).unwrap(),
        api::ActivationTarget::WithinProvider(api::Address {
            encoding: api::AddressEncoding::Opaque,
            bytes: target.key
        })
    );
    assert_eq!(fake.mounts.get(), 1);
}
#[test]
fn share_activation_requires_actual_native_path_and_preserves_raw_bytes() {
    let target = Linux::manual("smb://host/share/").unwrap().target;
    let missing = Fake::default();
    assert_eq!(
        backend::activate(&missing, &target, &context(&|| false))
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    let raw = b"/run/user/1000/gvfs/smb-share:server=host,share=\xff".to_vec();
    let mounted = Fake {
        path: Some(raw.clone()),
        ..Default::default()
    };
    assert_eq!(
        backend::activate(&mounted, &target, &context(&|| false)).unwrap(),
        api::ActivationTarget::Native(api::Address {
            encoding: api::AddressEncoding::UnixBytes,
            bytes: raw
        })
    );
    let relative = Fake {
        path: Some(b"relative/path".to_vec()),
        ..Default::default()
    };
    assert_eq!(
        backend::activate(&relative, &target, &context(&|| false))
            .unwrap_err()
            .code,
        ErrorCode::Invalid
    );
}
#[test]
fn activation_preserves_authentication_offline_and_cancelled_states() {
    let target = Linux::manual("smb://host/share/").unwrap().target;
    for code in [
        ErrorCode::AuthenticationRequired,
        ErrorCode::Offline,
        ErrorCode::PermissionDenied,
        ErrorCode::Unavailable,
        ErrorCode::TimedOut,
    ] {
        let fake = Fake {
            error: Some(code),
            ..Default::default()
        };
        assert_eq!(
            backend::activate(&fake, &target, &context(&|| false))
                .unwrap_err()
                .code,
            code
        );
    }
    let fake = Fake::default();
    assert_eq!(
        backend::activate(&fake, &target, &context(&|| true))
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert_eq!(fake.mounts.get(), 0);
}
