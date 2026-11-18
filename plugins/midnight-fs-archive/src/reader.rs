//! Regression tests for the brokered source reader.
use crate::storage::Reader;
use midnight_filesystem_api::MAX_READ;
use std::io::{self, BufRead, Read, Seek, SeekFrom};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io::Cursor, rc::Rc};

    #[derive(Default)]
    struct Observed {
        reads: Cell<usize>,
        largest_read: Cell<usize>,
        fail_read: Cell<Option<io::ErrorKind>>,
        fail_seek: Cell<bool>,
        fail_position: Cell<bool>,
    }

    struct Source {
        bytes: Cursor<Vec<u8>>,
        maximum_read: usize,
        observed: Rc<Observed>,
    }

    impl Read for Source {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            self.observed.reads.set(self.observed.reads.get() + 1);
            self.observed
                .largest_read
                .set(self.observed.largest_read.get().max(output.len()));
            if let Some(kind) = self.observed.fail_read.take() {
                return Err(kind.into());
            }
            let count = output.len().min(self.maximum_read);
            self.bytes.read(&mut output[..count])
        }
    }

    impl Seek for Source {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            if matches!(from, SeekFrom::Current(0)) {
                if self.observed.fail_position.get() {
                    return Err(io::ErrorKind::Unsupported.into());
                }
            } else if self.observed.fail_seek.replace(false) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            self.bytes.seek(from)
        }
    }

    fn reader(bytes: Vec<u8>, maximum_read: usize) -> (Reader, Rc<Observed>) {
        let observed = Rc::new(Observed::default());
        let length = bytes.len() as u64;
        let reader = Reader::new(
            Box::new(Source {
                bytes: Cursor::new(bytes),
                maximum_read,
                observed: observed.clone(),
            }),
            length,
        );
        (reader, observed)
    }

    #[test]
    fn in_window_seeks_keep_bytes_and_report_the_logical_cursor() {
        let (mut reader, observed) = reader(b"abcdefghij".to_vec(), usize::MAX);
        let mut bytes = [0; 3];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"abc");
        assert_eq!(reader.stream_position().unwrap(), 3);
        for from in [SeekFrom::Start(1), SeekFrom::Current(-2)] {
            assert_eq!(reader.seek(from).unwrap(), 1);
            reader.read_exact(&mut bytes[..2]).unwrap();
            assert_eq!(&bytes[..2], b"bc");
        }
        assert_eq!(reader.seek(SeekFrom::Current(0)).unwrap(), 3);
        assert_eq!(reader.seek(SeekFrom::Start(3)).unwrap(), 3);
        reader.seek_relative(1).unwrap();
        reader.read_exact(&mut bytes[..2]).unwrap();
        assert_eq!(&bytes[..2], b"ef");
        assert_eq!(reader.stream_position().unwrap(), 6);
        assert_eq!(observed.reads.get(), 1);
    }

    #[test]
    fn buffered_consumption_and_seek_share_the_same_cursor() {
        let (mut reader, observed) = reader(b"abcdef".to_vec(), usize::MAX);
        assert_eq!(reader.fill_buf().unwrap(), b"abcdef");
        reader.consume(4);
        assert_eq!(reader.stream_position().unwrap(), 4);
        assert_eq!(reader.seek(SeekFrom::Start(1)).unwrap(), 1);
        assert_eq!(reader.fill_buf().unwrap(), b"bcdef");
        reader.consume(2);
        let mut bytes = [0; 2];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"de");
        assert_eq!(reader.stream_position().unwrap(), 5);
        assert_eq!(observed.reads.get(), 1);
    }

    #[test]
    fn cache_misses_end_seeks_and_eof_keep_correct_positions() {
        let mut source = vec![b'a'; MAX_READ as usize];
        source.extend(vec![b'b'; MAX_READ as usize]);
        source.extend(vec![b'c'; MAX_READ as usize + 31]);
        let length = source.len() as u64;
        let (mut reader, observed) = reader(source, usize::MAX);
        let mut bytes = [0; 3];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"aaa");
        assert_eq!(
            reader
                .seek(SeekFrom::Start(u64::from(MAX_READ) + 7))
                .unwrap(),
            262151
        );
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"bbb");
        assert_eq!(reader.seek(SeekFrom::End(-3)).unwrap(), length - 3);
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"ccc");
        assert_eq!(reader.read(&mut bytes).unwrap(), 0);
        assert_eq!(reader.stream_position().unwrap(), length);
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), length);
        assert_eq!(
            reader.seek(SeekFrom::Start(length + 5)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.stream_position().unwrap(), length);
        assert_eq!(reader.read(&mut bytes).unwrap(), 0);
        reader.rewind().unwrap();
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"aaa");
        assert!(observed.largest_read.get() <= MAX_READ as usize);
    }

    #[test]
    fn short_reads_and_interruptions_preserve_bytes_and_cursor() {
        let (mut reader, observed) = reader(b"abcdefghijkl".to_vec(), 2);
        let mut first = [0; 6];
        reader.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"abcdef");
        assert_eq!(reader.stream_position().unwrap(), 6);
        assert_eq!(reader.seek(SeekFrom::Current(-1)).unwrap(), 5);
        let mut next = [0; 1];
        reader.read_exact(&mut next).unwrap();
        assert_eq!(&next, b"f");
        observed.fail_read.set(Some(io::ErrorKind::Interrupted));
        let mut rest = [0; 6];
        reader.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"ghijkl");
        assert_eq!(reader.stream_position().unwrap(), 12);
        assert_eq!(
            reader.read_exact(&mut next).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.stream_position().unwrap(), 12);
    }

    #[test]
    fn failed_refill_reports_only_consumed_bytes_and_can_retry() {
        let (mut reader, observed) = reader(b"abcdefgh".to_vec(), 2);
        let mut bytes = [0; 3];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"abc");
        observed.fail_read.set(Some(io::ErrorKind::Other));
        assert_eq!(
            reader.read_exact(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(reader.stream_position().unwrap(), 4);
        reader.read_exact(&mut bytes[..2]).unwrap();
        assert_eq!(&bytes[..2], b"ef");
        assert_eq!(reader.stream_position().unwrap(), 6);
    }

    #[test]
    fn failed_underlying_seek_keeps_buffered_bytes_available() {
        let mut source = b"abcd".to_vec();
        source.resize(MAX_READ as usize + 8, b'x');
        let (mut reader, observed) = reader(source, usize::MAX);
        let mut bytes = [0; 2];
        reader.read_exact(&mut bytes).unwrap();
        reader
            .seek(SeekFrom::Start(u64::from(MAX_READ) + 1))
            .unwrap();
        observed.fail_seek.set(true);
        assert_eq!(
            reader.read(&mut bytes).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(reader.stream_position().unwrap(), u64::from(MAX_READ) + 1);
        reader.seek(SeekFrom::Start(2)).unwrap();
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"cd");
        assert_eq!(observed.reads.get(), 1);
    }

    #[test]
    fn absolute_offsets_above_i64_and_relative_overflow_do_not_truncate() {
        let length = u64::MAX - 4;
        let mut reader = Reader::new(Box::new(Cursor::new(b"x".to_vec())), length);
        let offset = (1u64 << 63) + 17;
        assert_eq!(reader.seek(SeekFrom::Start(offset)).unwrap(), offset);
        assert_eq!(
            reader.seek(SeekFrom::Current(i64::MAX)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.stream_position().unwrap(), offset);
        assert_eq!(reader.seek(SeekFrom::Start(0)).unwrap(), 0);
        assert_eq!(
            reader.seek(SeekFrom::Current(-1)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.stream_position().unwrap(), 0);
        assert_eq!(
            reader.seek(SeekFrom::End(i64::MAX)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), length - 2);
        reader.rewind().unwrap();
        let mut bytes = [0; 1];
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"x");
    }

    #[test]
    fn unavailable_position_queries_still_allow_absolute_and_end_seeks() {
        let (mut reader, observed) = reader(b"abcdefghij".to_vec(), usize::MAX);
        let mut bytes = [0; 2];
        reader.read_exact(&mut bytes).unwrap();
        observed.fail_position.set(true);
        assert_eq!(reader.seek(SeekFrom::Start(1)).unwrap(), 1);
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"bc");
        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 8);
        reader.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"ij");
    }
}
