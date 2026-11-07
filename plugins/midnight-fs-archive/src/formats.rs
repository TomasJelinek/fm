use crate::*;
use std::io::Cursor;

impl Archive {
    pub(crate) fn zip(&mut self, source: &[u8]) -> Result<(), Error> {
        let mut zip = zip::ZipArchive::new(Cursor::new(source)).map_err(zip_error)?;
        if zip.len() > self.limits.max_entries {
            return Err(Error::limit());
        }
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
            let readable =
                kind == Kind::File && supported && !raw.encrypted() && tree::safe_name(&name);
            if raw.size() > self.limits.max_entry
                || raw.size()
                    > raw
                        .compressed_size()
                        .max(1)
                        .saturating_mul(self.limits.max_ratio)
            {
                return Err(Error::limit());
            }
            drop(raw);
            let data = if readable {
                Some(bounded_read(
                    zip.by_index(index).map_err(zip_error)?,
                    self.limits
                        .max_entry
                        .min(self.limits.max_total.saturating_sub(self.total)),
                )?)
            } else {
                None
            };
            self.add(&name, &display, kind, data, readable)?;
        }
        Ok(())
    }

    pub(crate) fn tar(&mut self, source: &[u8]) -> Result<(), Error> {
        let mut archive = tar::Archive::new(Cursor::new(source));
        for entry in archive.entries()? {
            let entry = entry?;
            let raw = entry.path_bytes().into_owned();
            let display = String::from_utf8_lossy(&raw).into_owned();
            let typ = entry.header().entry_type();
            let kind = if typ.is_dir() {
                Kind::Directory
            } else if typ.is_file() {
                Kind::File
            } else if typ.is_symlink() || typ.is_hard_link() {
                Kind::Link
            } else {
                Kind::Other
            };
            let readable = kind == Kind::File && tree::safe_name(&raw);
            if entry.size() > self.limits.max_entry {
                return Err(Error::limit());
            }
            let data = if readable {
                Some(bounded_read(
                    entry,
                    self.limits
                        .max_entry
                        .min(self.limits.max_total.saturating_sub(self.total)),
                )?)
            } else {
                None
            };
            self.add(&raw, &display, kind, data, readable)?;
        }
        Ok(())
    }

    pub(crate) fn sevenzip(&mut self, source: &[u8]) -> Result<(), Error> {
        let mut archive =
            sevenz_rust2::ArchiveReader::new(Cursor::new(source), sevenz_rust2::Password::empty())
                .map_err(seven_error)?;
        archive.set_thread_count(1);
        if archive.archive().files.len() > self.limits.max_entries {
            return Err(Error::limit());
        }
        let mut total = 0u64;
        for entry in &archive.archive().files {
            total = total.checked_add(entry.size).ok_or_else(Error::limit)?;
            if entry.size > self.limits.max_entry || total > self.limits.max_total {
                return Err(Error::limit());
            }
        }
        if total > (source.len() as u64).saturating_mul(self.limits.max_ratio) {
            return Err(Error::limit());
        }
        let mut failure = None;
        archive
            .for_each_entries(|entry, reader| {
                let result = (|| {
                    // Reparse points and Unix symlink mode carried in Windows attributes are not files.
                    let mode = entry.windows_attributes >> 16;
                    let kind = if entry.is_anti_item {
                        Kind::Other
                    } else if entry.has_windows_attributes
                        && (entry.windows_attributes & 0x400 != 0 || mode & 0xf000 == 0xa000)
                    {
                        Kind::Link
                    } else if entry.is_directory {
                        Kind::Directory
                    } else {
                        Kind::File
                    };
                    let readable = kind == Kind::File && tree::safe_name(entry.name.as_bytes());
                    // Drain each solid member exactly once. The decoder validates checksums at EOF.
                    let data = bounded_read(reader, self.limits.max_entry)?;
                    self.add(
                        entry.name.as_bytes(),
                        &entry.name,
                        kind,
                        readable.then_some(data),
                        readable,
                    )
                })();
                match result {
                    Ok(()) => Ok(true),
                    Err(error) => {
                        failure = Some(error);
                        Ok(false)
                    }
                }
            })
            .map_err(seven_error)?;
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }
}
fn zip_error(error: zip::result::ZipError) -> Error {
    let code = match error {
        zip::result::ZipError::UnsupportedArchive(_) => ErrorCode::Unsupported,
        _ => ErrorCode::Corrupt,
    };
    Error::new(code, error.to_string())
}
fn seven_error(error: sevenz_rust2::Error) -> Error {
    use sevenz_rust2::Error as E;
    let code = match error {
        E::PasswordRequired
        | E::UnsupportedCompressionMethod(_)
        | E::Unsupported(_)
        | E::ExternalUnsupported => ErrorCode::Unsupported,
        E::MaxMemLimited { .. } => ErrorCode::Limit,
        _ => ErrorCode::Corrupt,
    };
    Error::new(code, error.to_string())
}
