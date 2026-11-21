use super::*;
use windows_sys::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
};

#[test]
fn executable_icon_rejects_invalid_or_cancelled_requests() {
    let cancel = Cancellation::default();
    for path in ["tool.exe", "C:\\tool\0.exe"] {
        assert!(load(Path::new(path), 64, &cancel).is_none());
    }
    let path = std::env::current_exe().unwrap();
    assert!(load(&path, 0, &cancel).is_none());
    assert!(load(&path, 65, &cancel).is_none());
    cancel.cancel();
    assert!(load(&path, 64, &cancel).is_none());
    assert!(load_file(&path, 64, &cancel).is_none());
}

#[test]
fn executable_icon_extracts_real_unicode_path_resources_without_launching() {
    let directory = crate::test_fixtures::build_temp("fixture-");
    let source = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
    let target = directory.path().join("executable 文档.exe");
    std::fs::copy(source, &target).unwrap();
    let pixels = load(&target, 64, &Cancellation::default()).expect("system executable icon");
    assert_eq!(
        (pixels.width, pixels.height, pixels.rgba.len()),
        (64, 64, 16384)
    );
    assert!(
        pixels
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0)
    );
}

fn frame(edge: u32, depth: u16, at: usize) -> Frame {
    Frame {
        width: edge,
        height: edge,
        depth,
        at,
        bytes: 1,
    }
}

#[test]
fn frames_are_tried_from_the_smallest_large_enough_then_the_largest_smaller() {
    let frames = [
        frame(16, 32, 1),
        frame(48, 8, 2),
        frame(48, 32, 3),
        frame(32, 32, 4),
        frame(256, 32, 5),
        frame(2048, 32, 6),
        frame(0, 32, 7),
    ];
    let order =
        |edge| -> Vec<usize> { ranked(&frames, edge).iter().map(|frame| frame.at).collect() };
    // Exact first; the deeper of two equal frames before the shallower one.
    assert_eq!(order(48), [3, 2, 5, 4, 1]);
    assert_eq!(order(20), [4, 3, 2, 5, 1]);
    assert_eq!(order(16), [1, 4, 3, 2, 5]);
    // Nothing large enough: never a smaller frame while a larger one exists.
    assert_eq!(order(100), [5, 3, 2, 4, 1]);
    assert_eq!(order(256), [5, 3, 2, 4, 1]);
    // Frames beyond the drawing bound and empty ones are never tried.
    assert!(ranked(&[frame(2048, 32, 1), frame(0, 32, 2)], 16).is_empty());
}

#[test]
fn icon_directories_are_parsed_within_their_bounds() {
    // Two GRPICONDIR entries: 256 (written as zero) by 32 bits as resource 7, and 16
    // by 4 bits as resource 9.
    let mut group = vec![0, 0, 1, 0, 2, 0];
    group.extend([0, 0, 0, 0, 1, 0, 32, 0, 0x10, 0, 0, 0, 7, 0]);
    group.extend([16, 16, 16, 0, 1, 0, 4, 0, 0x20, 0, 0, 0, 9, 0]);
    assert_eq!(
        group_frames(&group),
        [
            Frame {
                width: 256,
                height: 256,
                depth: 32,
                at: 7,
                bytes: 0x10
            },
            Frame {
                width: 16,
                height: 16,
                depth: 4,
                at: 9,
                bytes: 0x20
            }
        ]
    );
    // A count beyond the bytes present reads only whole entries.
    let mut truncated = group.clone();
    truncated[4] = 9;
    truncated.truncate(6 + 14 + 5);
    assert_eq!(group_frames(&truncated).len(), 1);
    // Not an icon directory: a cursor, a bad reserved field, a short header.
    for broken in [&[0, 0, 2, 0, 1, 0][..], &[1, 0, 1, 0, 1, 0], &[0, 0, 1]] {
        assert!(group_frames(broken).is_empty(), "{broken:?}");
    }
    // At most MAX_FRAMES entries are considered.
    let mut many = vec![0, 0, 1, 0, 0xFF, 0xFF];
    for _ in 0..MAX_FRAMES + 10 {
        many.extend([16, 16, 0, 0, 1, 0, 32, 0, 1, 0, 0, 0, 1, 0]);
    }
    assert_eq!(group_frames(&many).len(), MAX_FRAMES);
    // ICONDIR entries end with the frame's file offset.
    let mut file = vec![0, 0, 1, 0, 1, 0];
    file.extend([32, 32, 0, 0, 1, 0, 32, 0, 0x30, 0, 0, 0, 0x16, 0, 0, 0]);
    assert_eq!(
        file_frames(&file),
        [Frame {
            width: 32,
            height: 32,
            depth: 32,
            at: 0x16,
            bytes: 0x30
        }]
    );
}

