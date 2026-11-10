use super::*;

fn location(name: &str) -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("_build/virtual-checksum-formats")
        .join(name)
}
fn parse(input: &[u8], name: &str) -> Result<ParsedManifest, FsError> {
    parse_manifest(
        input,
        &location(name),
        ChecksumLimits::default(),
        &Cancellation::default(),
    )
}

#[test]
fn salamander_records_detect_all_five_algorithms_and_native_subdirectories() {
    for (input, algorithm, digest) in [
        (
            "folder\\č文 abc.txt  352441C2\r\n".to_owned(),
            ChecksumAlgorithm::Crc32,
            vec![0x35, 0x24, 0x41, 0xc2],
        ),
        (
            format!("{} *folder/č文 abc.txt\r\n", "ab".repeat(16)),
            ChecksumAlgorithm::Md5,
            vec![0xab; 16],
        ),
        (
            format!("{}  folder/č文 abc.txt\r\n", "AB".repeat(20)),
            ChecksumAlgorithm::Sha1,
            vec![0xab; 20],
        ),
        (
            format!("{} *folder/č文 abc.txt\n", "ab".repeat(32)),
            ChecksumAlgorithm::Sha256,
            vec![0xab; 32],
        ),
        (
            format!("{} *folder/č文 abc.txt", "ab".repeat(64)),
            ChecksumAlgorithm::Sha512,
            vec![0xab; 64],
        ),
    ] {
        let parsed = parse(input.as_bytes(), "manifest.txt").unwrap();
        assert_eq!(parsed.algorithm, algorithm);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, location("folder/č文 abc.txt"));
        assert_eq!(parsed.entries[0].digest, digest);
    }
}

#[test]
fn bsd_records_preserve_parentheses_and_comments_are_ignored() {
    for (label, count, algorithm) in [
        ("MD5", 16, ChecksumAlgorithm::Md5),
        ("SHA1", 20, ChecksumAlgorithm::Sha1),
        ("SHA256", 32, ChecksumAlgorithm::Sha256),
        ("SHA512", 64, ChecksumAlgorithm::Sha512),
    ] {
        let input = format!(
            "\u{feff}; exported\r\n # comment\n\n{label}(file () 文.txt)= {}\n",
            "fa".repeat(count)
        );
        let parsed = parse(input.as_bytes(), "manifest.txt").unwrap();
        assert_eq!(parsed.algorithm, algorithm);
        assert_eq!(parsed.entries[0].path, location("file () 文.txt"));
        assert_eq!(parsed.entries[0].digest, vec![0xfa; count]);
    }
}

#[test]
fn gnu_text_binary_and_escaped_newlines_preserve_filename_whitespace() {
    for (record, expected) in [
        (" * leading trailing ", " leading trailing "),
        ("   leading trailing ", " leading trailing "),
        ("\tfile", "file"),
    ] {
        let input = format!("{}{record}\n", "12".repeat(16));
        assert_eq!(
            parse(input.as_bytes(), "x.md5").unwrap().entries[0].path,
            location(expected)
        );
    }
    let input = format!("\\{} *line\\ncarriage\\r文\n", "12".repeat(32));
    let result = parse(input.as_bytes(), "x.sha256").unwrap();
    assert_eq!(result.entries[0].path, location("line\ncarriage\r文"));
}

#[test]
fn hash_only_records_require_matching_sidecar_extension_and_one_entry() {
    for (extension, length, algorithm) in [
        ("sfv", 4, ChecksumAlgorithm::Crc32),
        ("md5", 16, ChecksumAlgorithm::Md5),
        ("sha1", 20, ChecksumAlgorithm::Sha1),
        ("sha256", 32, ChecksumAlgorithm::Sha256),
        ("sha512", 64, ChecksumAlgorithm::Sha512),
    ] {
        let text = format!("{}\r\n", "01".repeat(length));
        let parsed = parse(text.as_bytes(), &format!("č文.bin.{extension}")).unwrap();
        assert_eq!(parsed.algorithm, algorithm);
        assert_eq!(parsed.entries[0].path, location("č文.bin"));
        assert!(parse(text.as_bytes(), "file.unknown").is_err());
        assert!(
            parse(
                format!("{text}{text}").as_bytes(),
                &format!("file.{extension}")
            )
            .is_err()
        );
    }
    assert!(parse("01".repeat(16).as_bytes(), "file.sha256").is_err());
}

