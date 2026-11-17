use midnight_filesystem_api::{Format, StagePurpose};
use midnight_fs_archive::{
    Archive, Limits,
    storage::{MemoryStorage, Storage},
};
use std::{
    cell::Cell,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    rc::Rc,
};

struct Counted {
    inner: Cursor<Vec<u8>>,
    bytes: Rc<Cell<u64>>,
}
impl Read for Counted {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(output)?;
        self.bytes.set(self.bytes.get() + count as u64);
        Ok(count)
    }
}
impl Seek for Counted {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.inner.seek(from)
    }
}

#[test]
fn zip_mount_reads_index_without_copying_whole_source_and_extracts_selected_member() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "large",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    for _ in 0..32 {
        zip.write_all(&[7; 256 * 1024]).unwrap();
    }
    let source = zip.finish().unwrap().into_inner();
    let length = source.len() as u64;
    let reads = Rc::new(Cell::new(0));
    let storage = Rc::new(MemoryStorage::default());
    let mut archive = Archive::open_stream(
        Format::Zip,
        Box::new(Counted {
            inner: Cursor::new(source),
            bytes: reads.clone(),
        }),
        length,
        storage.clone(),
        Limits::default(),
    )
    .unwrap();
    assert!(
        reads.get() < 1024 * 1024,
        "index read {} of {length}",
        reads.get()
    );
    let member = archive.list(1, 0).unwrap().remove(0);
    let ready = archive.prepare(member.id).unwrap();
    assert_eq!(ready.length, 8 * 1024 * 1024);
    assert_eq!(
        storage
            .read(ready.stage, ready.offset + ready.length - 3, 3)
            .unwrap(),
        [7; 3]
    );
}

/// A captured source that clamps each read like the host broker and counts the
/// bytes it is asked for, which is what crosses the worker pipe.
struct Brokered {
    inner: Cursor<Vec<u8>>,
    requested: Rc<Cell<u64>>,
}
impl Read for Brokered {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let remaining = self.inner.get_ref().len() as u64 - self.inner.position();
        let count = (output.len() as u64).min(256 * 1024).min(remaining);
        self.requested.set(self.requested.get() + count);
        self.inner.read(&mut output[..count as usize])
    }
}
impl Seek for Brokered {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.inner.seek(from)
    }
}
fn open_brokered(format: Format, source: Vec<u8>) -> (Archive, Rc<Cell<u64>>) {
    let length = source.len() as u64;
    let requested = Rc::new(Cell::new(0));
    let archive = Archive::open_stream(
        format,
        Box::new(Brokered {
            inner: Cursor::new(source),
            requested: requested.clone(),
        }),
        length,
        Rc::new(MemoryStorage::default()),
        Limits::default(),
    )
    .unwrap();
    (archive, requested)
}

#[test]
fn many_member_zip_and_tar_indexes_fetch_about_their_own_size() {
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut tar = tar::Builder::new(Vec::new());
    for index in 0..1000 {
        let data = vec![index as u8; 1024];
        zip.start_file(format!("member-{index:04}.bin"), options)
            .unwrap();
        zip.write_all(&data).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("member-{index:04}.bin"),
            data.as_slice(),
        )
        .unwrap();
    }
    for (format, source) in [
        (Format::Zip, zip.finish().unwrap().into_inner()),
        (Format::Tar, tar.into_inner().unwrap()),
    ] {
        let length = source.len() as u64;
        let (archive, requested) = open_brokered(format, source);
        assert_eq!(archive.list(1, 0).unwrap().len(), 128);
        eprintln!(
            "{format:?}: index fetched {} of {length} bytes",
            requested.get()
        );
        // A seek-discarding 256 KiB buffer fetched about 1000 x 256 KiB here.
        assert!(
            requested.get() < 2 * length,
            "{format:?} index fetched {} bytes of a {length}-byte source",
            requested.get()
        );
    }
}

#[test]
fn tar_index_of_large_members_fetches_only_headers() {
    let mut tar = tar::Builder::new(Vec::new());
    for index in 0..16 {
        let mut header = tar::Header::new_gnu();
        header.set_size(1024 * 1024);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("large-{index}.bin"),
            vec![index; 1024 * 1024].as_slice(),
        )
        .unwrap();
    }
    let (archive, requested) = open_brokered(Format::Tar, tar.into_inner().unwrap());
    assert_eq!(archive.list(1, 0).unwrap().len(), 16);
    eprintln!("large-member TAR: index fetched {} bytes", requested.get());
    assert!(
        requested.get() < 1024 * 1024,
        "index fetched {} bytes",
        requested.get()
    );
}

