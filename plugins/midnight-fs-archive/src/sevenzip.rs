//! Metadata-only mounting; each requested compression block is verified atomically.
use crate::*;
use sevenz_rust2::{ArchiveEntry, BlockDecoder, EncoderMethod, Password};
use std::io::Cursor;

#[cfg(test)]
#[path = "sevenzip_tests.rs"]
mod tests;

pub(crate) struct Content {
    source: Cursor<Vec<u8>>,
    metadata: sevenz_rust2::Archive,
    members: HashMap<u64, usize>,
    file_nodes: Vec<Option<u64>>,
}

impl Archive {
    pub(crate) fn sevenzip(&mut self, source: Vec<u8>) -> Result<(), Error> {
        let source_length = source.len() as u64;
        let mut source = Cursor::new(source);
        // This parses and verifies the 7z index (including compressed headers), not bodies.
        let metadata =
            sevenz_rust2::Archive::read(&mut source, &Password::empty()).map_err(seven_error)?;
        if metadata.files.len() > self.limits.max_entries {
            return Err(Error::limit());
        }
        if metadata.stream_map.file_block_index.len() != metadata.files.len() {
            return Err(corrupt_block());
        }
        let mut total = 0u64;
        let mut block_sizes = vec![0u64; metadata.blocks.len()];
        for (index, entry) in metadata.files.iter().enumerate() {
            total = total.checked_add(entry.size).ok_or_else(Error::limit)?;
            if entry.size > self.limits.max_entry || total > self.limits.max_total {
                return Err(Error::limit());
            }
            if entry.size == 0 && entry.has_crc && entry.crc != 0 {
                return Err(corrupt_block());
            }
            if let Some(block) = metadata.stream_map.file_block_index[index] {
                let size = block_sizes.get_mut(block).ok_or_else(corrupt_block)?;
                *size = size.checked_add(entry.size).ok_or_else(Error::limit)?;
            } else if entry.has_stream || entry.size != 0 {
                return Err(corrupt_block());
            }
        }
        if total > source_length.saturating_mul(self.limits.max_ratio) {
            return Err(Error::limit());
        }
        for (block, size) in metadata.blocks.iter().zip(block_sizes) {
            if size != block.get_unpack_size() {
                return Err(corrupt_block());
            }
            if size == 0 && block.has_crc && block.crc != 0 {
                return Err(corrupt_block());
            }
            // Keep unsupported/encrypted inputs explicit even though payload decoding is deferred.
            if block
                .coders
                .iter()
                .any(|coder| !supported(coder.encoder_method_id()))
            {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "Encrypted or unsupported 7z codec",
                ));
            }
        }
        let mut file_nodes = Vec::with_capacity(metadata.files.len());
        for entry in &metadata.files {
            let kind = kind(entry);
            let readable = kind == Kind::File && tree::safe_name(entry.name.as_bytes());
            self.add(entry.name.as_bytes(), &entry.name, kind, None, readable)?;
            if readable {
                let id = self.indexed_size(entry.size)?;
                file_nodes.push(Some(id));
                if !entry.has_stream {
                    self.contents.insert(id, Vec::new());
                }
            } else {
                file_nodes.push(None);
            }
        }
        self.sevenzip_content = Some(Content::new(source, metadata, file_nodes)?);
        Ok(())
    }
}

