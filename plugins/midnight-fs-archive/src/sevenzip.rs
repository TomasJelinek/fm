//! Metadata-only mounting; each requested compression block is verified atomically.
use crate::*;
use sevenz_rust2::{ArchiveEntry, Block, BlockDecoder, EncoderMethod, Password};
use std::{
    collections::HashSet,
    io::{Seek, SeekFrom},
    ops::Range,
};

/// Largest LZMA/LZMA2 dictionary memory one block may use in the worker.
/// Members of a block that declares more are listed but unreadable.
const MAX_DICTIONARY: u64 = 256 * 1024 * 1024;
/// Verified solid-block siblings kept in worker memory, so the members read after
/// one block decode within this window need no further decode of that block.
const RETAINED_SIBLINGS: u64 = 128 * 1024 * 1024;
// A decodable block's dictionary and the siblings retained beside it stay within
// three eighths of the worker's memory; the rest holds the index and the decoders.
const _: () = assert!(MAX_DICTIONARY + RETAINED_SIBLINGS <= WORKER_MEMORY / 8 * 3);
/// Largest raw next header, which the parser reads whole into memory. Reading and
/// indexing one peaks at about five times its size (an uncompressed header of a
/// million members with 25-character paths: 50 MiB, peak 245 MiB, measured by
/// `measure_index_memory`), so an eighth of the worker's memory: 128 MiB holds the
/// uncompressed header of about two and a half million such members. A compressed
/// header (7-Zip's default) is a small raw next header whatever the archive holds.
const MAX_RAW_HEADER: u64 = WORKER_MEMORY / 8;
/// Allocation and map overhead charged per retained sibling.
const RETAINED_ENTRY_COST: u64 = 64;

#[cfg(test)]
#[path = "sevenzip_tests.rs"]
mod tests;

pub(crate) struct Content {
    source: Reader,
    metadata: sevenz_rust2::Archive,
    /// The compact stream index of each readable member, in increasing ID order.
    members: Vec<(u64, usize)>,
    /// Readable member node of each compact stream index.
    nodes: Vec<Option<u64>>,
    /// Compact stream indices of each block.
    block_files: Vec<Range<usize>>,
    /// Readable members without a stream, in increasing ID order.
    empty: Vec<u64>,
    retained: Retained,
}

/// Bytes of members of one block whose complete decode, including every member
/// length and checksum, was verified by an earlier read. Nothing unverified enters it.
#[derive(Default)]
struct Retained {
    block: Option<usize>,
    members: HashMap<u64, Vec<u8>>,
    cost: u64,
}
impl Retained {
    fn insert(&mut self, entry: u64, bytes: Vec<u8>) {
        self.cost += bytes.len() as u64 + RETAINED_ENTRY_COST;
        self.members.insert(entry, bytes);
    }
    fn remove(&mut self, entry: u64) {
        if let Some(bytes) = self.members.remove(&entry) {
            self.cost -= bytes.len() as u64 + RETAINED_ENTRY_COST;
        }
    }
}

impl Archive {
    pub(crate) fn sevenzip(&mut self, mut source: Reader, source_length: u64) -> Result<(), Error> {
        preflight_header(&mut source, source_length)?;
        // This parses and verifies the 7z index (including compressed headers), not bodies.
        let metadata =
            sevenz_rust2::Archive::read(&mut source, &Password::empty()).map_err(seven_error)?;
        self.index_sevenzip(source, metadata)
    }

    fn index_sevenzip(
        &mut self,
        source: Reader,
        mut metadata: sevenz_rust2::Archive,
    ) -> Result<(), Error> {
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
        let mut undecodable = HashSet::new();
        for (index, (block, size)) in metadata.blocks.iter().zip(block_sizes).enumerate() {
            if size != block.get_unpack_size() {
                return Err(corrupt_block());
            }
            if size == 0 && block.has_crc && block.crc != 0 {
                return Err(corrupt_block());
            }
            // Payload decoding is deferred: a block with an encrypted or unsupported
            // coder, or with a dictionary beyond the worker budget, only makes its own
            // members unreadable, as ZIP does per member.
            if !decodable(block, dictionary_bytes(block)) {
                undecodable.insert(index);
            }
        }
        let mut file_nodes = Vec::with_capacity(metadata.files.len());
        for (index, entry) in metadata.files.iter_mut().enumerate() {
            let kind = kind(entry);
            let decodable = metadata.stream_map.file_block_index[index]
                .is_none_or(|block| !undecodable.contains(&block));
            // The index keeps its own copy; block decoding never reads entry names.
            let name = std::mem::take(&mut entry.name);
            let readable = kind == Kind::File && decodable && tree::safe_name(name.as_bytes());
            let id = self.add(name.as_bytes(), &name, kind, None, readable)?;
            if let Some(member) = self.member_mut(id) {
                member.modified = entry
                    .has_last_modified_date
                    .then(|| timestamps::filetime(u64::from(entry.last_modified_date)));
            }
            if readable {
                let id = self.indexed_size(entry.size)?;
                file_nodes.push(Some(id));
            } else {
                file_nodes.push(None);
            }
        }
        self.sevenzip_content = Some(Content::new(source, metadata, file_nodes)?);
        Ok(())
    }
}