#[test]
fn unicode_boms_decode_without_replacement_and_malformed_encodings_fail() {
    let input = "文.txt  352441c2\r\n";
    for little in [true, false] {
        let mut bytes = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in input.encode_utf16() {
            bytes.extend(if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        assert_eq!(
            parse(&bytes, "files.sfv").unwrap().entries[0].path,
            location("文.txt")
        );
    }
    for malformed in [
        &[0xff, 0xfe, 0x00][..],
        &[0xff, 0xfe, 0x00, 0xd8],
        &[0xff],
        b"file\0.txt  352441c2\n",
    ] {
        assert!(parse(malformed, "files.sfv").is_err());
    }
}

#[test]
fn untrusted_names_cannot_escape_manifest_directory_or_use_windows_aliases() {
    for name in [
        "../outside",
        "nested/../../outside",
        "..\\outside",
        "/absolute",
        "\\rooted",
        "C:/drive",
        "C:relative",
        "\\\\server\\share",
        "\\\\?\\C:\\file",
        "file:stream",
        "./file",
        "nested//file",
        "nested/./file",
        "nested\\..\\file",
        "NUL",
        "aux.txt",
        "COM1",
        "LPT9.log",
        "folder/NUL.txt",
        "file.",
    ] {
        let input = format!("{} *{name}\n", "01".repeat(16));
        assert!(parse(input.as_bytes(), "files.md5").is_err(), "{name}");
    }
}

#[test]
fn duplicates_mixed_algorithms_ambiguous_lines_and_empty_input_fail_atomically() {
    let md5 = "01".repeat(16);
    for input in [
        "".to_owned(),
        "; comment\n# comment\n\n".to_owned(),
        format!("{md5} *file\n{md5} *file\n"),
        format!("{md5} *file\n{md5} *FILE\n"),
        format!("{md5} *a/file\n{md5} *a\\file\n"),
        format!("{md5} *file\n{} *other\n", "01".repeat(20)),
        format!("{md5} *\n"),
        "file  1234567z\n".to_owned(),
        format!("\\{md5} *bad\\qescape\n"),
        format!("MD5 (file) = {}\n", "01".repeat(20)),
    ] {
        assert!(parse(input.as_bytes(), "files.md5").is_err(), "{input:?}");
    }
}

#[test]
fn row_depth_node_memory_budgets_and_cancellation_reject_whole_manifest() {
    let input = b"folder/first  352441c2\nfolder/second  352441c2\n";
    for limits in [
        ChecksumLimits {
            max_rows: 1,
            ..ChecksumLimits::default()
        },
        ChecksumLimits {
            max_nodes: 1,
            ..ChecksumLimits::default()
        },
        ChecksumLimits {
            max_depth: 1,
            ..ChecksumLimits::default()
        },
        ChecksumLimits {
            max_retained_bytes: 1,
            ..ChecksumLimits::default()
        },
    ] {
        assert!(
            parse_manifest(input, &location("x.sfv"), limits, &Cancellation::default()).is_err()
        );
    }
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        parse_manifest(
            input,
            &location("x.sfv"),
            ChecksumLimits::default(),
            &cancel
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::Interrupted
    );
}

#[test]
fn sfv_names_that_resemble_bsd_or_gnu_syntax_remain_exact() {
    for name in [
        "MD5 (report).txt".to_owned(),
        "MD5 (report) =".to_owned(),
        format!("{} file.txt", "ab".repeat(16)),
    ] {
        let text = format!("{name}  352441c2\n");
        let parsed = parse(text.as_bytes(), "files.sfv").unwrap();
        assert_eq!(parsed.algorithm, ChecksumAlgorithm::Crc32);
        assert_eq!(parsed.entries[0].path, location(&name));
        assert_eq!(parsed.entries[0].digest, vec![0x35, 0x24, 0x41, 0xc2]);
    }
}

#[test]
fn external_crc32_bsd_sfv_ambiguity_cannot_select_a_different_file() {
    for name in ["files.sfv", "files.txt"] {
        let result = parse(b"CRC32 (different.txt) =  352441c2\n", name);
        assert!(result.is_err(), "{name}");
    }
}

#[test]
fn overlapping_gnu_and_sfv_records_need_a_matching_extension_to_disambiguate() {
    let text = format!("{} *report 352441c2\n", "ab".repeat(16));
    let gnu = parse(text.as_bytes(), "files.md5").unwrap();
    assert_eq!(gnu.algorithm, ChecksumAlgorithm::Md5);
    assert_eq!(gnu.entries[0].path, location("report 352441c2"));
    let sfv = parse(text.as_bytes(), "files.sfv").unwrap();
    assert_eq!(sfv.algorithm, ChecksumAlgorithm::Crc32);
    assert_eq!(
        sfv.entries[0].path,
        location(&format!("{} *report", "ab".repeat(16)))
    );
    assert!(parse(text.as_bytes(), "files.txt").is_err());
}
