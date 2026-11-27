use super::{Cancellation, Fixture, LocalFileSystem, TransferService, verified_copy_and_conflict};
use midnight_platform_windows::test_network_ids::{Response, Scope};
use std::{fs, path::Path};

/// Each direction has its own source and destination. The copy dialog uses full
/// SHA-256 and preserves timestamps; the second copy resolves an occupied name.
async fn check_directions(local: &Path, share: &Path, response: Option<Response>) {
    let _scope = response.map(|reply| Scope::new(share, reply).unwrap());
    if response.is_some() {
        // Exercise the fallback even on CI's NTFS directory or on a modern SMB
        // server. Move, which still requires an actual file ID, witnesses the probe.
        let provider = LocalFileSystem::new(None);
        assert!(
            provider
                .prepare_move_destination(share, &Cancellation::default())
                .is_err_and(|failure| failure.kind == std::io::ErrorKind::Unsupported)
        );
    }
    for (name, from, to) in [
        ("upload", local, share),
        ("download", share, local),
        ("within-share", share, share),
    ] {
        let source = from.join(format!("{name}-input"));
        let target = to.join(format!("{name}-output"));
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        verified_copy_and_conflict(&source, &target).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_handles_missing_and_zero_network_ids_in_both_directions() {
    for reply in [Response::Unsupported, Response::AllZero] {
        let fixture = Fixture::new("network-ids");
        check_directions(&fixture.root, &fixture.directory("share"), Some(reply)).await;
    }
}

/// Some servers identify the destination root but return a zero ID for entries
/// below it. Exercise the transition when the job creates its first folder.
async fn check_mixed_destination(local: &Path, share: &Path) {
    let _scope = Scope::children(share, Response::AllZero).unwrap();
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    assert!(provider.prepare_move_destination(share, &cancel).is_ok());
    verified_copy_and_conflict(local, share).await;
    assert!(
        provider
            .prepare_move_destination(&share.join("source"), &cancel)
            .is_err_and(|e| e.kind == std::io::ErrorKind::Unsupported)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_handles_zero_ids_below_an_identified_network_destination() {
    let local = Fixture::new("mixed-network-ids");
    check_mixed_destination(&local.directory("local"), &local.directory("share")).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in: set MIDNIGHT_TEST_SMB_DIRECTORY to a writable SMB share"]
async fn live_smb_verified_copy_with_native_missing_and_zero_ids() {
    let base = std::env::var_os("MIDNIGHT_TEST_SMB_DIRECTORY").unwrap();
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    assert!(
        provider
            .link_destination(Path::new(&base), &cancel)
            .unwrap()
            .network
    );
    for reply in [None, Some(Response::Unsupported), Some(Response::AllZero)] {
        let local = Fixture::new("smb-copy");
        let remote = tempfile::Builder::new()
            .prefix("midnight-smb-copy-")
            .tempdir_in(&base)
            .unwrap();
        check_directions(&local.root, remote.path(), reply).await;
        if let Some(response) = reply {
            let _scope = Scope::new(remote.path(), response).unwrap();
            assert_eq!(
                provider.native_drop_device(remote.path(), &cancel).unwrap(),
                None
            );
        }
        remote.close().unwrap();
    }
    let local = Fixture::new("smb-mixed-ids");
    let remote = tempfile::Builder::new()
        .prefix("midnight-smb-mixed-")
        .tempdir_in(&base)
        .unwrap();
    // A server that supplies the root ID can exercise the mixed-ID case too.
    if provider
        .prepare_move_destination(remote.path(), &cancel)
        .is_ok()
    {
        check_mixed_destination(&local.root, remote.path()).await;
    }
    remote.close().unwrap();
}

/// Verify the share guard across paths with unrelated canonical names and native
/// IDs: a local NTFS folder and the same folder exposed through loopback SMB.
#[test]
#[ignore = "opt-in: also set MIDNIGHT_TEST_SMB_LOCAL_DIRECTORY to the share's local folder"]
fn live_smb_output_guard_excludes_its_local_alias() {
    use std::ffi::OsStr;
    let share = std::env::var_os("MIDNIGHT_TEST_SMB_DIRECTORY").unwrap();
    let local = std::env::var_os("MIDNIGHT_TEST_SMB_LOCAL_DIRECTORY").unwrap();
    let remote = tempfile::Builder::new()
        .prefix("midnight-smb-alias-")
        .tempdir_in(share)
        .unwrap();
    let alias = Path::new(&local).join(remote.path().file_name().unwrap());
    let marker = "alias-check";
    fs::write(remote.path().join(marker), b"same folder").unwrap();
    assert_eq!(fs::read(alias.join(marker)).unwrap(), b"same folder");
    let _scope = Scope::new(remote.path(), Response::AllZero).unwrap();
    let provider = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    for (from, to, name) in [
        (remote.path(), alias.as_path(), "local-output"),
        (alias.as_path(), remote.path(), "share-output"),
    ] {
        let parent = provider.prepare_transfer_destination(to, &cancel).unwrap();
        let source = provider.prepare_transfer_source(from, &cancel).unwrap();
        let output = parent
            .create_copy_directory(
                OsStr::new(name),
                source.requires_copy_directory_guard(),
                &cancel,
            )
            .unwrap();
        let alias_child = provider
            .prepare_transfer_source(&from.join(name), &cancel)
            .and_then(|mut child| child.next_child(&cancel));
        let err = alias_child
            .err()
            .expect("guarded output reopened through its alias");
        assert!(
            err.kind == std::io::ErrorKind::PermissionDenied
                || err.kind == std::io::ErrorKind::ResourceBusy
                || err.message.contains("os error 32"),
            "{err:?}"
        );
        drop(output);
    }
    remote.close().unwrap();
}
