//! Protocol 4: member encryption, unreadability reasons and `Unlock`, with archives
//! written by the pinned `zip` and `sevenz-rust2` writers.
use midnight_filesystem_api::{
    Encryption, ErrorCode, Format, Kind, Node, Password, UnlockStatus, Unreadable,
};
use midnight_fs_archive::{Archive, Limits};
use std::io::{Cursor, Write};
use zip::{AesMode, CompressionMethod, unstable::write::FileOptionsExt, write::SimpleFileOptions};

const PASSWORD: &str = "Kyrandia-heslo-🔑";

#[derive(Clone, Copy)]
enum Lock {
    Plain,
    ZipCrypto(&'static str),
    Aes(AesMode, &'static str),
}

fn zip_archive(members: &[(&str, Vec<u8>, CompressionMethod, Lock)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes, method, lock) in members {
        let options = SimpleFileOptions::default().compression_method(*method);
        match lock {
            Lock::Plain => writer.start_file(*name, options).unwrap(),
            Lock::ZipCrypto(password) => writer
                .start_file(
                    *name,
                    options
                        .with_deprecated_encryption(password.as_bytes())
                        .unwrap(),
                )
                .unwrap(),
            Lock::Aes(mode, password) => writer
                .start_file(*name, options.with_aes_encryption(*mode, password))
                .unwrap(),
        }
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn content(seed: usize, length: usize) -> Vec<u8> {
    (0..length)
        .map(|at| (at * 7 + seed * 13) as u8 % 251)
        .collect()
}

fn listed(archive: &Archive, directory: u64) -> Vec<Node> {
    archive.list(directory, 0).unwrap()
}

fn named<'a>(nodes: &'a [Node], name: &str) -> &'a Node {
    nodes
        .iter()
        .find(|node| node.display == name)
        .unwrap_or_else(|| panic!("{name} listed"))
}

/// The whole member, read in protocol-sized chunks.
fn read_all(archive: &mut Archive, id: u64) -> Result<Vec<u8>, midnight_fs_archive::Error> {
    let mut bytes = Vec::new();
    loop {
        let (chunk, end) = archive.read(id, bytes.len() as u64, 64 * 1024)?;
        bytes.extend(chunk);
        if end {
            return Ok(bytes);
        }
    }
}

fn unlock(archive: &mut Archive, password: &str) -> UnlockStatus {
    archive
        .unlock(Password::new(password.as_bytes().to_vec()))
        .unwrap()
}

/// The user's `share.zip`: a folder of WinZip AES-256 members beside one plain file.
#[test]
fn a_folder_of_winzip_aes_members_lists_sizes_and_reads_after_unlock() {
    let mut members: Vec<_> = (0..12)
        .map(|index| {
            (
                format!("Kyrandia/member-{index:02}.emc"),
                content(index, 1368 + index * 517),
                if index % 2 == 0 {
                    CompressionMethod::Deflated
                } else {
                    CompressionMethod::Stored
                },
                Lock::Aes(AesMode::Aes256, PASSWORD),
            )
        })
        .collect();
    members.push((
        "readme.txt".into(),
        b"plain".to_vec(),
        CompressionMethod::Stored,
        Lock::Plain,
    ));
    let members: Vec<_> = members
        .iter()
        .map(|(name, bytes, method, lock)| (name.as_str(), bytes.clone(), *method, *lock))
        .collect();
    let mut archive = Archive::open(Format::Zip, zip_archive(&members), Limits::default()).unwrap();
    let root = listed(&archive, 1);
    let folder = named(&root, "Kyrandia");
    assert_eq!(folder.kind, Kind::Directory);
    assert!(
        folder.contains_encrypted,
        "the folder holds encrypted members"
    );
    assert_eq!(folder.encryption, Encryption::None);
    let readme = named(&root, "readme.txt").clone();
    assert!(!readme.contains_encrypted && readme.encryption == Encryption::None);
    let files = listed(&archive, folder.id);
    assert_eq!(files.len(), 12);
    for (node, (_, bytes, ..)) in files.iter().zip(&members) {
        assert_eq!(node.encryption, Encryption::Aes256, "{}", node.display);
        assert!(node.readable && node.unreadable.is_none());
        assert_eq!(node.size, Some(bytes.len() as u64), "{}", node.display);
    }
    // Locked: nothing is staged, and the session goes on.
    let error = read_all(&mut archive, files[0].id).unwrap_err();
    assert_eq!(error.code, ErrorCode::PasswordRequired, "{error}");
    assert_eq!(read_all(&mut archive, readme.id).unwrap(), b"plain");
    assert_eq!(unlock(&mut archive, "wrong"), UnlockStatus::WrongPassword);
    assert!(!archive.unlocked());
    assert_eq!(
        read_all(&mut archive, files[1].id).unwrap_err().code,
        ErrorCode::PasswordRequired
    );
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
    assert!(archive.unlocked());
    for (node, (_, bytes, ..)) in files.iter().zip(&members) {
        assert_eq!(
            &read_all(&mut archive, node.id).unwrap(),
            bytes,
            "{}",
            node.display
        );
    }
}

#[test]
fn zipcrypto_and_every_aes_key_length_open_with_one_password_in_a_mixed_archive() {
    let members = [
        (
            "legacy.bin",
            content(1, 3000),
            CompressionMethod::Deflated,
            Lock::ZipCrypto(PASSWORD),
        ),
        (
            "stored.bin",
            content(2, 77),
            CompressionMethod::Stored,
            Lock::ZipCrypto(PASSWORD),
        ),
        (
            "aes128.bin",
            content(3, 4096),
            CompressionMethod::Deflated,
            Lock::Aes(AesMode::Aes128, PASSWORD),
        ),
        (
            "aes192.bin",
            content(4, 10),
            CompressionMethod::Stored,
            Lock::Aes(AesMode::Aes192, PASSWORD),
        ),
        (
            "plain.txt",
            b"plain text".to_vec(),
            CompressionMethod::Deflated,
            Lock::Plain,
        ),
    ];
    let mut archive = Archive::open(Format::Zip, zip_archive(&members), Limits::default()).unwrap();
    let nodes = listed(&archive, 1);
    let expected = [
        Encryption::ZipCrypto,
        Encryption::ZipCrypto,
        Encryption::Aes128,
        Encryption::Aes192,
        Encryption::None,
    ];
    for ((name, bytes, ..), encryption) in members.iter().zip(expected) {
        let node = named(&nodes, name);
        assert_eq!(node.encryption, encryption, "{name}");
        assert_eq!(node.size, Some(bytes.len() as u64), "{name}");
        assert!(node.readable, "{name}");
    }
    assert_eq!(
        unlock(&mut archive, "Kyrandia"),
        UnlockStatus::WrongPassword
    );
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
    for (name, bytes, ..) in &members {
        let id = named(&nodes, name).id;
        assert_eq!(&read_all(&mut archive, id).unwrap(), bytes, "{name}");
    }
}

/// Byte offset of the first content byte of the member called `name` (after its
/// local header; for an encrypted member, its encryption header comes first).
fn data_start(archive: &[u8], name: &str) -> usize {
    let header = archive
        .windows(4)
        .enumerate()
        .filter(|(_, bytes)| *bytes == b"PK\x03\x04")
        .map(|(at, _)| at)
        .find(|at| archive[at + 30..].starts_with(name.as_bytes()))
        .unwrap();
    let field = |at: usize| usize::from(u16::from_le_bytes([archive[at], archive[at + 1]]));
    header + 30 + field(header + 26) + field(header + 28)
}

#[test]
fn tampered_ciphertext_fails_authentication_instead_of_reading_garbage() {
    let data = content(5, 2000);
    for (lock, header) in [
        (Lock::Aes(AesMode::Aes256, PASSWORD), 16 + 2),
        (Lock::ZipCrypto(PASSWORD), 12),
    ] {
        let mut source =
            zip_archive(&[("secret.bin", data.clone(), CompressionMethod::Stored, lock)]);
        let at = data_start(&source, "secret.bin") + header + 100;
        source[at] ^= 0x20;
        let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
        let id = listed(&archive, 1)[0].id;
        assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
        let error = read_all(&mut archive, id).unwrap_err();
        assert_eq!(error.code, ErrorCode::Corrupt, "{error}");
    }
}

#[test]
fn a_member_with_another_password_reports_wrong_password_and_keeps_the_mount() {
    let mut members: Vec<_> = (0..4)
        .map(|index| {
            (
                format!("first-{index}.bin"),
                content(index, 300),
                CompressionMethod::Stored,
                Lock::Aes(AesMode::Aes256, PASSWORD),
            )
        })
        .collect();
    members.push((
        "other.bin".into(),
        content(9, 300),
        CompressionMethod::Stored,
        Lock::Aes(AesMode::Aes256, "another password"),
    ));
    let members: Vec<_> = members
        .iter()
        .map(|(name, bytes, method, lock)| (name.as_str(), bytes.clone(), *method, *lock))
        .collect();
    let mut archive = Archive::open(Format::Zip, zip_archive(&members), Limits::default()).unwrap();
    let nodes = listed(&archive, 1);
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
    let other = named(&nodes, "other.bin").id;
    assert_eq!(
        read_all(&mut archive, other).unwrap_err().code,
        ErrorCode::WrongPassword
    );
    let first = named(&nodes, "first-0.bin").id;
    assert_eq!(read_all(&mut archive, first).unwrap(), members[0].1);
}

/// Patch the compression method and general-purpose flags of every record of `name`.
fn patch_records(archive: &mut [u8], name: &str, method: Option<u16>, flags: u16) {
    let mut at = 0;
    while let Some(found) = archive[at..]
        .windows(name.len())
        .position(|bytes| bytes == name.as_bytes())
    {
        let start = at + found;
        at = start + 1;
        let (flags_at, method_at) =
            if start >= 30 && archive[start - 30..start - 26] == *b"PK\x03\x04" {
                (start - 30 + 6, start - 30 + 8)
            } else if start >= 46 && archive[start - 46..start - 42] == *b"PK\x01\x02" {
                (start - 46 + 8, start - 46 + 10)
            } else {
                continue;
            };
        let old = u16::from_le_bytes([archive[flags_at], archive[flags_at + 1]]);
        archive[flags_at..flags_at + 2].copy_from_slice(&(old | flags).to_le_bytes());
        if let Some(method) = method {
            archive[method_at..method_at + 2].copy_from_slice(&method.to_le_bytes());
        }
    }
}

#[test]
fn unreadable_members_name_their_reason_and_keep_their_size() {
    let mut source = zip_archive(&[
        (
            "imploded.bin",
            content(1, 40),
            CompressionMethod::Stored,
            Lock::Plain,
        ),
        (
            "ppmd.bin",
            content(2, 41),
            CompressionMethod::Stored,
            Lock::Plain,
        ),
        (
            "unknown.bin",
            content(3, 42),
            CompressionMethod::Stored,
            Lock::Plain,
        ),
        (
            "strong.bin",
            content(4, 43),
            CompressionMethod::Stored,
            Lock::Plain,
        ),
    ]);
    patch_records(&mut source, "imploded.bin", Some(6), 0);
    patch_records(&mut source, "ppmd.bin", Some(98), 0);
    patch_records(&mut source, "unknown.bin", Some(77), 0);
    // PKWARE strong encryption: bit 6 beside bit 0.
    patch_records(&mut source, "strong.bin", None, 0x41);
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let nodes = listed(&archive, 1);
    for (name, size, reason) in [
        (
            "imploded.bin",
            40,
            Unreadable::Compression("Implode".into()),
        ),
        ("ppmd.bin", 41, Unreadable::Compression("PPMd".into())),
        (
            "unknown.bin",
            42,
            Unreadable::Compression("method 77".into()),
        ),
        ("strong.bin", 43, Unreadable::Encryption),
    ] {
        let node = named(&nodes, name);
        assert!(!node.readable, "{name}");
        assert_eq!(node.unreadable.as_ref(), Some(&reason), "{name}");
        assert_eq!(node.size, Some(size), "{name}");
        let error = read_all(&mut archive, node.id).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported, "{name}");
        assert!(
            !error.message.contains("Unsafe, encrypted"),
            "{}",
            error.message
        );
    }
    assert_eq!(named(&nodes, "strong.bin").encryption, Encryption::Unknown);
    let error = read_all(&mut archive, named(&nodes, "imploded.bin").id).unwrap_err();
    assert!(error.message.contains("Implode"), "{}", error.message);
    // Its only encrypted member uses a method this worker cannot decrypt.
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unsupported);
}