#[test]
fn frame_headers_report_the_true_size_of_bitmap_and_png_frames() {
    assert_eq!(frame_header(&bitmap(24, [1, 2, 3])), Some((24, 24, 32)));
    assert_eq!(frame_header(&png(300, [1, 2, 3])), Some((300, 300, 32)));
    let mut huge = png(16, [1, 2, 3]);
    huge[16..20].copy_from_slice(&4096u32.to_be_bytes());
    assert_eq!(frame_header(&huge), None, "beyond the drawing bound");
    assert_eq!(
        frame_header(&bitmap(24, [1, 2, 3])[..12]),
        None,
        "truncated"
    );
    assert_eq!(frame_header(b"not an icon frame at all, but long"), None);
}

/// A 32-bit bitmap icon frame of one opaque color: a BITMAPINFOHEADER whose height
/// covers the color and mask rows, bottom-up BGRA rows and an empty AND mask.
fn bitmap(edge: u32, [red, green, blue]: [u8; 3]) -> Vec<u8> {
    let mask_row = edge.div_ceil(32) * 4;
    let mut bits = Vec::new();
    bits.extend(40u32.to_le_bytes());
    bits.extend((edge as i32).to_le_bytes());
    bits.extend((edge as i32 * 2).to_le_bytes());
    bits.extend(1u16.to_le_bytes());
    bits.extend(32u16.to_le_bytes());
    bits.extend([0; 24]);
    for _ in 0..edge * edge {
        bits.extend([blue, green, red, 255]);
    }
    bits.extend(vec![0; (mask_row * edge) as usize]);
    bits
}

