//! Safe bounded fixture storage and observable positional-source counters.
use midnight_office_preview::{Cancellation, OfficeError, OfficeSource};
use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom, Write},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

enum Data {
    Fill(u8),
    Bytes(Vec<u8>),
}
struct Segment {
    start: u64,
    length: u64,
    data: Data,
}

/// Encoded output from the REAL ZIP/deflate writer. Long byte runs have compact
/// storage, so a valid level-zero deflate stream need not occupy 130 MiB on disk/RAM.
#[derive(Default)]
pub struct Tape {
    segments: Vec<Segment>,
    patches: BTreeMap<u64, Vec<u8>>,
    length: u64,
    literal_bytes: usize,
}

impl Tape {
    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn literal_payload_bytes(&self) -> usize {
        self.literal_bytes
    }

    fn append_fill(&mut self, value: u8, length: usize) {
        if let Some(Segment {
            length: previous,
            data: Data::Fill(old),
            ..
        }) = self.segments.last_mut()
            && *old == value
        {
            *previous += length as u64;
        } else {
            self.segments.push(Segment {
                start: self.length,
                length: length as u64,
                data: Data::Fill(value),
            });
        }
        self.length += length as u64;
    }

    fn append_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(Segment {
            length,
            data: Data::Bytes(previous),
            ..
        }) = self.segments.last_mut()
            && previous.len() + bytes.len() <= 4096
        {
            previous.extend_from_slice(bytes);
            *length += bytes.len() as u64;
        } else {
            self.segments.push(Segment {
                start: self.length,
                length: bytes.len() as u64,
                data: Data::Bytes(bytes.to_vec()),
            });
        }
        self.length += bytes.len() as u64;
        self.literal_bytes += bytes.len();
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut literal = 0;
        let mut position = 0;
        while position < bytes.len() {
            let mut end = position + 1;
            while end < bytes.len() && bytes[end] == bytes[position] {
                end += 1;
            }
            if end - position >= 32 {
                self.append_bytes(&bytes[literal..position]);
                self.append_fill(bytes[position], end - position);
                literal = end;
            }
            position = end;
        }
        self.append_bytes(&bytes[literal..]);
        // A fixture-generation failure is explicit, not a production-limit override.
        if self.segments.len() > 100_000 || self.literal_payload_bytes() > 8 * 1024 * 1024 {
            return Err(io::Error::other("performance fixture storage limit"));
        }
        Ok(())
    }

    fn patch(&mut self, start: u64, bytes: &[u8]) -> io::Result<()> {
        let end = start + bytes.len() as u64;
        let mut keys: Vec<_> = self
            .patches
            .range(start..end)
            .map(|(&key, _)| key)
            .collect();
        if let Some((&key, old)) = self.patches.range(..start).next_back()
            && key + old.len() as u64 > start
        {
            keys.push(key);
        }
        for key in keys {
            let old = self.patches.remove(&key).unwrap();
            self.literal_bytes -= old.len();
            if key < start {
                let prefix = old[..(start - key) as usize].to_vec();
                self.literal_bytes += prefix.len();
                self.patches.insert(key, prefix);
            }
            if key + old.len() as u64 > end {
                let suffix = old[(end - key) as usize..].to_vec();
                self.literal_bytes += suffix.len();
                self.patches.insert(end, suffix);
            }
        }
        self.literal_bytes += bytes.len();
        self.patches.insert(start, bytes.to_vec());
        if self.patches.len() > 100_000 || self.literal_bytes > 8 * 1024 * 1024 {
            return Err(io::Error::other("performance fixture patch storage limit"));
        }
        Ok(())
    }

    pub fn read_at(&self, offset: u64, output: &mut [u8]) -> io::Result<usize> {
        if offset > self.length {
            return Err(io::Error::other("fixture offset beyond EOF"));
        }
        let count = output.len().min((self.length - offset) as usize);
        if count == 0 {
            return Ok(0);
        }
        let mut index = self
            .segments
            .partition_point(|segment| segment.start <= offset)
            .checked_sub(1)
            .ok_or_else(|| io::Error::other("fixture segment missing"))?;
        let mut position = offset;
        let mut written = 0;
        while written < count {
            let segment = self
                .segments
                .get(index)
                .ok_or_else(|| io::Error::other("fixture gap"))?;
            let within = (position - segment.start) as usize;
            let length = (count - written).min(segment.length as usize - within);
            let part = &mut output[written..written + length];
            match &segment.data {
                Data::Fill(value) => part.fill(*value),
                Data::Bytes(bytes) => part.copy_from_slice(&bytes[within..within + length]),
            }
            written += length;
            position += length as u64;
            index += 1;
        }
        // ZIP backpatches local CRC/length fields. Preserve exact last-write-wins bytes.
        let apply = |start: u64, bytes: &[u8], output: &mut [u8]| {
            let begin = offset.max(start);
            let end = (offset + count as u64).min(start + bytes.len() as u64);
            if begin < end {
                output[(begin - offset) as usize..(end - offset) as usize]
                    .copy_from_slice(&bytes[(begin - start) as usize..(end - start) as usize]);
            }
        };
        if let Some((&start, bytes)) = self.patches.range(..=offset).next_back() {
            apply(start, bytes, output);
        }
        for (&start, bytes) in self.patches.range(offset + 1..offset + count as u64) {
            apply(start, bytes, output);
        }
        Ok(count)
    }

    pub fn reader(&self) -> TapeReader<'_> {
        TapeReader {
            tape: self,
            position: 0,
        }
    }
}