fn decodable(block: &Block, dictionary: u64) -> bool {
    dictionary <= MAX_DICTIONARY
        && block
            .coders
            .iter()
            .all(|coder| supported(coder.encoder_method_id()))
}

/// Dictionary bytes the pinned decoders allocate for one block. Malformed coder
/// properties count as zero here; the decoder rejects them when the block is read.
fn dictionary_bytes(block: &Block) -> u64 {
    block
        .coders
        .iter()
        .enumerate()
        .map(|(index, coder)| {
            coder_dictionary(
                coder.encoder_method_id(),
                coder.properties(),
                block.get_unpack_size_at_index(index),
            )
        })
        .fold(0u64, u64::saturating_add)
}

fn coder_dictionary(method: &[u8], properties: &[u8], unpacked: u64) -> u64 {
    match method {
        EncoderMethod::ID_LZMA => match properties.get(1..5) {
            // LZMA never keeps more history than its own output length.
            Some(size) => {
                u64::from(u32::from_le_bytes([size[0], size[1], size[2], size[3]])).min(unpacked)
            }
            None => 0,
        },
        EncoderMethod::ID_LZMA2 => match properties.first() {
            Some(40) => u64::from(u32::MAX),
            Some(&bits) if bits < 40 => {
                u64::from(2 | (u32::from(bits) & 1)) << (u32::from(bits) / 2 + 11)
            }
            _ => 0,
        },
        _ => 0,
    }
}

fn preflight_header(source: &mut Reader, source_length: u64) -> Result<(), Error> {
    source.seek(SeekFrom::Start(0))?;
    let mut header = [0; 32];
    source.read_exact(&mut header)?;
    if header[..6] != [0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c] {
        return Err(Error::new(ErrorCode::Corrupt, "Invalid 7z signature"));
    }
    let mut offset = [0; 8];
    offset.copy_from_slice(&header[12..20]);
    let mut length = [0; 8];
    length.copy_from_slice(&header[20..28]);
    let offset = u64::from_le_bytes(offset);
    let length = u64::from_le_bytes(length);
    if 32u64
        .checked_add(offset)
        .and_then(|start| start.checked_add(length))
        .is_none_or(|end| end > source_length)
    {
        return Err(Error::new(
            ErrorCode::Corrupt,
            "7z next header range exceeds the captured source",
        ));
    }
    if length > MAX_RAW_HEADER {
        const MIB: u64 = 1024 * 1024;
        return Err(Error::new(
            ErrorCode::Limit,
            format!(
                "This 7z archive's uncompressed header of {} MiB needs more memory than \
                 the archive worker has: it reads headers of up to {} MiB",
                length.div_ceil(MIB),
                MAX_RAW_HEADER / MIB
            ),
        ));
    }
    // The pinned parser verifies header CRCs. Its optional recovery scan is bounded
    // to 1 MiB. Encoded-header output has no public limit hook; process memory
    // containment remains necessary for it. Payload dictionaries are checked against
    // `MAX_DICTIONARY` from the public coder properties before any member decode.
    source.seek(SeekFrom::Start(0))?;
    Ok(())
}

