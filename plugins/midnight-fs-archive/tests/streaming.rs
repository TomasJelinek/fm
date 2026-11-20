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
fn open_brokered(format: Format, source: Vec<u8>) -> (Archive, Rc<Cell<u64>>, Rc<MemoryStorage>) {
    let length = source.len() as u64;
    let requested = Rc::new(Cell::new(0));
    let storage = Rc::new(MemoryStorage::default());
    let archive = Archive::open_stream(
        format,
        Box::new(Brokered {
            inner: Cursor::new(source),
            requested: requested.clone(),
        }),
        length,
        storage.clone(),
        Limits::default(),
    )
    .unwrap();
    (archive, requested, storage)
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
        let (archive, requested, _) = open_brokered(format, source);
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
    let (archive, requested, _) = open_brokered(Format::Tar, tar.into_inner().unwrap());
    assert_eq!(archive.list(1, 0).unwrap().len(), 16);
    eprintln!("large-member TAR: index fetched {} bytes", requested.get());
    assert!(
        requested.get() < 1024 * 1024,
        "index fetched {} bytes",
        requested.get()
    );
}

#[test]
fn solid_sevenzip_members_share_block_decodes_in_production_reads() {
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
    let (mut archive, requested, storage) = open_brokered(Format::SevenZip, source);
    let members: Vec<(u64, usize)> = archive
        .list(1, 0)
        .unwrap()
        .iter()
        .map(|node| (node.id, node.display[7..].parse().unwrap()))
        .collect();
    // `prepare` is what the worker answers each host Read with; `Archive::read` would
    // also consult the fixture-only content cache.
    let mut fetched = |order: &[(u64, usize)]| {
        let before = requested.get();
        for (id, index) in order {
            let content = archive.prepare(*id).unwrap();
            let bytes = storage
                .read(content.stage, content.offset, content.length as u32)
                .unwrap();
            assert_eq!(bytes, payloads[*index]);
        }
        requested.get() - before
    };
    // Copy-out order: one verified block decode serves all 20 members; reading each
    // one separately used to decode the 1.3 MiB block 20 times.
    let forward = fetched(&members);
    // A sibling stays in the worker only until it is staged; repeated reads normally
    // hit the host's lease cache. Reading them all again, in reverse, decodes the
    // block exactly once more: that first read retains the others, nearest first.
    let reversed: Vec<_> = members.iter().rev().copied().collect();
    let again = fetched(&reversed);
    eprintln!("solid 7z: members fetched {forward} then {again} bytes of a {length}-byte archive");
    for pass in [forward, again] {
        assert!(
            pass > length / 2 && pass < length + 256 * 1024,
            "a pass over all members fetched {pass} bytes of a {length}-byte archive"
        );
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct SourceCounts {
    reads: u64,
    seeks: u64,
    bytes: u64,
    largest_read: usize,
}

struct CountingReadSeek {
    inner: Cursor<Vec<u8>>,
    counts: Rc<Cell<SourceCounts>>,
}
impl Read for CountingReadSeek {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(output)?;
        let mut counts = self.counts.get();
        counts.reads += 1;
        counts.bytes += read as u64;
        counts.largest_read = counts.largest_read.max(output.len());
        self.counts.set(counts);
        Ok(read)
    }
}
impl Seek for CountingReadSeek {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let mut counts = self.counts.get();
        counts.seeks += 1;
        self.counts.set(counts);
        self.inner.seek(from)
    }
}

fn directory_nodes(archive: &Archive, directory: u64) -> Vec<midnight_filesystem_api::Node> {
    let mut nodes = Vec::new();
    let mut after = 0;
    loop {
        let batch = archive.list(directory, after).unwrap();
        let Some(last) = batch.last() else {
            return nodes;
        };
        after = last.id;
        nodes.extend(batch);
    }
}

