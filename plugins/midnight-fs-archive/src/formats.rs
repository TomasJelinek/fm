use crate::*;
use std::{
    cell::RefCell,
    collections::HashSet,
    io::{Seek, SeekFrom},
};

const CENTRAL_HEADER: [u8; 4] = *b"PK\x01\x02";

impl Archive {
    pub(crate) fn zip(&mut self, source: Reader) -> Result<(), Error> {
        let source = Shared(Rc::new(RefCell::new(source)));
        let mut zip = zip::ZipArchive::new(source.clone()).map_err(zip_error)?;
        if zip.len() > self.limits.max_entries {
            return Err(Error::limit());
        }
        let ambiguous = merged_names(
            source,
            zip.central_directory_start(),
            self.limits.max_entries,
        )?;
        for index in 0..zip.len() {
            let raw = zip.by_index_raw(index).map_err(zip_error)?;
            let name = raw.name_raw().to_vec();
            let display = raw.name().to_owned();
            let kind = if raw.is_symlink() {
                Kind::Link
            } else if raw.is_dir() {
                Kind::Directory
            } else {
                Kind::File
            };
            let supported = matches!(
                raw.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            );
            let readable = kind == Kind::File
                && supported
                && !raw.encrypted()
                && tree::safe_name(&name)
                && !ambiguous.contains(name.as_slice());
            if raw.size() > self.limits.max_entry {
                return Err(Error::limit());
            }
            let size = raw.size();
            let modified = timestamps::zip_modified(raw.extra_data_fields(), raw.last_modified());
            drop(raw);
            let id = self.add(&name, &display, kind, None, readable)?;
            if let Some(node) = self.node_mut(id) {
                node.modified = modified;
            }
            if readable {
                self.total = self.total.checked_add(size).ok_or_else(Error::limit)?;
                if self.total > self.limits.max_total {
                    return Err(Error::limit());
                }
                let node = self.nodes.last_mut().ok_or_else(Error::limit)?;
                node.size = Some(size);
                self.zip_members.insert(node.id, index);
            }
        }
        self.zip = Some(zip);
        Ok(())
    }
}

/// zip 8.6 indexes central-directory records by raw name, so records repeating a
/// name collapse into one entry carrying the last record's data, and the earlier
/// records become unreachable. Walk the records themselves to find such names:
/// their surviving entry is listed but unreadable instead of silently ambiguous.
fn merged_names(
    mut source: Shared,
    start: u64,
    max_entries: usize,
) -> Result<HashSet<Box<[u8]>>, Error> {
    let mut seen = HashSet::new();
    let mut merged = HashSet::new();
    source.seek(SeekFrom::Start(start))?;
    let mut header = [0; 46];
    for _ in 0..=max_entries {
        let mut signature = [0; 4];
        match source.read_exact(&mut signature) {
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(merged),
            result => result?,
        }
        if signature != CENTRAL_HEADER {
            return Ok(merged);
        }
        source.read_exact(&mut header[4..])?;
        let field = |offset: usize| u16::from_le_bytes([header[offset], header[offset + 1]]);
        let mut name = vec![0; usize::from(field(28))].into_boxed_slice();
        source.read_exact(&mut name)?;
        source.seek(SeekFrom::Current(
            i64::from(field(30)) + i64::from(field(32)),
        ))?;
        if let Some(name) = seen.replace(name) {
            merged.insert(name);
        }
    }
    Err(Error::limit())
}

/// The ZIP reader and the record walk share one read window. Both seek before
/// reading, and no borrow outlives a single read or seek.
#[derive(Clone)]
pub(crate) struct Shared(Rc<RefCell<Reader>>);
impl Shared {
    fn reader(&self) -> io::Result<std::cell::RefMut<'_, Reader>> {
        self.0
            .try_borrow_mut()
            .map_err(|_| io::Error::other("ZIP source is already in use"))
    }
}
impl Read for Shared {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reader()?.read(output)
    }
}
impl Seek for Shared {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.reader()?.seek(from)
    }
    fn stream_position(&mut self) -> io::Result<u64> {
        self.reader()?.stream_position()
    }
}

pub(crate) fn zip_error(error: zip::result::ZipError) -> Error {
    let code = match error {
        zip::result::ZipError::UnsupportedArchive(_) => ErrorCode::Unsupported,
        _ => ErrorCode::Corrupt,
    };
    Error::new(code, error.to_string())
}
