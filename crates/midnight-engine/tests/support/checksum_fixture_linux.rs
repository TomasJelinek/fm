pub fn fixture() -> tempfile::TempDir {
    let root = std::env::temp_dir().join("midnight-checksums/_build/tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