#[derive(Default)]
pub struct TapeWriter {
    tape: Tape,
    position: u64,
}
impl TapeWriter {
    pub fn finish(self) -> Tape {
        self.tape
    }
}
impl Write for TapeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.position == self.tape.length {
            self.tape.append(bytes)?;
        } else {
            if self.position + bytes.len() as u64 > self.tape.length || bytes.len() > 65_536 {
                return Err(io::Error::other("unsupported fixture backpatch"));
            }
            self.tape.patch(self.position, bytes)?;
        }
        self.position += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn seek_position(position: u64, length: u64, from: SeekFrom) -> io::Result<u64> {
    let next = match from {
        SeekFrom::Start(value) => i128::from(value),
        SeekFrom::Current(value) => i128::from(position) + i128::from(value),
        SeekFrom::End(value) => i128::from(length) + i128::from(value),
    };
    if next < 0 || next > i128::from(length) {
        return Err(io::Error::other("fixture seek"));
    }
    Ok(next as u64)
}
impl Seek for TapeWriter {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.tape.length, from)?;
        Ok(self.position)
    }
}
pub struct TapeReader<'a> {
    tape: &'a Tape,
    position: u64,
}
impl Read for TapeReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.tape.read_at(self.position, output)?;
        self.position += count as u64;
        Ok(count)
    }
}
impl Seek for TapeReader<'_> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = seek_position(self.position, self.tape.length, from)?;
        Ok(self.position)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub calls: u64,
    pub delivered: u64,
    /// Calls/bytes whose delivered range intersects the compressed worksheet member.
    pub payload_calls: u64,
    pub payload_delivered: u64,
    /// Separate calls/bytes intersecting the actual compressed shared-string member.
    pub shared_string_calls: u64,
    pub shared_string_delivered: u64,
    pub offset_jumps: u64,
    pub backward_jumps: u64,
    pub calls_entered_cancelled: u64,
    pub cancel_triggers: u64,
    pub max_requested: usize,
}
impl Counts {
    /// Counters are phase deltas; max_requested remains the whole-source high-water mark.
    pub fn since(self, before: Self) -> Self {
        Self {
            calls: self.calls - before.calls,
            delivered: self.delivered - before.delivered,
            payload_calls: self.payload_calls - before.payload_calls,
            payload_delivered: self.payload_delivered - before.payload_delivered,
            shared_string_calls: self.shared_string_calls - before.shared_string_calls,
            shared_string_delivered: self.shared_string_delivered - before.shared_string_delivered,
            offset_jumps: self.offset_jumps - before.offset_jumps,
            backward_jumps: self.backward_jumps - before.backward_jumps,
            calls_entered_cancelled: self.calls_entered_cancelled - before.calls_entered_cancelled,
            cancel_triggers: self.cancel_triggers - before.cancel_triggers,
            max_requested: self.max_requested,
        }
    }
}