impl Content {
    fn new(
        mut source: Cursor<Vec<u8>>,
        mut metadata: sevenz_rust2::Archive,
        original_nodes: Vec<Option<u64>>,
    ) -> Result<Self, Error> {
        // BlockDecoder's span length counts substreams, while a 7z file table may
        // interleave directories/empty no-stream files. Keep their original pane IDs,
        // but compact only the decoder's file table to actual stream ordinals.
        let original_files = std::mem::take(&mut metadata.files);
        let original_blocks = std::mem::take(&mut metadata.stream_map.file_block_index);
        let mut first = vec![0; metadata.blocks.len()];
        let mut counts = vec![0usize; metadata.blocks.len()];
        let mut file_nodes = Vec::new();
        let mut members = HashMap::new();
        for ((file, block), node) in original_files
            .into_iter()
            .zip(original_blocks)
            .zip(original_nodes)
        {
            if !file.has_stream {
                continue;
            }
            let block = block
                .filter(|block| *block < counts.len())
                .ok_or_else(corrupt_block)?;
            let index = metadata.files.len();
            if counts[block] == 0 {
                first[block] = index;
            } else if first[block] + counts[block] != index {
                return Err(corrupt_block());
            }
            counts[block] += 1;
            if let Some(id) = node {
                members.insert(id, index);
            }
            file_nodes.push(node);
            metadata.files.push(file);
            metadata.stream_map.file_block_index.push(Some(block));
        }
        metadata.stream_map.block_first_file_index = first;
        for (block, expected) in counts.into_iter().enumerate() {
            // No decoder span may index beyond the compact table, including empty blocks.
            if BlockDecoder::new(1, block, &metadata, &Password::empty(), &mut source).entry_count()
                != expected
            {
                return Err(corrupt_block());
            }
        }
        Ok(Self {
            source,
            metadata,
            members,
            file_nodes,
        })
    }

    pub(crate) fn decode(
        &mut self,
        entry: u64,
        limits: Limits,
    ) -> Result<HashMap<u64, Vec<u8>>, Error> {
        let index = *self.members.get(&entry).ok_or_else(corrupt_block)?;
        let block = self.metadata.stream_map.file_block_index[index].ok_or_else(corrupt_block)?;
        let mut file_index = self.metadata.stream_map.block_first_file_index[block];
        let block_size = self.metadata.blocks[block].get_unpack_size();
        let mut actual = 0u64;
        let mut staged = HashMap::new();
        let mut failure = None;
        let password = Password::empty();
        BlockDecoder::new(1, block, &self.metadata, &password, &mut self.source)
            .for_each_entries(&mut |file, reader| {
                let result = (|| {
                    // Even unsafe siblings must be drained to validate a solid block. Nothing
                    // enters the retained cache until every member and the block CRC succeed.
                    let bytes = bounded_read(
                        reader,
                        limits
                            .max_entry
                            .min(limits.max_total.saturating_sub(actual)),
                    )?;
                    if bytes.len() as u64 != file.size {
                        return Err(corrupt_block());
                    }
                    actual = actual
                        .checked_add(bytes.len() as u64)
                        .ok_or_else(Error::limit)?;
                    if let Some(id) = self.file_nodes.get(file_index).copied().flatten() {
                        staged.insert(id, bytes);
                    }
                    file_index += 1;
                    Ok(())
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
        if actual != block_size || !staged.contains_key(&entry) {
            return Err(corrupt_block());
        }
        Ok(staged)
    }
}

fn kind(entry: &ArchiveEntry) -> Kind {
    let mode = entry.windows_attributes >> 16;
    if entry.is_anti_item {
        Kind::Other
    } else if entry.has_windows_attributes
        && (entry.windows_attributes & 0x400 != 0 || mode & 0xf000 == 0xa000)
    {
        Kind::Link
    } else if entry.is_directory {
        Kind::Directory
    } else {
        Kind::File
    }
}

fn supported(method: &[u8]) -> bool {
    matches!(
        method,
        EncoderMethod::ID_COPY
            | EncoderMethod::ID_LZMA
            | EncoderMethod::ID_LZMA2
            | EncoderMethod::ID_BCJ_X86
            | EncoderMethod::ID_BCJ2
            | EncoderMethod::ID_BCJ_ARM
            | EncoderMethod::ID_BCJ_ARM64
            | EncoderMethod::ID_BCJ_ARM_THUMB
            | EncoderMethod::ID_BCJ_PPC
            | EncoderMethod::ID_BCJ_IA64
            | EncoderMethod::ID_BCJ_SPARC
            | EncoderMethod::ID_BCJ_RISCV
            | EncoderMethod::ID_DELTA
    )
}

fn corrupt_block() -> Error {
    Error::new(
        ErrorCode::Corrupt,
        "7z member or block length differs from its index",
    )
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