#[test]
fn plain_archives_are_not_encrypted_and_links_and_unsafe_names_say_why() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("../escape.txt", options).unwrap();
    writer.write_all(b"unsafe").unwrap();
    writer.add_symlink("link", "target", options).unwrap();
    let source = writer.finish().unwrap().into_inner();
    let mut archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let nodes = listed(&archive, 1);
    let escape = named(&nodes, "../escape.txt");
    assert_eq!(escape.unreadable, Some(Unreadable::UnsafeName));
    assert_eq!(escape.size, Some(6));
    assert_eq!(named(&nodes, "link").unreadable, Some(Unreadable::Link));
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::NotEncrypted);
}

#[test]
fn tar_links_special_records_and_unsafe_names_say_why() {
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    tar.append_link(&mut header, "link", "target").unwrap();
    let mut fifo = tar::Header::new_gnu();
    fifo.set_entry_type(tar::EntryType::Fifo);
    fifo.set_size(0);
    fifo.set_cksum();
    tar.append_data(&mut fifo, "pipe", std::io::empty())
        .unwrap();
    // The writer refuses `..`; the name bytes go into the header directly.
    let mut file = tar::Header::new_gnu();
    file.set_size(5);
    let name = b"a/../../escape";
    file.as_old_mut().name[..name.len()].copy_from_slice(name);
    file.set_cksum();
    tar.append(&file, &b"bytes"[..]).unwrap();
    let source = tar.into_inner().unwrap();
    let mut archive = Archive::open(Format::Tar, source, Limits::default()).unwrap();
    let nodes = listed(&archive, 1);
    assert_eq!(named(&nodes, "link").unreadable, Some(Unreadable::Link));
    assert_eq!(
        named(&nodes, "pipe").unreadable,
        Some(Unreadable::SpecialFile)
    );
    let escape = named(&nodes, "a/../../escape");
    assert_eq!(escape.unreadable, Some(Unreadable::UnsafeName));
    assert_eq!(escape.size, Some(5), "the length its header declares");
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::NotEncrypted);
}