pub struct Source {
    tape: Arc<Tape>,
    payload: Range<u64>,
    shared_strings: Range<u64>,
    chunk: usize,
    calls: AtomicU64,
    delivered: AtomicU64,
    payload_calls: AtomicU64,
    payload_delivered: AtomicU64,
    shared_string_calls: AtomicU64,
    shared_string_delivered: AtomicU64,
    offset_jumps: AtomicU64,
    backward_jumps: AtomicU64,
    last_end: AtomicU64,
    calls_entered_cancelled: AtomicU64,
    cancel_triggers: AtomicU64,
    max_requested: AtomicUsize,
    arm_after: AtomicU64,
    armed_delivered: AtomicU64,
}
impl Source {
    pub fn new(
        tape: Arc<Tape>,
        payload: Range<u64>,
        shared_strings: Range<u64>,
        chunk: usize,
    ) -> Arc<Self> {
        assert!((1..=65_536).contains(&chunk));
        Arc::new(Self {
            tape,
            payload,
            shared_strings,
            chunk,
            calls: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
            payload_calls: AtomicU64::new(0),
            payload_delivered: AtomicU64::new(0),
            shared_string_calls: AtomicU64::new(0),
            shared_string_delivered: AtomicU64::new(0),
            offset_jumps: AtomicU64::new(0),
            backward_jumps: AtomicU64::new(0),
            last_end: AtomicU64::new(u64::MAX),
            calls_entered_cancelled: AtomicU64::new(0),
            cancel_triggers: AtomicU64::new(0),
            max_requested: AtomicUsize::new(0),
            arm_after: AtomicU64::new(0),
            armed_delivered: AtomicU64::new(0),
        })
    }
    pub fn arm_cancel_after_payload_bytes(&self, bytes: u64) {
        assert!(bytes > 0);
        self.armed_delivered.store(0, Ordering::Relaxed);
        self.arm_after.store(bytes, Ordering::Release);
    }
    pub fn counts(&self) -> Counts {
        Counts {
            calls: self.calls.load(Ordering::Relaxed),
            delivered: self.delivered.load(Ordering::Relaxed),
            payload_calls: self.payload_calls.load(Ordering::Relaxed),
            payload_delivered: self.payload_delivered.load(Ordering::Relaxed),
            shared_string_calls: self.shared_string_calls.load(Ordering::Relaxed),
            shared_string_delivered: self.shared_string_delivered.load(Ordering::Relaxed),
            offset_jumps: self.offset_jumps.load(Ordering::Relaxed),
            backward_jumps: self.backward_jumps.load(Ordering::Relaxed),
            calls_entered_cancelled: self.calls_entered_cancelled.load(Ordering::Relaxed),
            cancel_triggers: self.cancel_triggers.load(Ordering::Relaxed),
            max_requested: self.max_requested.load(Ordering::Relaxed),
        }
    }
}
impl OfficeSource for Source {
    fn len(&self) -> u64 {
        self.tape.len()
    }
    fn is_empty(&self) -> bool {
        self.tape.is_empty()
    }
    fn read_at(&self, offset: u64, output: &mut [u8], cancel: &Cancellation) -> io::Result<usize> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.max_requested
            .fetch_max(output.len(), Ordering::Relaxed);
        if cancel.is_cancelled() {
            self.calls_entered_cancelled.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other(OfficeError::Cancelled));
        }
        let length = output.len().min(self.chunk);
        let count = self.tape.read_at(offset, &mut output[..length])?;
        self.delivered.fetch_add(count as u64, Ordering::Relaxed);
        let end = offset + count as u64;
        let previous = self.last_end.swap(end, Ordering::Relaxed);
        if previous != u64::MAX && previous != offset {
            self.offset_jumps.fetch_add(1, Ordering::Relaxed);
            if offset < previous {
                self.backward_jumps.fetch_add(1, Ordering::Relaxed);
            }
        }
        let delivered = end
            .min(self.payload.end)
            .saturating_sub(offset.max(self.payload.start));
        if delivered > 0 {
            self.payload_calls.fetch_add(1, Ordering::Relaxed);
            self.payload_delivered
                .fetch_add(delivered, Ordering::Relaxed);
            let threshold = self.arm_after.load(Ordering::Acquire);
            if threshold != 0 {
                let seen = self.armed_delivered.fetch_add(delivered, Ordering::Relaxed) + delivered;
                if seen >= threshold
                    && self
                        .arm_after
                        .compare_exchange(threshold, 0, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    self.cancel_triggers.fetch_add(1, Ordering::Relaxed);
                    cancel.cancel();
                }
            }
        }
        // Separate actual encoded-member delivery; never infer decompressed output.
        // Cancellation remains armed only by worksheet payload, and the entry gate
        // above prevents either member from delivering bytes on a cancelled call.
        let shared = end
            .min(self.shared_strings.end)
            .saturating_sub(offset.max(self.shared_strings.start));
        if shared > 0 {
            self.shared_string_calls.fetch_add(1, Ordering::Relaxed);
            self.shared_string_delivered
                .fetch_add(shared, Ordering::Relaxed);
        }
        Ok(count)
    }
}