#[test]
fn many_entry_zip_listing_reuses_source_bytes_without_changing_member_identity() {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.add_directory("folder/", options).unwrap();
    for index in 0..2048 {
        writer
            .start_file(format!("folder/member-{index:04}.txt"), options)
            .unwrap();
        write!(writer, "contents for member {index:04}").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let length = bytes.len() as u64;
    let counts = Rc::new(Cell::new(SourceCounts::default()));
    let mut archive = Archive::open_stream(
        Format::Zip,
        Box::new(CountingReadSeek {
            inner: Cursor::new(bytes.clone()),
            counts: counts.clone(),
        }),
        length,
        Rc::new(MemoryStorage::default()),
        Limits::default(),
    )
    .unwrap();
    let root = archive.list(1, 0).unwrap();
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].id, 2);
    assert_eq!(root[0].parent, 1);
    assert_eq!(root[0].name, b"folder");
    assert_eq!(root[0].kind, midnight_filesystem_api::Kind::Directory);
    let nodes = directory_nodes(&archive, 2);
    assert_eq!(nodes.len(), 2048);
    let indexed = counts.get();
    let reopened = Archive::open(Format::Zip, bytes, Limits::default()).unwrap();
    assert_eq!(root, reopened.list(1, 0).unwrap());
    assert_eq!(nodes, directory_nodes(&reopened, 2));
    for (index, id, name, contents) in [
        (0, 3, "member-0000.txt", b"contents for member 0000"),
        (1024, 1027, "member-1024.txt", b"contents for member 1024"),
        (2047, 2050, "member-2047.txt", b"contents for member 2047"),
    ] {
        let node = &nodes[index];
        assert_eq!(node.id, id);
        assert_eq!(node.parent, 2);
        assert_eq!(node.name, name.as_bytes());
        assert_eq!(node.display, name);
        assert_eq!(node.kind, midnight_filesystem_api::Kind::File);
        assert_eq!(node.size, Some(24));
        assert!(node.readable);
        assert_eq!(archive.read(id, 0, 24).unwrap(), (contents.to_vec(), true));
    }
    eprintln!(
        "zip_members=2048 source_bytes={length} index={indexed:?} after_three_members={:?}; \
         root=(2,folder) samples=(3,member-0000.txt),(1027,member-1024.txt),(2050,member-2047.txt)",
        counts.get()
    );
    // Count at the production broker's Read + Seek boundary, independently of time.
    // A metadata listing must not reread a large buffer for every tiny entry.
    assert!(
        indexed.bytes <= length * 4 + 8 * u64::from(midnight_filesystem_api::MAX_READ),
        "many-entry index amplified source reads: {indexed:?}, source bytes={length}"
    );
    assert!(indexed.largest_read <= midnight_filesystem_api::MAX_READ as usize);
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

/// Memory staging that counts reads of staged streams.
struct ReadCounting {
    inner: MemoryStorage,
    reads: Rc<Cell<u64>>,
}
impl Storage for ReadCounting {
    fn begin(&self, purpose: StagePurpose, expected: Option<u64>, maximum: u64) -> io::Result<u64> {
        self.inner.begin(purpose, expected, maximum)
    }
    fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.inner.append(stage, offset, bytes)
    }
    fn seal(&self, stage: u64, length: u64) -> io::Result<()> {
        self.inner.seal(stage, length)
    }
    fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.reads.set(self.reads.get() + u64::from(length));
        self.inner.read(stage, offset, length)
    }
}

/// 300 small members and one of 1 MiB, the large one in the middle of the stream.
fn gzip_tar() -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    let mut members = Vec::new();
    for index in 0..300u32 {
        members.push((
            format!("small-{index:03}.bin"),
            vec![(index % 251) as u8; 3000],
        ));
        if index == 150 {
            members.push((
                "large.bin".to_owned(),
                (0..1024 * 1024u32).map(|i| (i % 253) as u8).collect(),
            ));
        }
    }
    let mut tar = tar::Builder::new(Vec::new());
    for (name, data) in &members {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, name, data.as_slice()).unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    (gzip.finish().unwrap(), members)
}

#[test]
fn compressed_tar_is_indexed_while_staged_and_never_read_back_to_list() {
    let (source, members) = gzip_tar();
    let length = source.len() as u64;
    let reads = Rc::new(Cell::new(0));
    let mut archive = Archive::open_stream(
        Format::TarGzip,
        Box::new(Cursor::new(source)),
        length,
        Rc::new(ReadCounting {
            inner: MemoryStorage::default(),
            reads: reads.clone(),
        }),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(reads.get(), 0, "opening read the staged stream back");
    let nodes = directory_nodes(&archive, 1);
    assert_eq!(nodes.len(), members.len());
    assert_eq!(reads.get(), 0, "listing read the staged stream back");
    for (name, data) in [&members[0], &members[151], &members[300]] {
        let node = nodes
            .iter()
            .find(|node| node.name == name.as_bytes())
            .unwrap();
        let offset = data.len() as u64 / 2;
        let (bytes, _) = archive.read(node.id, offset, 1000).unwrap();
        let start = offset as usize;
        assert_eq!(
            bytes,
            &data[start..(start + 1000).min(data.len())],
            "{name}"
        );
    }
}

#[test]
fn compressed_tar_staging_failures_keep_their_codes() {
    use midnight_filesystem_api::ErrorCode;
    let (source, _) = gzip_tar();
    let limited = Limits {
        max_total: 64 * 1024,
        ..Limits::default()
    };
    let error = Archive::open(Format::TarGzip, source.clone(), limited)
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::Limit);
    let error = Archive::open_stream(
        Format::TarGzip,
        Box::new(Cursor::new(source.clone())),
        source.len() as u64,
        Rc::new(DiskFull),
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, ErrorCode::Io);
    assert!(error.message.contains("disk full"), "{}", error.message);
    // A stream cut inside the large member and one with a damaged trailer.
    let mut damaged = source.clone();
    let end = damaged.len();
    damaged[end - 5] ^= 1;
    for broken in [source[..source.len() / 2].to_vec(), damaged] {
        let error = Archive::open(Format::TarGzip, broken, Limits::default())
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::Corrupt, "{}", error.message);
    }
}
