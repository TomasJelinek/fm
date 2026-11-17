pub fn fixture() -> tempfile::TempDir {
    // A unique directory directly in this process's temporary directory: a
    // fixed shared parent in `/tmp` would be owned by whichever account or CI
    // job created it first and refuse everyone else's fixtures.
    tempfile::Builder::new()
        .prefix("midnight-checksums-")
        .tempdir()
        .unwrap()
}
