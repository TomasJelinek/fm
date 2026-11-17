//! Seek-preserving read window over a brokered image source.
//!
//! Every refill is one host round trip that also counts against the host's
//! cumulative read budget. `std::io::BufReader` discards its buffer on every seek,
//! so seeking codecs such as TIFF refetched a whole 256 KiB window per strip or
//! tile. This window keeps fetched bytes across seeks and only refills on a miss.
use std::io::{self, BufRead, Read, Seek, SeekFrom};

/// Largest window; equals the brokered read limit.
const MAX_WINDOW: usize = 256 * 1024;
/// Window used after a jump. It doubles while reads continue sequentially, so a
/// codec that hops between distant strips does not fetch a maximum window each time.
const JUMP_WINDOW: usize = 32 * 1024;

pub(crate) struct Window<R> {
    inner: R,
    buffer: Vec<u8>,
    /// Source offset of `buffer[0]` and the number of valid bytes after it.
    start: u64,
    filled: usize,
    position: u64,
    inner_position: u64,
    /// End and size of the previous inner read; they classify the next one.
    last_end: Option<u64>,
    window: usize,
}

impl<R: Read + Seek> Window<R> {
    pub(crate) fn new(mut inner: R) -> io::Result<Self> {
        let position = inner.stream_position()?;
        Ok(Self {
            inner,
            buffer: Vec::new(),
            start: position,
            filled: 0,
            position,
            inner_position: position,
            last_end: None,
            window: MAX_WINDOW,
        })
    }

    fn buffered(&self) -> Option<usize> {
        let offset = self.position.checked_sub(self.start)?;
        (offset < self.filled as u64).then_some(offset as usize)
    }

    /// The first read and sequential continuations grow the window; jumps restart small.
    fn next_window(&self) -> usize {
        match self.last_end {
            None => MAX_WINDOW,
            Some(end) if self.position >= end && self.position - end <= self.window as u64 => {
                self.window.saturating_mul(2).min(MAX_WINDOW)
            }
            Some(_) => JUMP_WINDOW,
        }
    }

    fn inner_read(&mut self, size: usize, direct: Option<&mut [u8]>) -> io::Result<usize> {
        if self.inner_position != self.position {
            self.inner_position = self.inner.seek(SeekFrom::Start(self.position))?;
        }
        let count = match direct {
            Some(output) => self.inner.read(output)?,
            None => {
                if self.buffer.len() < size {
                    self.buffer.resize(size, 0);
                }
                let count = self.inner.read(&mut self.buffer[..size])?;
                self.start = self.position;
                self.filled = count;
                count
            }
        };
        let end = self
            .position
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        self.inner_position = end;
        self.last_end = Some(end);
        self.window = size;
        Ok(count)
    }
}

impl<R: Read + Seek> Read for Window<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.buffered().is_none() {
            let size = self.next_window();
            if output.len() >= size {
                // A large request reads exactly what it needs, like `BufReader`.
                let request = output.len().min(MAX_WINDOW);
                let count = self.inner_read(request, Some(&mut output[..request]))?;
                self.position += count as u64;
                return Ok(count);
            }
        }
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<R: Read + Seek> BufRead for Window<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let offset = match self.buffered() {
            Some(offset) => offset,
            None => {
                let size = self.next_window();
                self.inner_read(size, None)?;
                0
            }
        };
        Ok(&self.buffer[offset..self.filled])
    }

    fn consume(&mut self, amount: usize) {
        let available = self
            .buffered()
            .map_or(0, |offset| self.filled - offset)
            .min(amount);
        self.position += available as u64;
    }
}

impl<R: Read + Seek> Seek for Window<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(target) => Some(target),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(_) => None,
        };
        // Fetched bytes stay valid: the brokered source is immutable for a decode.
        if let Some(target) = target
            && target >= self.start
            && target - self.start <= self.filled as u64
        {
            self.position = target;
            return Ok(target);
        }
        // Elsewhere the source itself validates the position, as before.
        let position = match (from, target) {
            (SeekFrom::End(delta), _) => self.inner.seek(SeekFrom::End(delta))?,
            (_, Some(target)) => self.inner.seek(SeekFrom::Start(target))?,
            (_, None) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Seek before the start of the image source",
                ));
            }
        };
        self.position = position;
        self.inner_position = position;
        Ok(position)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Records each inner read like the host broker: offset and clamped size.
    struct Broker {
        data: Cursor<Vec<u8>>,
        requests: Vec<(u64, usize)>,
    }
    impl Read for Broker {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let length = output.len().min(MAX_WINDOW);
            self.requests.push((self.data.position(), length));
            self.data.read(&mut output[..length])
        }
    }
    impl Seek for Broker {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.data.seek(from)
        }
    }

    fn source(length: usize) -> (Broker, Vec<u8>) {
        let data: Vec<u8> = (0..length).map(|i| (i * 13 + i / 241) as u8).collect();
        (
            Broker {
                data: Cursor::new(data.clone()),
                requests: Vec::new(),
            },
            data,
        )
    }

    #[test]
    fn strip_seeks_inside_fetched_bytes_reuse_them() {
        let (mut broker, data) = source(2 << 20);
        let mut window = Window::new(&mut broker).unwrap();
        let mut strip = [0; 600];
        for offset in (8..200_000u64).step_by(600) {
            window.seek(SeekFrom::Start(offset)).unwrap();
            window.read_exact(&mut strip).unwrap();
            assert_eq!(strip, data[offset as usize..offset as usize + 600]);
        }
        let requested: usize = broker.requests.iter().map(|(_, size)| *size).sum();
        assert_eq!(requested, MAX_WINDOW);
    }

    #[test]
    fn scattered_strips_fetch_jump_windows_and_grow_when_sequential() {
        let (mut broker, data) = source(4 << 20);
        let mut window = Window::new(&mut broker).unwrap();
        let mut bytes = [0; 100];
        window.read_exact(&mut bytes).unwrap();
        for offset in [3u64 << 20, 1 << 20] {
            window.seek(SeekFrom::Start(offset)).unwrap();
            window.read_exact(&mut bytes).unwrap();
            assert_eq!(bytes, data[offset as usize..offset as usize + 100]);
        }
        let mut chunk = [0; 4096];
        for _ in 0..64 {
            window.read_exact(&mut chunk).unwrap();
        }
        let sizes: Vec<_> = broker.requests.iter().map(|(_, size)| *size).collect();
        assert_eq!(
            sizes[..4],
            [MAX_WINDOW, JUMP_WINDOW, JUMP_WINDOW, 2 * JUMP_WINDOW]
        );
        assert_eq!(sizes[4..6], [4 * JUMP_WINDOW, MAX_WINDOW]);
    }

    #[test]
    fn unbuffered_seeks_are_validated_by_the_source() {
        let (mut broker, _) = source(1000);
        let mut window = Window::new(&mut broker).unwrap();
        assert_eq!(window.seek(SeekFrom::End(-10)).unwrap(), 990);
        assert_eq!(window.stream_position().unwrap(), 990);
        assert!(window.seek(SeekFrom::Current(-991)).is_err());
        let mut bytes = Vec::new();
        window.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 10);
        assert_eq!(window.seek(SeekFrom::Current(-5)).unwrap(), 995);
        assert_eq!(broker.requests.len(), 2, "{:?}", broker.requests);
    }
}
