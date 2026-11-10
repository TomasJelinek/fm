//! Raw Unix names that have identical lossy display text must remain distinct.
use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn same_name_preserves_non_utf8_native_identity() {
    let name = OsString::from_vec(b"raw\x80.txt".to_vec());
    let decoy = OsString::from_vec(b"raw\x81.txt".to_vec());
    assert_eq!(name.to_string_lossy(), decoy.to_string_lossy());
    let entries: Arc<[Entry]> = vec![
        Entry::new(
            EntryId(0),
            PathBuf::from("right").join(decoy),
            EntryKind::File,
        ),
        Entry::new(
            EntryId(1),
            PathBuf::from("right").join(&name),
            EntryKind::File,
        ),
    ]
    .into();
    let source = FileCompareSource::SameName {
        directory: "right".into(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let launch = FileCompareLaunch {
        sources: vec![
            FileCompareSource::Focused(PathBuf::from("left").join(&name)),
            source,
        ],
        disabled_plugins: Arc::from([]),
    };
    assert_eq!(
        resolve(&launch, &AtomicBool::new(false)).unwrap()[1],
        PathBuf::from("right").join(name)
    );
}
