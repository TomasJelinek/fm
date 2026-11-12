use midnight_filesystem_api::{ErrorCode, Format, StagePurpose};
use midnight_fs_archive::{
    Archive, Limits,
    storage::{MemoryStorage, Storage},
};
use std::{
    cell::Cell,
    io::{self, Cursor, Write},
    rc::Rc,
};

#[derive(Clone, Copy, PartialEq)]
enum Point {
    Append,
    Progress,
    Seal,
}
struct Controlled {
    inner: MemoryStorage,
    cancel: Cell<bool>,
    point: Point,
    checkpoints: Cell<usize>,
    seals: Cell<usize>,
}
impl Controlled {
    fn checkpoint(&self, point: Point) -> io::Result<()> {
        if point == self.point {
            self.checkpoints.set(self.checkpoints.get() + 1);
            if self.cancel.get() {
                return Err(io::ErrorKind::Interrupted.into());
            }
        }
        Ok(())
    }
}
impl Storage for Controlled {
    fn begin(&self, purpose: StagePurpose, expected: Option<u64>, maximum: u64) -> io::Result<u64> {
        self.inner.begin(purpose, expected, maximum)
    }
    fn append(&self, stage: u64, offset: u64, bytes: &[u8]) -> io::Result<()> {
        self.checkpoint(Point::Append)?;
        self.inner.append(stage, offset, bytes)
    }
    fn progress(&self, _: u64, _: u64) -> io::Result<()> {
        self.checkpoint(Point::Progress)
    }
    fn seal(&self, stage: u64, length: u64) -> io::Result<()> {
        self.checkpoint(Point::Seal)?;
        self.seals.set(self.seals.get() + 1);
        self.inner.seal(stage, length)
    }
    fn read(&self, stage: u64, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.inner.read(stage, offset, length)
    }
}
fn verify(format: Format, bytes: Vec<u8>, point: Point, entry: usize, expected: &[u8]) {
    let storage = Rc::new(Controlled {
        inner: MemoryStorage::default(),
        cancel: Cell::new(true),
        point,
        checkpoints: Cell::new(0),
        seals: Cell::new(0),
    });
    let length = bytes.len() as u64;
    let mut archive = Archive::open_stream(
        format,
        Box::new(Cursor::new(bytes)),
        length,
        storage.clone(),
        Limits::default(),
    )
    .unwrap();
    let nodes = archive.list(1, 0).unwrap();
    let id = nodes[entry].id;
    assert_eq!(archive.prepare(id).unwrap_err().code, ErrorCode::Cancelled);
    assert_eq!(
        storage.checkpoints.get(),
        1,
        "cancel must unwind instead of continuing checkpoints"
    );
    assert_eq!(storage.seals.get(), 0, "canceled content cannot seal");
    storage.cancel.set(false);
    assert_eq!(archive.list(1, 0).unwrap(), nodes);
    let ready = archive.prepare(id).unwrap();
    assert_eq!(
        storage
            .read(ready.stage, ready.offset, expected.len() as u32)
            .unwrap(),
        expected
    );
    assert_eq!(storage.seals.get(), 1);
}
#[test]
fn zip_cancelled_append_or_seal_can_retry_on_same_archive() {
    for point in [Point::Append, Point::Seal] {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("image", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&vec![9; 600_000]).unwrap();
        verify(
            Format::Zip,
            writer.finish().unwrap().into_inner(),
            point,
            0,
            &[9; 4],
        );
    }
}
#[test]
fn solid_sevenzip_cancellation_during_sibling_discard_preserves_retry() {
    use sevenz_rust2::{
        ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, SourceReader,
    };
    let mut writer = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    writer.set_content_methods(vec![EncoderConfiguration::new(EncoderMethod::COPY)]);
    writer
        .push_archive_entries(
            vec![
                ArchiveEntry::new_file("sibling"),
                ArchiveEntry::new_file("image"),
            ],
            vec![
                SourceReader::new(Cursor::new(vec![3; 600_000])),
                SourceReader::new(Cursor::new(b"image".to_vec())),
            ],
        )
        .unwrap();
    verify(
        Format::SevenZip,
        writer.finish().unwrap().into_inner(),
        Point::Progress,
        1,
        b"image",
    );
}
