//! Hand-built 7z archives of empty files with an uncompressed header, which the pinned
//! writer never produces: it always compresses headers.

/// A 7z number in its longest form: a marker byte and eight little-endian bytes.
fn number(output: &mut Vec<u8>, value: usize) {
    output.push(0xff);
    output.extend((value as u64).to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

/// Empty files named `names`, in this order, behind an uncompressed header.
pub fn archive(names: &[String]) -> Vec<u8> {
    let count = names.len();
    // kHeader, kFilesInfo and the number of files.
    let mut header = vec![0x01, 0x05];
    number(&mut header, count);
    // kEmptyStream and kEmptyFile: every member is a file without data.
    for property in [0x0e, 0x0f] {
        header.push(property);
        number(&mut header, count.div_ceil(8));
        header.resize(header.len() + count.div_ceil(8), 0xff);
    }
    let mut utf16 = Vec::new();
    for name in names {
        utf16.extend(name.encode_utf16().flat_map(u16::to_le_bytes));
        utf16.extend([0, 0]);
    }
    // kName, its size, inline (not external) names, then the ends of the file
    // properties and of the header.
    header.push(0x11);
    number(&mut header, utf16.len() + 1);
    header.push(0);
    header.extend(utf16);
    header.extend([0, 0]);
    let mut archive = vec![0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c, 0, 4, 0, 0, 0, 0];
    archive.extend(0u64.to_le_bytes());
    archive.extend((header.len() as u64).to_le_bytes());
    archive.extend(crc32(&header).to_le_bytes());
    let start = crc32(&archive[12..32]);
    archive[8..12].copy_from_slice(&start.to_le_bytes());
    archive.extend(header);
    archive
}