/// A PNG frame of one opaque color, with stored (uncompressed) deflate blocks.
fn png(edge: u32, [red, green, blue]: [u8; 3]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
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
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend(kind);
        out.extend(data);
        let crc = crc32(&out[start..]);
        out.extend(crc.to_be_bytes());
    }
    // Each row: filter type 0 (none), then its pixels.
    let mut row = vec![0];
    row.extend([red, green, blue, 255].repeat(edge as usize));
    let raw = row.repeat(edge as usize);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (index, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(index + 1 == blocks.len()));
        let length = block.len() as u16;
        zlib.extend(length.to_le_bytes());
        zlib.extend((!length).to_le_bytes());
        zlib.extend(*block);
    }
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut header = Vec::new();
    header.extend(edge.to_be_bytes());
    header.extend(edge.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

const RED: [u8; 3] = [220, 20, 20];
const GREEN: [u8; 3] = [20, 200, 20];
const BLUE: [u8; 3] = [20, 20, 220];
const YELLOW: [u8; 3] = [230, 210, 10];

/// Frames of 16 (red), 32 (green), 48 (blue) and 256 px (yellow, PNG).
fn fixture_frames() -> Vec<Vec<u8>> {
    vec![
        bitmap(16, RED),
        bitmap(32, GREEN),
        bitmap(48, BLUE),
        png(256, YELLOW),
    ]
}

/// An icon file holding `frames` in the given order.
fn icon_file(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut file = vec![0, 0, 1, 0];
    file.extend((frames.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * frames.len();
    for bits in frames {
        let (width, _, _) = frame_header(bits).unwrap();
        let byte = if width >= 256 { 0 } else { width as u8 };
        file.extend([byte, byte, 0, 0, 1, 0, 32, 0]);
        file.extend((bits.len() as u32).to_le_bytes());
        file.extend((offset as u32).to_le_bytes());
        offset += bits.len();
    }
    for bits in frames {
        file.extend(bits);
    }
    file
}

/// The color at the center of a drawn icon.
fn center(rgba: &[u8], edge: u16) -> [u8; 4] {
    let edge = usize::from(edge);
    rgba.as_chunks::<4>().0[edge / 2 * edge + edge / 2]
}

fn assert_color(actual: [u8; 4], expected: [u8; 3], context: &str) {
    let close = actual[..3]
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 4);
    assert!(
        close && actual[3] == 255,
        "{context}: {actual:?} is not {expected:?}"
    );
}

#[test]
fn icon_file_frames_are_chosen_by_the_requested_edge() {
    let directory = crate::test_fixtures::build_temp("icon-frames-");
    let path = directory.path().join("frames 文档.ico");
    // Stored largest first, so neither file order nor the first entry decides.
    let mut frames = fixture_frames();
    frames.reverse();
    std::fs::write(&path, icon_file(&frames)).unwrap();
    let cancel = Cancellation::default();
    for (edge, color, from) in [
        (16, RED, "the exact 16 px frame"),
        (20, GREEN, "the 32 px frame reduced"),
        (32, GREEN, "the exact 32 px frame"),
        (40, BLUE, "the 48 px frame reduced"),
        (96, YELLOW, "the 256 px PNG frame reduced"),
        (216, YELLOW, "the 256 px PNG frame reduced"),
        (256, YELLOW, "the exact 256 px PNG frame"),
    ] {
        let rgba = load_rgba(&path, 0, edge, &cancel).expect("icon file artwork");
        assert_eq!(rgba.len(), usize::from(edge) * usize::from(edge) * 4);
        assert_color(
            center(&rgba, edge),
            color,
            &format!("{edge} px from {from}"),
        );
    }
}

/// A copy of a system program whose icon groups are replaced by `groups`, each a
/// list of frames, stored as RT_ICON resources 1, 2, … and RT_GROUP_ICON 1, 2, ….
fn program_with_icons(directory: &Path, groups: &[Vec<Vec<u8>>]) -> PathBuf {
    let source = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/cmd.exe");
    let target = directory.join("program 文档.exe");
    std::fs::copy(source, &target).unwrap();
    let wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: a terminated path of a private copy; every resource is replaced.
    let update = unsafe { BeginUpdateResourceW(wide.as_ptr(), 1) };
    assert!(!update.is_null(), "{}", io::Error::last_os_error());
    let mut id = 1u16;
    for (group_id, frames) in (1u16..).zip(groups) {
        let mut directory = vec![0, 0, 1, 0];
        directory.extend((frames.len() as u16).to_le_bytes());
        for bits in frames {
            let (width, _, _) = frame_header(bits).unwrap();
            let byte = if width >= 256 { 0 } else { width as u8 };
            directory.extend([byte, byte, 0, 0, 1, 0, 32, 0]);
            directory.extend((bits.len() as u32).to_le_bytes());
            directory.extend(id.to_le_bytes());
            // SAFETY: the handle is open; the data outlives the call.
            let added = unsafe {
                UpdateResourceW(
                    update,
                    RT_ICON,
                    id as usize as PCWSTR,
                    0,
                    bits.as_ptr().cast(),
                    bits.len() as u32,
                )
            };
            assert_ne!(added, 0, "{}", io::Error::last_os_error());
            id += 1;
        }
        // SAFETY: as above.
        let added = unsafe {
            UpdateResourceW(
                update,
                RT_GROUP_ICON,
                group_id as usize as PCWSTR,
                0,
                directory.as_ptr().cast(),
                directory.len() as u32,
            )
        };
        assert_ne!(added, 0, "{}", io::Error::last_os_error());
    }
    // SAFETY: commits and closes the update handle exactly once.
    assert_ne!(
        unsafe { EndUpdateResourceW(update, 0) },
        0,
        "{}",
        io::Error::last_os_error()
    );
    target
}

#[test]
fn program_frames_are_chosen_by_the_requested_edge_and_groups_by_index() {
    let directory = crate::test_fixtures::build_temp("program-frames-");
    let mut frames = fixture_frames();
    frames.swap(0, 3);
    let second = vec![bitmap(32, [90, 60, 30])];
    let path = program_with_icons(directory.path(), &[frames, second]);
    let cancel = Cancellation::default();
    for (edge, color) in [
        (16, RED),
        (24, GREEN),
        (48, BLUE),
        (64, YELLOW),
        (216, YELLOW),
        (256, YELLOW),
    ] {
        let rgba = load_rgba(&path, 0, edge, &cancel).expect("program artwork");
        assert_color(center(&rgba, edge), color, &format!("{edge} px"));
    }
    // The second group by its position, as icon indexes count groups.
    let rgba = load_rgba(&path, 1, 32, &cancel).expect("second group");
    assert_color(center(&rgba, 32), [90, 60, 30], "second group");
    // A listed program is read through its pinned final path.
    let pixels = load_file(&path, 216, &cancel).expect("listed program artwork");
    assert_eq!((pixels.width, pixels.height), (216, 216));
    assert_color(center(&pixels.rgba, 216), YELLOW, "listed program");
}

#[test]
fn a_program_without_large_frames_is_enlarged_from_its_largest() {
    let directory = crate::test_fixtures::build_temp("program-small-");
    let path = program_with_icons(directory.path(), &[vec![bitmap(16, RED), bitmap(48, BLUE)]]);
    let rgba = load_rgba(&path, 0, 200, &Cancellation::default()).expect("artwork");
    // The 48 px frame, not the 16 px one, fills the whole tile.
    assert_color(center(&rgba, 200), BLUE, "the largest frame");
    assert_color(rgba.as_chunks::<4>().0[2 * 200 + 2], BLUE, "a corner pixel");
}

#[test]
fn cancellation_before_the_frame_is_drawn_yields_nothing() {
    let directory = crate::test_fixtures::build_temp("program-cancel-");
    let path = program_with_icons(directory.path(), &[fixture_frames()]);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(load_rgba(&path, 0, 96, &cancel).is_none());
    assert!(load_file(&path, 96, &cancel).is_none());
}

#[test]
fn a_listed_file_that_is_a_link_or_not_a_disk_file_is_not_read() {
    let directory = crate::test_fixtures::build_temp("program-link-");
    let path = program_with_icons(directory.path(), &[fixture_frames()]);
    let link = directory.path().join("link.exe");
    if std::os::windows::fs::symlink_file(&path, &link).is_ok() {
        assert!(load_file(&link, 96, &Cancellation::default()).is_none());
    }
    assert!(load_file(directory.path(), 96, &Cancellation::default()).is_none());
    assert!(load_file(Path::new("relative.exe"), 96, &Cancellation::default()).is_none());
}
