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