#[test]
fn solid_sevenzip_members_cost_one_block_decode_together() {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, SourceReader,
    };
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    let payloads: Vec<Vec<u8>> = (0..20u8).map(|index| vec![index; 64 * 1024]).collect();
    writer
        .push_archive_entries(
            (0..20)
                .map(|index| ArchiveEntry::new_file(&format!("member-{index:02}")))
                .collect(),
            payloads
                .iter()
                .map(|bytes| SourceReader::new(Cursor::new(bytes.clone())))
                .collect(),
        )
        .unwrap();
    let source = writer.finish().unwrap().into_inner();
    let length = source.len() as u64;
    let (mut archive, requested) = open_brokered(Format::SevenZip, source);
    let opened = requested.get();
    let nodes = archive.list(1, 0).unwrap();
    // Copy-out order and a reversed browse: every member is still read exactly.
    for node in nodes.iter().chain(nodes.iter().rev()).take(30) {
        let index = node.display[7..].parse::<usize>().unwrap();
        let (bytes, _) = archive.read(node.id, 0, 256 * 1024).unwrap();
        assert_eq!(bytes, payloads[index]);
    }
    // One verified block pass serves all 20 members; reading each one separately
    // used to decode the 1.3 MiB block 20 times.
    eprintln!(
        "solid 7z: member reads fetched {} of {length} bytes",
        requested.get() - opened
    );
    assert!(
        requested.get() - opened < length + 256 * 1024,
        "member reads fetched {} bytes of a {length}-byte archive",
        requested.get() - opened
    );
}

struct DiskFull;
impl Storage for DiskFull {
    fn begin(&self, _: StagePurpose, _: Option<u64>, _: u64) -> io::Result<u64> {
        Ok(1)
    }
    fn append(&self, _: u64, _: u64, _: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::StorageFull,
            "fixture disk full",
        ))
    }
    fn seal(&self, _: u64, _: u64) -> io::Result<()> {
        panic!("failed stage must never seal")
    }
    fn read(&self, _: u64, _: u64, _: u32) -> io::Result<Vec<u8>> {
        panic!("failed stage must never read")
    }
}
#[test]
fn storage_failure_cannot_publish_content() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("a", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"a").unwrap();
    let source = zip.finish().unwrap().into_inner();
    let length = source.len() as u64;
    let mut archive = Archive::open_stream(
        Format::Zip,
        Box::new(Cursor::new(source)),
        length,
        Rc::new(DiskFull),
        Limits::default(),
    )
    .unwrap();
    let error = archive.prepare(2).unwrap_err();
    assert!(error.message.contains("disk full"));
    assert_eq!(error.code, midnight_filesystem_api::ErrorCode::Io);
}

struct SparseTar {
    header: Vec<u8>,
    position: u64,
    length: u64,
    reads: Rc<Cell<u64>>,
}
impl Read for SparseTar {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = (self.length - self.position).min(output.len() as u64) as usize;
        output[..count].fill(0);
        if self.position < self.header.len() as u64 {
            let start = self.position as usize;
            let copied = count.min(self.header.len() - start);
            output[..copied].copy_from_slice(&self.header[start..start + copied]);
        }
        self.position += count as u64;
        self.reads.set(self.reads.get() + count as u64);
        Ok(count)
    }
}
impl Seek for SparseTar {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .filter(|position| *position <= self.length)
        .ok_or(io::ErrorKind::InvalidInput)?;
        self.position = position;
        Ok(position)
    }
}
#[test]
fn sparse_five_gibibyte_tar_indexes_with_u64_offsets_and_bounded_reads() {
    let length = 5 * 1024 * 1024 * 1024;
    let mut header = tar::Header::new_gnu();
    header.set_path("large.bin").unwrap();
    header.set_size(length);
    header.set_mode(0o600);
    header.set_cksum();
    let reads = Rc::new(Cell::new(0));
    let source = SparseTar {
        header: header.as_bytes().to_vec(),
        position: 0,
        length: length + 1536,
        reads: reads.clone(),
    };
    let archive = Archive::open_stream(
        Format::Tar,
        Box::new(source),
        length + 1536,
        Rc::new(MemoryStorage::default()),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(archive.list(1, 0).unwrap()[0].size, Some(length));
    assert!(
        reads.get() < 1024 * 1024,
        "sparse index read {} bytes",
        reads.get()
    );
}
