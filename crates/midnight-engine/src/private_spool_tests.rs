use super::*;
use std::path::Path;

fn kept(path: PathBuf) -> SpoolRoot {
    SpoolRoot {
        path,
        per_process: false,
    }
}

fn per_process(path: PathBuf) -> SpoolRoot {
    SpoolRoot {
        path,
        per_process: true,
    }
}

fn session(base: &Path, account: Option<&str>) -> Session {
    Session {
        application: "App",
        cache: Some(base.join("cache")),
        runtime: Some(base.join("runtime")),
        account: account.map(str::to_owned),
        temporary: base.join("temporary"),
    }
}

#[test]
fn spool_roots_share_one_order_with_a_per_process_temporary_root_last() {
    let base = std::env::temp_dir();
    let temporary = base.join("temporary");
    let desktop = session(&base, Some("1000"));
    assert_eq!(
        spool_roots(&desktop, 42, "file-list", SpoolContent::Large),
        [
            kept(base.join("cache").join("App").join("file-list")),
            kept(base.join("runtime").join("App").join("file-list")),
            kept(temporary.join("App-1000-file-list")),
            per_process(temporary.join("App-file-list-42")),
        ]
    );
    // Reports differ only in skipping the disk-backed cache.
    assert_eq!(
        spool_roots(&desktop, 42, "reports", SpoolContent::Reports),
        [
            kept(base.join("runtime").join("App").join("reports")),
            kept(temporary.join("App-1000-reports")),
            per_process(temporary.join("App-reports-42")),
        ]
    );
    // Relative bases are ignored; without an account label only the
    // per-process temporary root remains.
    let relative = Session {
        cache: Some("cache".into()),
        runtime: Some("runtime".into()),
        ..session(&base, None)
    };
    assert_eq!(
        spool_roots(&relative, 7, "file-list", SpoolContent::Large),
        [per_process(temporary.join("App-file-list-7"))]
    );
    let relative = Session {
        temporary: "temporary".into(),
        ..relative
    };
    assert!(spool_roots(&relative, 7, "reports", SpoolContent::Reports).is_empty());
    // A label too long for one path component keeps only the per-process root.
    let long = "6c".repeat(MAX_COMPONENT / 2);
    assert_eq!(
        spool_roots(
            &session(&base, Some(&long)),
            7,
            "reports",
            SpoolContent::Reports
        ),
        [
            kept(base.join("runtime").join("App").join("reports")),
            per_process(temporary.join("App-reports-7")),
        ]
    );
}

#[test]
fn the_current_session_ends_with_its_per_process_temporary_root() {
    for content in [SpoolContent::Reports, SpoolContent::Large] {
        let roots = private_spool_roots("reports", content);
        assert!(roots.iter().all(|root| root.path.is_absolute()));
        let (last, earlier) = roots.split_last().unwrap();
        assert!(last.per_process);
        // The temporary directory, whose links Linux resolves first.
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap();
        assert_eq!(
            canonical(last.path.parent().unwrap()),
            canonical(&std::env::temp_dir())
        );
        assert!(
            last.path
                .to_string_lossy()
                .ends_with(&format!("-reports-{}", std::process::id()))
        );
        assert!(earlier.iter().all(|root| !root.per_process));
    }
}

#[test]
fn without_a_usable_candidate_the_last_refusal_is_reported() {
    let Err(error) = create_private_spool(&[], PrivateSpoolDirectory::create) else {
        panic!("no candidate root");
    };
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    let temporary = tempfile::tempdir().unwrap();
    let occupied = temporary.path().join("occupied 資料");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let Err(error) = create_private_spool(&[kept(occupied.clone())], PrivateSpoolDirectory::create)
    else {
        panic!("an occupied name is not a private spool root");
    };
    assert!(error.to_string().contains("occupied 資料"), "{error}");
    assert!(occupied.is_file());
}

#[test]
fn a_per_process_root_is_removed_after_its_last_spool_closes() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("MidnightExplorer-reports-7");
    let roots = [per_process(path.clone())];
    let first = create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    let mut second =
        create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    assert!(path.is_dir());
    drop(first);
    // The other spool still uses the root: it stays and accepts new spools.
    assert!(path.is_dir());
    second.append(0, b"kept").unwrap();
    let third = create_private_spool(&roots, PrivateSpoolDirectory::create_append_only).unwrap();
    drop((second, third));
    assert!(
        !path.exists(),
        "the last spool removes its per-process root"
    );
    // A shared or per-user root is kept for later runs.
    let kept_root = temporary.path().join("per-user");
    drop(
        create_private_spool(
            &[kept(kept_root.clone())],
            PrivateSpoolDirectory::create_append_only,
        )
        .unwrap(),
    );
    assert!(kept_root.is_dir());
}