impl Content {
    fn new(
        mut source: Reader,
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
        let mut empty = Vec::new();
        let mut members = Vec::new();
        let mut nodes = Vec::new();
        // Readable member IDs increase in file order, so both lists stay sorted.
        for ((file, block), node) in original_files
            .into_iter()
            .zip(original_blocks)
            .zip(original_nodes)
        {
            if !file.has_stream {
                if let Some(node) = node {
                    empty.push(node);
                }
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
                members.push((id, index));
            }
            nodes.push(node);
            metadata.files.push(file);
            metadata.stream_map.file_block_index.push(Some(block));
        }
        members.shrink_to_fit();
        empty.shrink_to_fit();
        nodes.shrink_to_fit();
        metadata.files.shrink_to_fit();
        metadata.stream_map.file_block_index.shrink_to_fit();
        let block_files = first
            .iter()
            .zip(&counts)
            .map(|(first, count)| *first..*first + *count)
            .collect();
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
            nodes,
            block_files,
            empty,
            retained: Retained::default(),
        })
    }

    pub(crate) fn decode(
        &mut self,
        entry: u64,
        storage: Rc<dyn Storage>,
    ) -> Result<storage::Content, Error> {
        if self.empty.binary_search(&entry).is_ok() {
            return Stage::new(storage, StagePurpose::Member { entry }, Some(0), 0)?.finish();
        }
        let index = *tree::find(&self.members, entry).ok_or_else(corrupt_block)?;
        let length = self.metadata.files[index].size;
        if let Some(bytes) = self.retained.members.get(&entry) {
            // Verified together with its whole block by an earlier read.
            let mut staged = Stage::new(
                storage,
                StagePurpose::Member { entry },
                Some(length),
                length,
            )?;
            staged.copy(bytes.as_slice())?;
            let content = staged.finish()?;
            // Published: the host lease cache serves repeated reads.
            self.retained.remove(entry);
            return Ok(content);
        }
        let block = self.metadata.stream_map.file_block_index[index].ok_or_else(corrupt_block)?;
        let mut file_index = self.metadata.stream_map.block_first_file_index[block];
        let block_size = self.metadata.blocks[block].get_unpack_size();
        let mut staged = Stage::new(
            storage,
            StagePurpose::Member { entry },
            Some(length),
            length,
        )?;
        if self.retained.block != Some(block) {
            // Siblings of another block give way to the block decoded now. Siblings of
            // this block stay, so a cancelled decode does not discard verified bytes.
            self.retained = Retained {
                block: Some(block),
                ..Retained::default()
            };
        }
        let keep = self.retention(block, index);
        let mut kept = Vec::new();
        let mut actual = 0u64;
        let mut failure = None;
        let password = Password::empty();
        BlockDecoder::new(1, block, &self.metadata, &password, &mut self.source)
            .for_each_entries(&mut |file, reader| {
                let result = (|| {
                    // Even unsafe siblings must be drained to validate a solid block. Nothing
                    // enters the retained cache until every member and the block CRC succeed.
                    let count = if file_index == index {
                        staged.copy(reader)?
                    } else if let Some(node) = keep.get(&file_index) {
                        let bytes = staged.retain(reader, file.size)?;
                        let count = bytes.len() as u64;
                        kept.push((*node, bytes));
                        count
                    } else {
                        staged.discard(reader)?
                    };
                    if count != file.size {
                        return Err(corrupt_block());
                    }
                    actual = actual.checked_add(count).ok_or_else(Error::limit)?;
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
        if actual != block_size {
            return Err(corrupt_block());
        }
        for (node, bytes) in kept {
            self.retained.insert(node, bytes);
        }
        staged.finish()
    }

    /// Readable siblings worth keeping from one decode of `block`: later members
    /// first (sequential copy-out and thumbnails), then earlier ones nearest first,
    /// within the retention budget beside those already kept. A member outside this
    /// window that is read later costs another decode of the whole block.
    fn retention(&self, block: usize, requested: usize) -> HashMap<usize, u64> {
        let files = self.block_files[block].clone();
        let mut cost = self.retained.cost;
        let mut keep = HashMap::new();
        for index in (requested + 1..files.end).chain((files.start..requested).rev()) {
            let Some(node) = self.nodes[index] else {
                continue;
            };
            if self.retained.members.contains_key(&node) {
                continue;
            }
            let next = cost
                .saturating_add(self.metadata.files[index].size)
                .saturating_add(RETAINED_ENTRY_COST);
            if next <= RETAINED_SIBLINGS {
                cost = next;
                keep.insert(index, node);
            }
        }
        keep
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
    if let E::Io(io, _) = &error
        && let Some(failure) = Failure::of(io)
    {
        return failure.clone().into();
    }
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