fn sevenzip(entries: &[(&str, Vec<u8>)], encrypt_names: bool) -> Vec<u8> {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod,
        encoder_options::AesEncoderOptions,
    };
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![
        AesEncoderOptions::new(PASSWORD.into()).into(),
        EncoderConfiguration::new(EncoderMethod::LZMA2),
    ]);
    writer.set_encrypt_header(encrypt_names);
    // One block per member: the unlock check decodes the first byte of each.
    for (name, bytes) in entries {
        writer
            .push_archive_entry(
                ArchiveEntry::new_file(name),
                Some(Cursor::new(bytes.clone())),
            )
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn sevenzip_aes_blocks_list_without_and_read_with_the_password() {
    let entries: Vec<_> = (0..4)
        .map(|index| {
            (
                format!("folder/part-{index}.bin"),
                content(index, 5000 + index),
            )
        })
        .collect();
    let entries: Vec<_> = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.clone()))
        .collect();
    let mut archive = Archive::open(
        Format::SevenZip,
        sevenzip(&entries, false),
        Limits::default(),
    )
    .unwrap();
    let folder = listed(&archive, 1).remove(0);
    assert!(folder.contains_encrypted);
    let nodes = listed(&archive, folder.id);
    for (node, (_, bytes)) in nodes.iter().zip(&entries) {
        assert_eq!(node.encryption, Encryption::SevenZipAes256);
        assert!(node.readable);
        assert_eq!(node.size, Some(bytes.len() as u64));
    }
    assert_eq!(
        read_all(&mut archive, nodes[0].id).unwrap_err().code,
        ErrorCode::PasswordRequired
    );
    assert_eq!(unlock(&mut archive, "wrong"), UnlockStatus::WrongPassword);
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
    for (node, (_, bytes)) in nodes.iter().zip(&entries) {
        assert_eq!(&read_all(&mut archive, node.id).unwrap(), bytes);
    }
}

