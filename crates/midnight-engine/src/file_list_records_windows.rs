use midnight_core::EntryMetadata;
use std::path::{Component, Path};

pub(super) fn parts(directory: &str) -> (String, String) {
    let root = match Path::new(directory).components().next() {
        Some(Component::Prefix(prefix)) => prefix.as_os_str().to_string_lossy().into_owned(),
        _ => String::new(),
    };
    let mut path = directory.get(root.len()..).unwrap_or(directory).to_owned();
    if !path.ends_with(['\\', '/']) {
        path.push('\\');
    }
    (root, path)
}

pub(super) fn attributes(metadata: &EntryMetadata) -> String {
    let Some(bits) = metadata.native_attributes else {
        return "-".into();
    };
    let result: String = [
        (0x1, 'R'),
        (0x2, 'H'),
        (0x4, 'S'),
        (0x10, 'D'),
        (0x20, 'A'),
        (0x100, 'T'),
        (0x200, 'P'),
        (0x400, 'L'),
        (0x800, 'C'),
        (0x1000, 'O'),
        (0x2000, 'I'),
        (0x4000, 'E'),
    ]
    .into_iter()
    .filter(|(bit, _)| bits & bit != 0)
    .map(|(_, letter)| letter)
    .collect();
    if result.is_empty() {
        "-".into()
    } else {
        result
    }
}
