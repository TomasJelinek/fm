use super::*;
use std::{
    cell::{Cell, RefCell},
    io::{Cursor, Read, Seek, SeekFrom},
    rc::Rc,
};

/// Offset and size of every inner read request.
type Requests = Rc<RefCell<Vec<(u64, usize)>>>;

/// Records every inner read request, clamped like the host broker.
struct Broker {
    data: Cursor<Vec<u8>>,
    requests: Requests,
}
impl Read for Broker {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(MAX_WINDOW);
        self.requests
            .borrow_mut()
            .push((self.data.position(), length));
        self.data.read(&mut output[..length])
    }
}
impl Seek for Broker {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.data.seek(from)
    }
}

fn window(length: usize) -> (Window, Requests, Vec<u8>) {
    let data: Vec<u8> = (0..length).map(|i| (i * 7 + i / 251) as u8).collect();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let broker = Broker {
        data: Cursor::new(data.clone()),
        requests: requests.clone(),
    };
    (Window::new(Box::new(broker), length as u64), requests, data)
}

#[test]
fn seeks_inside_fetched_bytes_do_not_refetch() {
    let (mut window, requests, data) = window(1 << 20);
    let mut bytes = [0; 512];
    for offset in [0u64, 4096, 512, 100_000, 0, 200_000] {
        window.seek(SeekFrom::Start(offset)).unwrap();
        window.read_exact(&mut bytes).unwrap();
        assert_eq!(bytes, data[offset as usize..offset as usize + 512]);
    }
    // One initial maximum window serves every probe inside the first 256 KiB.
    assert_eq!(*requests.borrow(), [(0, MAX_WINDOW)]);
    window.seek(SeekFrom::Current(-512)).unwrap();
    assert_eq!(window.stream_position().unwrap(), 200_000);
    assert_eq!(requests.borrow().len(), 1);
}

#[test]
fn jumps_start_small_and_sequential_reads_grow_to_the_bounded_maximum() {
    let (mut window, requests, data) = window(4 << 20);
    let mut byte = [0];
    window.seek(SeekFrom::Start(3 << 20)).unwrap();
    window.read_exact(&mut byte).unwrap();
    window.seek(SeekFrom::Start(1 << 20)).unwrap();
    let mut chunk = [0; 4096];
    for offset in (1 << 20..2 << 20).step_by(chunk.len()) {
        window.read_exact(&mut chunk).unwrap();
        assert_eq!(chunk, data[offset..offset + chunk.len()]);
    }
    let sizes: Vec<_> = requests.borrow().iter().map(|(_, size)| *size).collect();
    assert_eq!(sizes[..2], [MAX_WINDOW, JUMP_WINDOW]);
    assert_eq!(
        sizes[2..6],
        [
            2 * JUMP_WINDOW,
            4 * JUMP_WINDOW,
            8 * JUMP_WINDOW,
            MAX_WINDOW
        ]
    );
    let requested: usize = sizes[1..].iter().sum();
    assert!(requested < (1 << 20) + MAX_WINDOW, "{requested}");
}

#[test]
fn large_reads_bypass_the_window_and_scattered_headers_stay_bounded() {
    let (mut window, requests, data) = window(8 << 20);
    let mut large = vec![0; MAX_WINDOW];
    window.read_exact(&mut large[..1]).unwrap();
    window.seek(SeekFrom::Start(1 << 20)).unwrap();
    window.read_exact(&mut large).unwrap();
    assert_eq!(large, data[1 << 20..(1 << 20) + MAX_WINDOW]);
    assert_eq!(requests.borrow()[1], (1 << 20, MAX_WINDOW));
    requests.borrow_mut().clear();
    // One small header per 512 KiB member transfers one jump window each.
    let mut header = [0; 30];
    for member in 4..16u64 {
        window.seek(SeekFrom::Start(member << 19)).unwrap();
        window.read_exact(&mut header).unwrap();
        let start = (member << 19) as usize;
        assert_eq!(header, data[start..start + 30]);
    }
    let requested: usize = requests.borrow().iter().map(|(_, size)| *size).sum();
    assert_eq!(requested, 12 * JUMP_WINDOW);
}

#[test]
fn seeks_are_checked_against_the_immutable_source_length() {
    let (mut window, requests, _) = window(1000);
    assert_eq!(window.seek(SeekFrom::End(0)).unwrap(), 1000);
    assert_eq!(window.read(&mut [0; 8]).unwrap(), 0);
    for from in [
        SeekFrom::Start(1001),
        SeekFrom::End(1),
        SeekFrom::Current(1),
        SeekFrom::Current(-1001),
    ] {
        assert_eq!(
            window.seek(from).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    assert_eq!(window.stream_position().unwrap(), 1000);
    assert!(requests.borrow().iter().all(|(offset, _)| *offset == 1000));
}

/// Fails one read after scribbling over the caller's buffer and moving the source.
struct Flaky {
    data: Cursor<Vec<u8>>,
    fail: Rc<Cell<bool>>,
}
impl Read for Flaky {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.fail.replace(false) {
            output.fill(0xee);
            self.data.seek(SeekFrom::Current(output.len() as i64 / 2))?;
            return Err(io::Error::other("fixture read failure"));
        }
        self.data.read(output)
    }
}
impl Seek for Flaky {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.data.seek(from)
    }
}

#[test]
fn a_failed_refill_never_serves_overwritten_or_misplaced_bytes() {
    let data: Vec<u8> = (0..1 << 20).map(|i| (i * 7 + i / 251) as u8).collect();
    let fail = Rc::new(Cell::new(false));
    let flaky = Flaky {
        data: Cursor::new(data.clone()),
        fail: fail.clone(),
    };
    let mut window = Window::new(Box::new(flaky), data.len() as u64);
    let mut bytes = [0; 64];
    window.read_exact(&mut bytes).unwrap();
    // A refill elsewhere fails after writing into the buffer and moving the source.
    window.seek(SeekFrom::Start(600_000)).unwrap();
    fail.set(true);
    assert!(window.read(&mut bytes).is_err());
    // The first window's bytes were overwritten, so they are fetched again.
    window.seek(SeekFrom::Start(100)).unwrap();
    window.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, data[100..164]);
    // After a failure the source position is unknown, so the retry seeks again.
    window.seek(SeekFrom::Start(600_000)).unwrap();
    fail.set(true);
    assert!(window.read(&mut bytes).is_err());
    window.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, data[600_000..600_064]);
}

#[test]
fn interleaved_reads_and_seeks_match_the_source() {
    let (mut window, _, data) = window(700_000);
    let mut state = 0x2545_f491u32;
    let mut next = |bound: usize| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as usize % bound
    };
    let mut position = 0usize;
    for _ in 0..2000 {
        if next(3) == 0 {
            position = next(data.len() + 1);
            window.seek(SeekFrom::Start(position as u64)).unwrap();
        }
        let mut bytes = vec![0; next(300_000)];
        let count = if next(2) == 0 {
            window.read(&mut bytes).unwrap()
        } else {
            let available = window.fill_buf().unwrap();
            let count = available.len().min(bytes.len());
            bytes[..count].copy_from_slice(&available[..count]);
            window.consume(count);
            count
        };
        assert_eq!(bytes[..count], data[position..position + count]);
        assert!(count > 0 || bytes.is_empty() || position == data.len());
        position += count;
        assert_eq!(window.stream_position().unwrap(), position as u64);
    }
}
