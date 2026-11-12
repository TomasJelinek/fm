use midnight_core::EntryMetadata;

pub(super) fn parts(directory: &str) -> (String, String) {
    let mut path = directory.to_owned();
    if !path.ends_with('/') {
        path.push('/');
    }
    (String::new(), path)
}

pub(super) fn attributes(metadata: &EntryMetadata) -> String {
    metadata
        .unix_mode
        .map_or_else(|| "-".into(), |mode| format!("{:04o}", mode & 0o7777))
}