#[test]
fn sevenzip_encrypted_names_list_only_after_unlock() {
    let entries = [("hidden/name.txt", b"secret names".to_vec())];
    let mut archive = Archive::open(
        Format::SevenZip,
        sevenzip(&entries, true),
        Limits::default(),
    )
    .unwrap();
    let error = archive.list(1, 0).unwrap_err();
    assert_eq!(error.code, ErrorCode::PasswordRequired, "{error}");
    assert_eq!(unlock(&mut archive, "wrong"), UnlockStatus::WrongPassword);
    assert_eq!(
        archive.list(1, 0).unwrap_err().code,
        ErrorCode::PasswordRequired
    );
    assert_eq!(unlock(&mut archive, PASSWORD), UnlockStatus::Unlocked);
    let folder = listed(&archive, 1).remove(0);
    assert_eq!(folder.display, "hidden");
    let file = listed(&archive, folder.id).remove(0);
    assert_eq!(read_all(&mut archive, file.id).unwrap(), b"secret names");
}

/// The user's own archive, read-only and never unlocked: every member is listed with
/// the size its directory records and its encryption.
#[test]
#[ignore = "reads the archive named by MIDNIGHT_ARCHIVE_SHARE_ZIP"]
fn a_real_winzip_aes_archive_lists_every_member_with_its_size_and_encryption() {
    let path = std::env::var_os("MIDNIGHT_ARCHIVE_SHARE_ZIP").expect("archive path");
    let source = std::fs::read(path).unwrap();
    let archive = Archive::open(Format::Zip, source, Limits::default()).unwrap();
    let (mut files, mut sized, mut aes, mut readable) = (0, 0, 0, 0);
    let mut folders = vec![1];
    while let Some(folder) = folders.pop() {
        let mut after = 0;
        loop {
            let (nodes, more) = archive.page(folder, after).unwrap();
            for node in &nodes {
                after = node.id;
                if node.kind == Kind::Directory {
                    folders.push(node.id);
                    continue;
                }
                files += 1;
                sized += usize::from(node.size.is_some());
                aes += usize::from(node.encryption == Encryption::Aes256);
                readable += usize::from(node.readable);
            }
            if !more {
                break;
            }
        }
    }
    eprintln!("members={files} sized={sized} aes256={aes} readable={readable}");
    assert_eq!(sized, files, "every member keeps its size");
    assert!(aes > 0);
    assert_eq!(
        readable, files,
        "AES members open once the archive is unlocked"
    );
}
