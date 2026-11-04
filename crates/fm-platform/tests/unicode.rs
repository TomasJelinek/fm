use fm_core::{Cancellation, DirectoryService, Entry, EntryKind};
use fm_platform::LocalFileSystem;
use std::{collections::HashSet, ffi::OsString, fs, path::Path};

// These fixtures catch lossy/ANSI conversion of either names or ancestor paths.
const NAMES: [&str; 6] = [
    "Příliš žluťoučký kůň_ěščřžýáíéůú.txt",
    "中文文件_資料.txt",
    "日本語のファイル_ひらがな.txt",
    "العربية_ملف.txt",
    "עברית_קובץ.txt",
    "emoji_🧪🚀😀_👩‍💻.txt",
];

fn list(provider: &LocalFileSystem, path: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    provider
        .read_directory(path, &Cancellation::default(), &mut |entry| {
            entries.push(entry);
            true
        })
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", path.display()));
    entries
}

#[test]
fn multilingual_files_and_directories_keep_exact_names_metadata_and_navigable_paths() {
    let root = tempfile::Builder::new()
        .prefix("FM-Žluťoučký_中文_🧪-")
        .tempdir()
        .unwrap();
    let provider = LocalFileSystem::new(None);
    let mut expected = HashSet::new();
    for name in NAMES {
        fs::write(root.path().join(name), name.as_bytes()).unwrap();
        let directory = format!("adresář_{name}");
        let path = root.path().join(&directory);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("vnořený_中文_🧪.txt"), b"nested payload").unwrap();
        expected.insert(OsString::from(name));
        expected.insert(OsString::from(directory));
    }
    let entries = list(&provider, root.path());
    assert_eq!(entries.len(), 12);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<HashSet<_>>(),
        expected
    );
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.id)
            .collect::<HashSet<_>>()
            .len(),
        12
    );
    for entry in entries {
        assert_eq!(entry.path, root.path().join(&entry.name));
        assert_eq!(entry.display_name, entry.name.to_str().unwrap());
        let metadata = provider.entry_metadata(&entry.path).unwrap();
        assert!(metadata.modified.is_some());
        if entry.kind == EntryKind::Directory {
            assert_eq!(metadata.size, None);
            let children = list(&provider, &entry.path);
            assert_eq!(children.len(), 1);
            assert_eq!(children[0].name, "vnořený_中文_🧪.txt");
            assert_eq!(fs::read(&children[0].path).unwrap(), b"nested payload");
        } else {
            assert_eq!(entry.kind, EntryKind::File);
            let payload = entry.name.to_str().unwrap().as_bytes();
            assert_eq!(metadata.size, Some(payload.len() as u64));
            assert_eq!(fs::read(&entry.path).unwrap(), payload);
        }
    }
}

#[test]
fn composed_and_decomposed_names_remain_distinct_and_open_their_own_contents() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = [
        ("café.txt", b"composed".as_slice()),
        ("cafe\u{301}.txt", b"decomposed".as_slice()),
    ];
    for (name, contents) in fixtures {
        fs::write(root.path().join(name), contents).unwrap();
    }
    let provider = LocalFileSystem::new(None);
    let entries = list(&provider, root.path());
    assert_eq!(
        entries.len(),
        2,
        "normalization must not merge filesystem identities"
    );
    assert_ne!(entries[0].id, entries[1].id);
    for (name, contents) in fixtures {
        let entry = entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(entry.display_name, name);
        assert_eq!(entry.path, root.path().join(name));
        assert_eq!(
            provider.entry_metadata(&entry.path).unwrap().size,
            Some(contents.len() as u64)
        );
        assert_eq!(fs::read(&entry.path).unwrap(), contents);
    }
}
