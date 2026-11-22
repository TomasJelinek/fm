use super::super::{SpoolContent, SpoolRoot, create_private_spool, spool_roots};
use super::*;
use midnight_worker_platform::spool::PrivateSpoolDirectory;

fn kept(path: impl Into<PathBuf>) -> SpoolRoot {
    SpoolRoot {
        path: path.into(),
        per_process: false,
    }
}

fn per_process(path: impl Into<PathBuf>) -> SpoolRoot {
    SpoolRoot {
        path: path.into(),
        per_process: true,
    }
}

#[test]
fn spool_roots_use_the_temporary_directory_per_user_then_per_process() {
    let temporary = std::env::temp_dir();
    let session = from_environment(Some("alice/../x".into()), temporary.clone());
    let user = "MidnightExplorer-616c6963652f2e2e2f78";
    // Without XDG directories, large content and reports share the order.
    for (name, content) in [
        ("file-list", SpoolContent::Large),
        ("reports", SpoolContent::Reports),
    ] {
        assert_eq!(
            spool_roots(&session, 42, name, content),
            [
                kept(temporary.join(format!("{user}-{name}"))),
                per_process(temporary.join(format!("MidnightExplorer-{name}-42"))),
            ]
        );
    }
    // An empty or missing account name keeps only the per-process root.
    for user in [Some(OsString::new()), None] {
        assert_eq!(
            spool_roots(
                &from_environment(user, temporary.clone()),
                7,
                "reports",
                SpoolContent::Reports
            ),
            [per_process(temporary.join("MidnightExplorer-reports-7"))]
        );
    }
}

#[test]
fn account_labels_keep_non_ascii_names_distinct() {
    assert_eq!(
        label(OsStr::new("Пользователь")),
        "d09fd0bed0bbd18cd0b7d0bed0b2d0b0d182d0b5d0bbd18c"
    );
    assert_eq!(label(OsStr::new("山田")), "e5b1b1e794b0");
    // Names an ASCII filter would have merged, or dropped entirely, stay apart.
    let names = [
        "Пользователь",
        "Пользователи",
        "山田",
        "山本",
        "jöe",
        "je",
        "Bob",
        "bob",
    ];
    let labels = names
        .iter()
        .map(|name| label(OsStr::new(name)))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(labels.len(), names.len());
}

#[test]
fn linked_foreign_and_shared_roots_are_refused_for_the_last_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    // This user's own private directory, reached through a junction that
    // another account could have planted under the expected name.
    let target = temporary.path().join("target");
    drop(PrivateSpoolDirectory::open(&target).unwrap());
    let linked = temporary.path().join("linked");
    crate::native_test_links::create(&linked, &target);
    assert!(linked.is_dir(), "the junction leads to its target");
    // Owned by TrustedInstaller, with access for other accounts.
    let foreign =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    // An own directory that inherits the parent's ACL, not a private one.
    let shared = temporary.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    let last = temporary.path().join("MidnightExplorer-reports-7");
    let roots = [
        kept(&linked),
        kept(&foreign),
        kept(&shared),
        per_process(&last),
    ];
    let mut spool = create_private_spool(&roots, PrivateSpoolDirectory::create).unwrap();
    assert_eq!(spool.append(0, "東京 list".as_bytes()).unwrap(), 11);
    spool.finish().unwrap();
    let mut bytes = [0; 11];
    assert_eq!(spool.read_at(0, &mut bytes).unwrap(), 11);
    assert_eq!(&bytes, "東京 list".as_bytes());
    // A delete-on-close spool keeps its name while open: only the last
    // candidate holds one, so every earlier candidate was refused.
    assert_eq!(std::fs::read_dir(&last).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(&shared).unwrap().count(), 0);
    drop(spool);
    assert!(
        !last.exists(),
        "the last spool removes its per-process root"
    );
    let error = create_private_spool(&roots[..3], PrivateSpoolDirectory::create)
        .err()
        .expect("no private root among the refused candidates");
    assert!(error.to_string().contains("shared"), "{error}");
}
