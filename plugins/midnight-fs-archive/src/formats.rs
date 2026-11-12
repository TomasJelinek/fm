use crate::*;

impl Archive {
    pub(crate) fn zip(&mut self, source: Reader) -> Result<(), Error> {
        let mut zip = zip::ZipArchive::new(source).map_err(zip_error)?;
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
            let size = raw.size();
            let modified = timestamps::zip_modified(raw.extra_data_fields(), raw.last_modified());
            drop(raw);
            let id = self.add(&name, &display, kind, None, readable)?;
            if let Some(node) = self.nodes.get_mut((id - 2) as usize) {
                node.modified = modified.filter(|time| time.validate().is_ok());
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
pub(crate) fn zip_error(error: zip::result::ZipError) -> Error {
    let code = match error {
        zip::result::ZipError::UnsupportedArchive(_) => ErrorCode::Unsupported,
        _ => ErrorCode::Corrupt,
    };
    Error::new(code, error.to_string())
}
