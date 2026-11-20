//! Archives inside archives through real restricted workers: ZIP and gzipped TAR inside
//! a ZIP, a text file named `.zip`, the depth limit, the nested-mount allowance and the
//! release of every level. Requires the independently built archive worker.
use midnight_core::{
    ArchiveFormat, ArchiveLocation, ArchiveMount, ArchiveNode, ArchiveParent, Cancellation,
    MAX_ARCHIVE_DEPTH,
};
use midnight_filesystem_host::{Catalog, Mount};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
#[cfg(target_os = "linux")]
#[path = "support/linux.rs"]
mod native;

fn install(root: &Path, worker: &Path) {
    let package = root.join("filesystems").join("archives");
    fs::create_dir_all(&package).unwrap();
    let filename = worker.file_name().unwrap().to_str().unwrap();
    fs::copy(worker, package.join(filename)).unwrap();
    #[cfg(target_os = "linux")]
    native::secure_test_executable(&package.join(filename));
    fs::write(package.join("plugin.toml"), format!(
        "schema_version = 1\nid = \"midnight.fs.archive\"\nkind = \"filesystem-worker\"\nversion = \"0.2.0\"\nentry_executable = \"{filename}\"\ntarget = \"{}\"\nprotocol = 3\nstable_entry_ids = true\nformats = [\"zip\", \"tar\", \"gzip\", \"tar-gzip\", \"7z\"]\n",
        midnight_plugin_host::host_target())).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A ZIP of stored (uncompressed) files.
fn zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let offset = output.len() as u32;
        let header = |signature: u32, central: bool| {
            let mut header = signature.to_le_bytes().to_vec();
            if central {
                header.extend_from_slice(&20_u16.to_le_bytes());
            }
            for field in [20_u16, 0x0800, 0, 0, 0x21] {
                header.extend_from_slice(&field.to_le_bytes());
            }
            header.extend_from_slice(&crc32(data).to_le_bytes());
            header.extend_from_slice(&(data.len() as u32).to_le_bytes());
            header.extend_from_slice(&(data.len() as u32).to_le_bytes());
            header.extend_from_slice(&(name.len() as u16).to_le_bytes());
            header.extend_from_slice(&0_u16.to_le_bytes());
            header
        };
        output.extend(header(0x0403_4B50, false));
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(data);
        let mut entry = header(0x0201_4B50, true);
        entry.extend_from_slice(&[0; 6]);
        entry.extend_from_slice(&(0o100_644_u32 << 16).to_le_bytes());
        entry.extend_from_slice(&offset.to_le_bytes());
        entry.extend_from_slice(name.as_bytes());
        central.extend(entry);
    }
    let start = output.len() as u32;
    let size = central.len() as u32;
    output.extend(central);
    output.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&(members.len() as u16).to_le_bytes());
    output.extend_from_slice(&(members.len() as u16).to_le_bytes());
    output.extend_from_slice(&size.to_le_bytes());
    output.extend_from_slice(&start.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output
}

/// A ustar archive of files.
fn tar(members: &[(&str, &[u8])]) -> Vec<u8> {
    fn octal(field: &mut [u8], value: u64) {
        let text = format!("{value:0width$o}", width = field.len() - 1);
        field[..text.len()].copy_from_slice(text.as_bytes());
    }
    let mut output = Vec::new();
    for (name, data) in members {
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        octal(&mut header[100..108], 0o644);
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], data.len() as u64);
        octal(&mut header[136..148], 1_767_312_000);
        header[148..156].fill(b' ');
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        output.extend_from_slice(&header);
        output.extend_from_slice(data);
        output.resize(output.len().next_multiple_of(512), 0);
    }
    output.resize(output.len() + 1024, 0);
    output
}

/// A gzip member of stored deflate blocks.
fn gzip(data: &[u8]) -> Vec<u8> {
    let mut output = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
    let mut chunks = data.chunks(65_535).peekable();
    while let Some(chunk) = chunks.next() {
        output.push(u8::from(chunks.peek().is_none()));
        let length = chunk.len() as u16;
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(&(!length).to_le_bytes());
        output.extend_from_slice(chunk);
    }
    output.extend_from_slice(&crc32(data).to_le_bytes());
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output
}

fn named(nodes: &[ArchiveNode], name: &str) -> ArchiveNode {
    nodes
        .iter()
        .find(|node| node.display_name == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} is listed"))
}

/// Stage `name` from the root of `mount` and open it as an archive of `format`.
fn open_inside(
    catalog: &Catalog,
    mount: &Arc<Mount>,
    trail: &str,
    name: &str,
    format: ArchiveFormat,
) -> Result<Arc<Mount>, midnight_core::FsError> {
    let cancel = Cancellation::default();
    let member = named(&mount.list(1, &cancel).unwrap(), name);
    let progress = Mutex::new(Vec::new());
    let content = mount.stage_member(member.id.entry, &|| false, &|staged| {
        progress.lock().unwrap().push(staged)
    })?;
    assert_eq!(Some(content.length()), member.size);
    let enclosing: Arc<dyn ArchiveMount> = mount.clone();
    let parent = ArchiveParent {
        location: ArchiveLocation {
            address: mount.address(),
            mount: Some(enclosing),
            directory: 1,
            trail: vec![(1, trail.into())],
        },
        member: member.id,
        name: name.into(),
    };
    let (package, format) = catalog.choose_format(format, None).unwrap();
    Mount::open_nested(package, format, parent, content, &cancel)
}

