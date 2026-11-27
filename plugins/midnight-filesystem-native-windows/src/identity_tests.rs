use super::*;
use std::cell::Cell;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NETNAME_DELETED};

const NO_CLASS: [u32; 5] = [
    ERROR_INVALID_FUNCTION,
    ERROR_NOT_SUPPORTED,
    ERROR_INVALID_PARAMETER,
    ERROR_CALL_NOT_IMPLEMENTED,
    ERROR_INVALID_LEVEL,
];

fn index_information() -> BY_HANDLE_FILE_INFORMATION {
    BY_HANDLE_FILE_INFORMATION {
        dwVolumeSerialNumber: 0x1731_1e74,
        nFileIndexHigh: 0x0102_0304,
        nFileIndexLow: 0x0506_0708,
        ..Default::default()
    }
}

fn widened_index() -> (u64, [u8; 16]) {
    let mut id = [0; 16];
    id[..8].copy_from_slice(&0x0102_0304_0506_0708_u64.to_le_bytes());
    (0x1731_1e74, id)
}

/// NTFS and ReFS: the 128-bit ID and 64-bit serial, without asking for the index.
#[test]
fn a_128_bit_id_is_used_as_reported() {
    let id = FILE_ID_INFO {
        VolumeSerialNumber: 0x0123_4567_89ab_cdef,
        FileId: FILE_ID_128 {
            Identifier: [7; 16],
        },
    };
    let asked = Cell::new(false);
    let identity = choose_identity(Ok(id), || {
        asked.set(true);
        Ok(index_information())
    })
    .unwrap();
    assert_eq!(identity, (0x0123_4567_89ab_cdef, [7; 16]));
    assert!(!asked.get());
}

/// FAT32, exFAT, CD/DVD, WebDAV and SMB 2 answer `FileIdInfo` with one of these errors:
/// the 64-bit index stands in, both halves in place and the serial widened.
#[test]
fn a_file_system_without_128_bit_ids_uses_the_file_index() {
    for code in NO_CLASS {
        let identity = choose_identity(Err(io::Error::from_raw_os_error(code as i32)), || {
            Ok(index_information())
        })
        .unwrap();
        assert_eq!(identity, widened_index(), "{code}");
    }
}

/// A server may answer `FileIdInfo` with an all-zero ID (it has no 128-bit IDs); the
/// host's Find treats that like a missing ID, and so does this plugin.
#[test]
fn an_all_zero_id_uses_the_file_index() {
    let id = FILE_ID_INFO {
        VolumeSerialNumber: 0x0123_4567_89ab_cdef,
        FileId: FILE_ID_128 {
            Identifier: [0; 16],
        },
    };
    assert_eq!(
        choose_identity(Ok(id), || Ok(index_information())).unwrap(),
        widened_index()
    );
}

/// A denial or a lost connection is no missing information class: it stays the error,
/// and the index is never asked for.
#[test]
fn other_failures_stay_errors() {
    for code in [ERROR_ACCESS_DENIED, ERROR_NETNAME_DELETED] {
        let asked = Cell::new(false);
        let error = choose_identity(Err(io::Error::from_raw_os_error(code as i32)), || {
            asked.set(true);
            Ok(index_information())
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(code as i32));
        assert!(!asked.get(), "{code}");
    }
    let error = choose_identity(
        Err(io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32)),
        || Err(io::Error::from_raw_os_error(ERROR_NETNAME_DELETED as i32)),
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(ERROR_NETNAME_DELETED as i32));
    assert!(!unsupported_class(&io::Error::other("no OS code")));
}

