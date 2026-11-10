use super::*;

fn mount(root: &Path) -> Mount {
    let container = root.join("archive 文档.zip");
    std::fs::write(&container, b"source identity fixture").unwrap();
    Mount {
        id: 8,
        provider: "archive.fixture".into(),
        provider_version: "1.2.3".into(),
        stable_entry_ids: true,
        format: Format::Zip,
        source: CapturedFile::capture(&container).unwrap(),
        container,
        channel: Mutex::new(Channel {
            input: tempfile::tempfile().unwrap(),
            output: tempfile::tempfile().unwrap(),
            request: 0,
        }),
        supervision: Arc::new(Supervision {
            stop: AtomicBool::new(false),
            deadline: Mutex::new(None),
        }),
        protocol: Protocol::V2,
        staging: Arc::new(Mutex::new(None)),
    }
}

#[test]
fn cache_authority_survives_sessions_but_separates_providers_versions_formats_and_members() {
    let root = tempfile::tempdir().unwrap();
    let mut mount = mount(root.path());
    let revision = mount.source.stable_version();
    let identity = mount
        .persistent_member_identity(2, &revision)
        .unwrap()
        .unwrap();
    mount.id = 9;
    assert_eq!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    assert_ne!(
        mount
            .persistent_member_identity(3, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    mount.provider_version = "1.2.4".into();
    assert_ne!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    mount.provider_version = "1.2.3".into();
    mount.provider = "other.fixture".into();
    assert_ne!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    mount.provider = "archive.fixture".into();
    mount.format = Format::Tar;
    assert_ne!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
    mount.format = Format::Zip;
    mount.container = root.path().join("other archive.zip");
    assert_ne!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .unwrap(),
        identity
    );
}

#[test]
fn absent_capability_changed_sources_and_wrong_revisions_cannot_authorize_cache_reuse() {
    let root = tempfile::tempdir().unwrap();
    let mut mount = mount(root.path());
    let revision = mount.source.stable_version();
    mount.stable_entry_ids = false;
    assert!(
        mount
            .persistent_member_identity(2, &revision)
            .unwrap()
            .is_none()
    );
    mount.stable_entry_ids = true;
    assert!(mount.persistent_member_identity(0, &revision).is_err());
    assert!(
        mount
            .persistent_member_identity(2, b"different revision")
            .is_err()
    );
    std::fs::write(&mount.container, b"changed source").unwrap();
    assert!(mount.persistent_member_identity(2, &revision).is_err());
}