#[test]
#[ignore = "requires ARCHIVE_WORKER executable built by the independent plugin"]
fn real_workers_open_archives_inside_archives_and_give_every_level_up() {
    let root = tempfile::tempdir_in(
        std::env::var_os("ARCHIVE_TEST_INSTALL_ROOT").expect("private installation root"),
    )
    .unwrap();
    install(
        root.path(),
        Path::new(&std::env::var_os("ARCHIVE_WORKER").unwrap()),
    );
    let catalog = Catalog::discover(root.path()).unwrap();
    assert_eq!(catalog.packages.len(), 1, "{:?}", catalog.diagnostics);
    let deepest = zip(&[("deep.txt", b"Deepest.\n")]);
    let level4 = zip(&[("level5.zip", &deepest)]);
    let level3 = zip(&[("level4.zip", &level4)]);
    let level2 = zip(&[("level3.zip", &level3)]);
    let outer = zip(&[
        ("inner.zip", &zip(&[("inner.txt", b"Innermost.\n")])),
        ("source.tar.gz", &gzip(&tar(&[("notes.md", b"# Notes\n")]))),
        ("fake.zip", b"plain text\n"),
        ("level2.zip", &level2),
    ]);
    let path = root.path().join("outer.zip");
    fs::write(&path, &outer).unwrap();
    let cancel = Cancellation::default();
    let (package, format) = catalog.choose(&path).unwrap();
    let captured = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    let outer = Mount::open_captured(package, format, &path, captured, &cancel).unwrap();

    // A ZIP inside the ZIP: listed, read and reporting its enclosing folder.
    let inner = open_inside(
        &catalog,
        &outer,
        "outer.zip",
        "inner.zip",
        ArchiveFormat::Zip,
    )
    .unwrap();
    let text = named(&inner.list(1, &cancel).unwrap(), "inner.txt");
    assert_eq!(
        inner.read(text.id.entry, 0, 64, &cancel).unwrap(),
        b"Innermost.\n"
    );
    let parent = inner.parent().unwrap();
    assert_eq!(parent.location.mount_id(), Some(outer.id()));
    assert_eq!(parent.name, "inner.zip");
    drop(parent);
    let address = inner.address();
    assert_eq!(address.container, path);
    assert_ne!(address.source_version, outer.address().source_version);
    assert!(
        inner
            .persistent_member_identity(text.id.entry, &address.source_version)
            .unwrap()
            .is_some(),
        "a stable chain keeps persistent preview identities"
    );

    // A gzipped TAR inside the ZIP.
    let source = open_inside(
        &catalog,
        &outer,
        "outer.zip",
        "source.tar.gz",
        ArchiveFormat::TarGzip,
    )
    .unwrap();
    let notes = named(&source.list(1, &cancel).unwrap(), "notes.md");
    assert_eq!(
        source.read(notes.id.entry, 0, 64, &cancel).unwrap(),
        b"# Notes\n"
    );

    // A text file named `.zip` says so, and names its place inside the archive.
    let error = open_inside(
        &catalog,
        &outer,
        "outer.zip",
        "fake.zip",
        ArchiveFormat::Zip,
    )
    .unwrap_err();
    assert!(
        error
            .message
            .starts_with("This file is not a valid ZIP archive."),
        "{error}"
    );
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        error
            .path
            .to_string_lossy()
            .ends_with(&format!("outer.zip{separator}fake.zip")),
        "{error}"
    );

    // Released inner levels give their staged copy and the enclosing archive up.
    inner.release_on_worker().unwrap();
    source.release_on_worker().unwrap();

    // Three levels below the outer archive, and then the depth limit.
    let second = open_inside(
        &catalog,
        &outer,
        "outer.zip",
        "level2.zip",
        ArchiveFormat::Zip,
    )
    .unwrap();
    let third = open_inside(
        &catalog,
        &second,
        "level2.zip",
        "level3.zip",
        ArchiveFormat::Zip,
    )
    .unwrap();
    let fourth = open_inside(
        &catalog,
        &third,
        "level3.zip",
        "level4.zip",
        ArchiveFormat::Zip,
    )
    .unwrap();
    let depth = ArchiveLocation {
        address: fourth.address(),
        mount: Some(fourth.clone()),
        directory: 1,
        trail: vec![(1, "level4.zip".into())],
    }
    .depth();
    assert_eq!(depth, MAX_ARCHIVE_DEPTH);
    let error = open_inside(
        &catalog,
        &fourth,
        "level4.zip",
        "level5.zip",
        ArchiveFormat::Zip,
    )
    .unwrap_err();
    assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
    assert!(error.message.contains("at most 4 levels deep"), "{error}");
    // Three mounts of archives inside archives are open: a fourth waits for one.
    let error = open_inside(
        &catalog,
        &outer,
        "outer.zip",
        "inner.zip",
        ArchiveFormat::Zip,
    )
    .unwrap_err();
    assert!(
        error
            .message
            .contains("archives inside archives are already open"),
        "{error}"
    );
    for mount in [fourth, third, second, outer] {
        mount.release_on_worker().unwrap();
    }
}