/// FAT32 and exFAT implement no `FileAttributeTagInfo`: the basic attributes stand in
/// with tag 0; any other failure stays the error.
#[test]
fn a_file_system_without_reparse_tags_uses_the_basic_attributes() {
    for code in NO_CLASS {
        let info = choose_attribute_tag(Err(io::Error::from_raw_os_error(code as i32)), || {
            Ok(FILE_BASIC_INFO {
                FileAttributes: FILE_ATTRIBUTE_ARCHIVE,
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(info.FileAttributes, FILE_ATTRIBUTE_ARCHIVE, "{code}");
        assert_eq!(info.ReparseTag, 0);
    }
    let Err(error) = choose_attribute_tag(
        Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)),
        || -> io::Result<FILE_BASIC_INFO> { panic!("only a missing class falls back") },
    ) else {
        panic!("a denial stays an error")
    };
    assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
}

/// A fallback entry that claims a reparse point has no known tag and is refused as a
/// link, never read as a regular file or cloud placeholder.
#[test]
fn a_reparse_attribute_without_a_tag_is_refused() {
    let info = choose_attribute_tag(
        Err(io::Error::from_raw_os_error(ERROR_INVALID_PARAMETER as i32)),
        || {
            Ok(FILE_BASIC_INFO {
                FileAttributes: FILE_ATTRIBUTE_REPARSE_POINT,
                ..Default::default()
            })
        },
    )
    .unwrap();
    assert!(crate::system::readable(&info, FILE_TYPE_DISK).is_err());
    let plain = FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: FILE_ATTRIBUTE_ARCHIVE,
        ReparseTag: 0,
    };
    assert!(crate::system::readable(&plain, FILE_TYPE_DISK).is_ok());
}

/// On NTFS the identity is the file's own `FileIdInfo` and the attributes its tag query.
#[test]
fn a_local_ntfs_file_reports_its_128_bit_id() {
    let directory = std::env::temp_dir().join(format!(
        "midnight-native-identity-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("file.txt");
    std::fs::write(&path, b"identity").unwrap();
    let file = File::open(&path).unwrap();
    let id = file_id(&file);
    let identity = file_identity(&file).unwrap();
    if let Ok(id) = id
        && id.FileId.Identifier != [0; 16]
    {
        assert_eq!(identity, (id.VolumeSerialNumber, id.FileId.Identifier));
    }
    assert_eq!(attribute_tag(&file).unwrap().ReparseTag, 0);
    drop(file);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// Reads a file named by `variable` through this plugin's protocol as viewers,
/// thumbnails, comparisons and archives do, then validates the revision twice and
/// checks that a second open reports the same revision. Read only. Without the variable
/// the test is skipped, so a run of every ignored test passes where no such volume is.
fn read_named_file(variable: &str) {
    use crate::test_support::Client;
    use midnight_filesystem_plugin_api::{Request, Response};
    let Some(path) = std::env::var_os(variable).map(std::path::PathBuf::from) else {
        eprintln!("skipped: {variable} is not set");
        return;
    };
    let file = File::open(&path).unwrap();
    println!(
        "{variable}: identity {:02x?}, FileIdInfo {:?}, FileAttributeTagInfo {:?}",
        file_identity(&file).unwrap(),
        file_id(&file).err(),
        tag(&file).err(),
    );
    drop(file);
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    let open = || {
        let Response::Opened {
            reader,
            length,
            revision,
        } = client.call(Request::OpenRead {
            mount,
            node: node.id,
        })
        else {
            panic!("{} must open for reading", path.display())
        };
        (reader, length, revision)
    };
    let (reader, length, revision) = open();
    for _ in 0..2 {
        assert_eq!(
            client.call(Request::ValidateRead { reader }),
            Response::Unit
        );
    }
    let head = client.call(Request::ReadAt {
        reader,
        offset: 0,
        length: length.min(64) as u32,
    });
    assert!(matches!(head, Response::Content { .. }), "{head:?}");
    let (again, _, second) = open();
    assert_eq!(second, revision, "a second open reports the same revision");
    for reader in [reader, again] {
        assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    }
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}

/// A file on a FAT32 or exFAT volume (`MIDNIGHT_TEST_FAT_FILE`): before the index
/// fallback its read failed with ERROR_INVALID_PARAMETER, so no viewer, thumbnail,
/// comparison or archive could open it.
#[test]
#[ignore = "needs MIDNIGHT_TEST_FAT_FILE on a FAT32 or exFAT volume"]
fn a_file_on_a_fat_volume_is_read_and_validated() {
    read_named_file("MIDNIGHT_TEST_FAT_FILE");
}

/// A file on a network share, a mapped drive or a WebDAV folder
/// (`MIDNIGHT_TEST_SHARE_FILE`, a `\\server\share\…` or mapped-drive path).
#[test]
#[ignore = "needs MIDNIGHT_TEST_SHARE_FILE on a network share"]
fn a_file_on_a_network_share_is_read_and_validated() {
    read_named_file("MIDNIGHT_TEST_SHARE_FILE");
}

/// A file on a CD, DVD or mounted ISO image (`MIDNIGHT_TEST_OPTICAL_FILE`).
#[test]
#[ignore = "needs MIDNIGHT_TEST_OPTICAL_FILE on a CD, DVD or mounted ISO image"]
fn a_file_on_an_optical_volume_is_read_and_validated() {
    read_named_file("MIDNIGHT_TEST_OPTICAL_FILE");
}
