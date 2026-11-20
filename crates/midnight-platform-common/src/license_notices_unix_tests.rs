use super::*;
use std::os::unix::fs::symlink;

#[test]
fn linked_notices_folders_and_inventories_are_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside");
    write(&outside.join("LICENSE"), "SPOOFED license text");
    write(
        &outside.join("THIRD-PARTY.json"),
        r#"[{"name": "demo", "version": "1.0.0", "authors": ["Mallory"]}]"#,
    );
    let folder = root.path().join("package/licenses/demo-1.0.0");
    write(
        &folder.join("LICENSE-MIT"),
        "Copyright (c) 2020 Ada Lovelace\n",
    );
    symlink(outside.join("LICENSE"), folder.join("LICENSE-LINKED")).unwrap();
    symlink(&outside, folder.join("NOTICES")).unwrap();
    symlink(&outside, root.path().join("package/licenses/linked-1.0.0")).unwrap();
    symlink(
        outside.join("THIRD-PARTY.json"),
        root.path().join("package/THIRD-PARTY.json"),
    )
    .unwrap();
    let sources = package(&root.path().join("package"));
    let index = AuthorIndex::load(sources.inventory.as_deref());
    let notice = read_notice(&sources, &index, &request("demo", "1.0.0", None));
    assert!(notice.text.contains("Ada Lovelace"), "{}", notice.text);
    assert!(!notice.text.contains("SPOOFED"), "{}", notice.text);
    assert_eq!(notice.authors, ["Copyright (c) 2020 Ada Lovelace"]);
    let linked = read_notice(&sources, &index, &request("linked", "1.0.0", None));
    assert!(linked.text.is_empty(), "{}", linked.text);
}

#[test]
fn a_linked_licenses_folder_or_a_fifo_is_never_read() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    write(&real.join("demo-1.0.0/LICENSE"), "SPOOFED");
    fs::create_dir_all(root.path().join("package")).unwrap();
    symlink(&real, root.path().join("package/licenses")).unwrap();
    let sources = package(&root.path().join("package"));
    let notice = read_notice(
        &sources,
        &AuthorIndex::default(),
        &request("demo", "1.0.0", None),
    );
    assert!(notice.text.is_empty(), "{}", notice.text);
    // A FIFO would block a plain open; the type check refuses it first.
    let fifo = root.path().join("fifo");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(read_regular(&fifo, 16).is_err());
}
