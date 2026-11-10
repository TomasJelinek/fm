use super::{buffer::Buffer, *};
use std::mem::{offset_of, size_of};

fn number(buffer: &mut Buffer, offset: usize, value: u32) {
    buffer.bytes_mut()[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}
fn pointer(buffer: &mut Buffer, offset: usize, value: usize) {
    buffer.bytes_mut()[offset..offset + size_of::<usize>()].copy_from_slice(&value.to_ne_bytes());
}
fn text(buffer: &mut Buffer, offset: usize, units: &[u16]) -> usize {
    for (position, unit) in units.iter().chain(Some(&0)).enumerate() {
        buffer.bytes_mut()[offset + position * 2..offset + position * 2 + 2]
            .copy_from_slice(&unit.to_ne_bytes());
    }
    buffer.pointer() as usize + offset
}
fn fixture() -> Buffer {
    let mut buffer = Buffer::new(512).unwrap();
    number(
        &mut buffer,
        offset_of!(NETRESOURCEW, dwScope),
        RESOURCE_GLOBALNET,
    );
    number(
        &mut buffer,
        offset_of!(NETRESOURCEW, dwType),
        RESOURCETYPE_DISK,
    );
    number(
        &mut buffer,
        offset_of!(NETRESOURCEW, dwDisplayType),
        resource::DISPLAY_SERVER,
    );
    number(
        &mut buffer,
        offset_of!(NETRESOURCEW, dwUsage),
        RESOURCEUSAGE_CONTAINER,
    );
    let remote = text(&mut buffer, 128, &[92, 92, 0xd800, 0x6587]);
    pointer(&mut buffer, offset_of!(NETRESOURCEW, lpRemoteName), remote);
    let provider = text(
        &mut buffer,
        256,
        &"Provider".encode_utf16().collect::<Vec<_>>(),
    );
    pointer(&mut buffer, offset_of!(NETRESOURCEW, lpProvider), provider);
    buffer
}
#[test]
fn validated_native_buffer_preserves_raw_utf16_and_null_fields_in_owned_records() {
    let buffer = fixture();
    let records = buffer.parse(1).unwrap();
    drop(buffer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].remote, Some(vec![92, 92, 0xd800, 0x6587]));
    assert_eq!(records[0].local, None);
    assert_eq!(
        records[0].provider,
        Some("Provider".encode_utf16().collect())
    );
}
#[test]
fn pointers_outside_allocation_inside_headers_or_misaligned_are_never_dereferenced() {
    for field in [
        offset_of!(NETRESOURCEW, lpLocalName),
        offset_of!(NETRESOURCEW, lpRemoteName),
        offset_of!(NETRESOURCEW, lpComment),
        offset_of!(NETRESOURCEW, lpProvider),
    ] {
        for position in [
            0,
            1,
            size_of::<NETRESOURCEW>() - 2,
            129,
            512,
            514,
            usize::MAX,
        ] {
            let mut buffer = fixture();
            let base = buffer.pointer() as usize;
            pointer(&mut buffer, field, base.wrapping_add(position));
            assert!(buffer.parse(1).is_err(), "invalid string offset {position}");
        }
    }
}
#[test]
fn missing_terminator_count_overflow_and_short_headers_are_refused() {
    let mut buffer = fixture();
    let base = buffer.pointer() as usize;
    buffer.bytes_mut()[510..].fill(1);
    pointer(
        &mut buffer,
        offset_of!(NETRESOURCEW, lpRemoteName),
        base + 510,
    );
    assert!(buffer.parse(1).is_err());
    assert!(fixture().parse(u32::MAX).is_err());
    assert!(fixture().parse(0).is_err());
    assert!(
        Buffer::new(size_of::<NETRESOURCEW>() - 8)
            .unwrap()
            .parse(1)
            .is_err()
    );
}
#[test]
fn repeated_aliases_cannot_expand_a_page_beyond_its_retained_byte_budget() {
    let mut buffer = Buffer::new(buffer::MAX_BUFFER).unwrap();
    let raw = vec![120u16; 20_000];
    let remote = text(&mut buffer, 16 * 1024, &raw);
    for index in 0..128 {
        let start = index * size_of::<NETRESOURCEW>();
        number(
            &mut buffer,
            start + offset_of!(NETRESOURCEW, dwScope),
            RESOURCE_GLOBALNET,
        );
        pointer(
            &mut buffer,
            start + offset_of!(NETRESOURCEW, lpRemoteName),
            remote,
        );
    }
    assert_eq!(
        buffer.parse(128).unwrap_err().code,
        ErrorCode::ResourceLimit
    );
}
