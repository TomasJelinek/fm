//! Explicit, bounded syntax recovery. Foreground requests never enter this walk.
use crate::{
    checkpoints::Batch,
    decode, paging,
    plugin::{Session, lock},
    resume::Snapshot,
    styles::{RoleHint, Styles},
    syntax::{self, Language, State},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{
        abi,
        messages::{PrepareSource, ReadAt, SourcePrepared, Viewport},
    },
};
use std::sync::{Arc, atomic::Ordering};

const READ_BYTES: usize = 16 * 1024;
const MAX_REPLIES: usize = 64;
const TAIL_BYTES: usize = 256;
const SUFFIX_BYTES: usize = 8192;

#[cfg(test)]
std::thread_local! {
    static ANCHOR_COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn anchor_precedes(source: u64, target: u64) -> bool {
    #[cfg(test)]
    ANCHOR_COMPARISONS.with(|count| count.set(count.get() + 1));
    source <= target
}
// A prepared foreground read contains bounded lexical lookahead as well as the
// visible fragment. UTF-16 needs two raw bytes for each ASCII lookahead scalar.
pub(crate) const LOOKAHEAD: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    revision: u64,
    anchor: u64,
    backwards: bool,
    bytes: u32,
    lines: u32,
}
impl Key {
    fn new(revision: u64, viewport: &Viewport) -> Self {
        Self {
            revision,
            anchor: viewport.source_anchor,
            backwards: viewport.backwards,
            bytes: viewport.max_bytes,
            lines: viewport.max_lines,
        }
    }
}

struct Cursor {
    offset: u64,
    state: State,
    decoded: u64,
    line_start: bool,
    tail: [u8; TAIL_BYTES],
    tail_len: usize,
    eof: bool,
}
impl Cursor {
    fn new(snapshot: Snapshot) -> Self {
        Self {
            offset: snapshot.offset,
            state: snapshot.state,
            decoded: 0,
            line_start: snapshot.line_start,
            tail: [0; TAIL_BYTES],
            tail_len: 0,
            eof: false,
        }
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot::new(self.offset, self.state, self.line_start)
    }
}

struct Target {
    snapshot: Snapshot,
    origin: u64,
    measured: u64,
    measured_decoded: u64,
    selection: Selection,
    visible_start: Option<u64>,
    visible_end: Option<u64>,
    limit: Option<u64>,
    watch: Option<u64>,
    suffix: Option<Box<Suffix>>,
}

/// The bounded displayed suffix needed to measure a backward viewport's start.
/// Each scalar enters and leaves this UTF-8 ring once; no token is retained.
struct Suffix {
    bytes: [u8; SUFFIX_BYTES],
    head: usize,
    len: usize,
    newlines: usize,
}

impl Suffix {
    fn new() -> Self {
        Self {
            bytes: [0; SUFFIX_BYTES],
            head: 0,
            len: 0,
            newlines: 0,
        }
    }

    fn push(
        &mut self,
        ch: char,
        max_bytes: usize,
        max_lines: u32,
        cancelled: impl Fn() -> bool,
    ) -> bool {
        let limit = max_bytes.min(SUFFIX_BYTES);
        let width = ch.len_utf8();
        if width > limit || max_lines == 0 {
            self.head = 0;
            self.len = 0;
            self.newlines = 0;
            return true;
        }
        let mut removed = 0_usize;
        while self.len + width > limit {
            if removed.is_multiple_of(128) && cancelled() {
                return false;
            }
            self.pop();
            removed += 1;
        }
        let mut scalar = [0; 4];
        for &byte in ch.encode_utf8(&mut scalar).as_bytes() {
            self.bytes[(self.head + self.len) % SUFFIX_BYTES] = byte;
            self.len += 1;
        }
        self.newlines += usize::from(ch == '\n');
        // A terminal LF ends its preceding line instead of adding an empty one.
        let allowed = max_lines.saturating_sub(1) as usize + usize::from(ch == '\n');
        while self.newlines > allowed {
            if removed.is_multiple_of(128) && cancelled() {
                return false;
            }
            self.pop();
            removed += 1;
        }
        true
    }

    fn pop(&mut self) {
        let first = self.bytes[self.head];
        let width = match first {
            0..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        };
        self.head = (self.head + width) % SUFFIX_BYTES;
        self.len -= width;
        self.newlines -= usize::from(first == b'\n');
    }
}

#[derive(Clone, Copy)]
enum Selection {
    Forward {
        started: bool,
        bytes: u32,
        lines: u32,
    },
    Backward,
    Lookahead(u32),
}

/// The frontier and one reserved target are the two records omitted from the
/// ordinary checkpoint cache. A backward viewport retains only its bounded
/// displayed suffix until its exact selection range has been measured.
#[derive(Default)]
pub(crate) struct Index {
    key: Option<Key>,
    cursor: Option<Cursor>,
    target: Option<Target>,
    ready: bool,
    active: bool,
}
// Includes a conservative allowance for every sparse BTreeMap node and the
// bounded display name, recovery records/raw tail, and the backward suffix's
// complete boxed allocation plus conservative allocator overhead.
const _: () = assert!(
    size_of::<Index>()
        + size_of::<crate::checkpoints::Checkpoints>()
        + 128 * (size_of::<(u64, State)>() + 128)
        + 4096
        + size_of::<Suffix>()
        + 128
        <= 64 * 1024
);
impl Index {
    /// Take the actual owned state before positional I/O. Other foreground
    /// checkpoint insertions cannot evict this reservation or affect its copy.
    pub(crate) fn take_ready(&mut self, revision: u64, viewport: &Viewport) -> Option<Snapshot> {
        if !self.ready || self.key != Some(Key::new(revision, viewport)) {
            return None;
        }
        let target = self.target.take()?;
        self.ready = false;
        Some(target.snapshot)
    }
}

struct Lease {
    session: Arc<Session>,
    _work: Box<dyn FnOnce() + Send>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        lock(&self.session.preparation).active = false;
    }
}

struct Scan {
    session: Arc<Session>,
    key: Key,
    language: Language,
    lower: u64,
    cursor: Cursor,
    target: Option<Target>,
    remaining: usize,
    replies: usize,
    lease: Lease,
}

pub(crate) fn lower(viewport: &Viewport, bom: u64) -> u64 {
    let upper = viewport.source_anchor.max(bom);
    if viewport.backwards {
        upper
            .saturating_sub(u64::from(viewport.max_bytes.min(8192)) * 2 + 8)
            .max(bom)
    } else {
        upper
    }
}

pub(crate) fn end(viewport: &Viewport, length: Option<u64>, bom: u64) -> u64 {
    let upper = viewport.source_anchor.max(bom);
    let end = if viewport.backwards {
        upper.saturating_add(LOOKAHEAD)
    } else {
        upper.saturating_add(u64::from(viewport.max_bytes.min(8192)) * 4 + LOOKAHEAD)
    };
    length.map_or(end, |length| end.min(length.max(bom)))
}

pub(crate) fn start(
    context: Context,
    session: Arc<Session>,
    request: PrepareSource,
    work: Box<dyn FnOnce() + Send>,
) {
    if request.revision != session.revision
        || session
            .length
            .is_some_and(|length| request.viewport.source_anchor > length)
        || !(1..=262144).contains(&request.max_scan_bytes)
    {
        let _ = context.fail(abi::INVALID, "Invalid syntax preparation target");
        return;
    }
    let bom = lock(&session.metadata).bom;
    if session.length.is_some_and(|length| bom > length) {
        let _ = context.fail(abi::INVALID, "Source encoding exceeds its captured length");
        return;
    }
    let lower = lower(&request.viewport, bom);
    let Some((language, initial)) = paging::preparation_origin(&session, lower) else {
        let _ = context.fail(
            abi::UNSUPPORTED,
            "Syntax preparation is unavailable in this mode",
        );
        return;
    };
    let key = Key::new(session.revision, &request.viewport);
    let (cursor, target) = {
        let mut index = lock(&session.preparation);
        if index.active {
            let _ = context.fail(abi::QUEUE_FULL, "Syntax preparation is already active");
            return;
        }
        index.active = true;
        let continuing = index.key == Some(key);
        if !continuing {
            index.target = None;
            index.ready = false;
        }
        index.key = Some(key);
        let target = index.target.take();
        let cursor = index
            .cursor
            .take()
            .filter(|cursor| {
                (continuing && target.is_some())
                    || (initial.offset <= cursor.offset && cursor.offset <= lower)
            })
            .unwrap_or_else(|| Cursor::new(initial));
        (cursor, target)
    };
    let lease = Lease {
        session: session.clone(),
        _work: work,
    };
    let mut scan = Scan {
        session,
        key,
        language,
        lower,
        cursor,
        target,
        remaining: request.max_scan_bytes as usize,
        replies: 0,
        lease,
    };
    if scan.target.is_none() && scan.cursor.offset == scan.lower {
        scan.capture(&request.viewport, bom);
    }
    // A repeated Prepare for an already reserved target needs no I/O.
    let already_ready = lock(&scan.session.preparation).ready;
    if already_ready {
        finish(context, scan, true);
        return;
    }
    step(context, scan);
}

impl Scan {
    fn stopped(&self, context: &Context) -> bool {
        context.is_cancelled() || self.session.closed.load(Ordering::Acquire)
    }
    fn viewport(&self) -> Viewport {
        Viewport {
            source_anchor: self.key.anchor,
            backwards: self.key.backwards,
            max_bytes: self.key.bytes,
            max_lines: self.key.lines,
        }
    }
    fn capture(&mut self, viewport: &Viewport, _bom: u64) {
        self.target = Some(Target {
            snapshot: self.cursor.snapshot(),
            origin: self.cursor.decoded,
            measured: self.cursor.offset,
            measured_decoded: self.cursor.decoded,
            selection: if viewport.backwards {
                Selection::Backward
            } else {
                Selection::Forward {
                    started: false,
                    bytes: 0,
                    lines: 0,
                }
            },
            visible_start: viewport.backwards.then_some(self.cursor.decoded),
            visible_end: None,
            limit: None,
            watch: None,
            suffix: viewport.backwards.then(|| Box::new(Suffix::new())),
        });
    }
    fn readiness(&mut self) -> bool {
        let Some(target) = self.target.as_mut() else {
            return false;
        };
        if target.limit.is_none_or(|limit| self.cursor.decoded < limit) {
            return false;
        }
        if target.watch.is_some() {
            return target.snapshot.hints.iter().any(Option::is_some);
        }
        let Some((consumed, trailing)) = self.cursor.state.pending_span() else {
            return true;
        };
        let start = self.cursor.decoded.saturating_sub(consumed);
        let end = self.cursor.decoded.saturating_sub(trailing);
        let visible_start = target.visible_start.unwrap_or(target.origin);
        let visible_end = target.visible_end.unwrap_or(visible_start);
        if visible_start >= visible_end
            || consumed <= trailing
            || start >= visible_end
            || end <= visible_start
        {
            return true;
        }
        target.watch = Some(end.min(visible_end).saturating_sub(1).max(visible_start));
        false
    }
    fn resolution(&mut self, resolution: Option<RoleHint>) {
        if let (Some(target), Some(hint)) = (&mut self.target, resolution) {
            target.snapshot.hints[0] = Some(RoleHint {
                start: hint.start.saturating_sub(target.origin),
                end: hint.end.saturating_sub(target.origin),
                role: hint.role,
            });
        }
    }

    /// Measure the same visible end and lookahead used by foreground rendering.
    /// Counters and a bounded backward suffix survive a turn; received lexical
    /// tails are measured once.
    fn measure(&mut self, decoded: &decode::Decoded, eof: bool, context: &Context) -> bool {
        let Some(target) = self.target.as_mut() else {
            return true;
        };
        if target.limit.is_some() {
            return true;
        }
        let first = decoded
            .anchors
            .binary_search_by_key(&target.measured, |a| a.source_offset)
            .unwrap_or_else(|index| index);
        let text_start = decoded
            .anchors
            .get(first)
            .map_or(decoded.text.len(), |a| a.text_offset as usize);
        for (unit, (ch, anchors)) in decoded.text[text_start..]
            .chars()
            .zip(decoded.anchors[first..].windows(2))
            .enumerate()
        {
            if unit.is_multiple_of(128)
                && (context.is_cancelled() || self.session.closed.load(Ordering::Acquire))
            {
                return false;
            }
            if anchors[0].source_offset < target.measured {
                continue;
            }
            let width = ch.len_utf8() as u32;
            let mut selection = target.selection;
            match selection {
                Selection::Forward {
                    mut started,
                    bytes,
                    lines,
                } => {
                    started |= anchors[0].source_offset >= self.key.anchor;
                    if started {
                        target.visible_start.get_or_insert(target.measured_decoded);
                    }
                    selection = if started
                        && (bytes + width > self.key.bytes.min(8192) || lines >= self.key.lines)
                    {
                        target.visible_end = Some(target.measured_decoded);
                        Selection::Lookahead(0)
                    } else {
                        Selection::Forward {
                            started,
                            bytes,
                            lines,
                        }
                    };
                }
                Selection::Backward if anchors[1].source_offset > self.key.anchor => {
                    target.visible_end = Some(target.measured_decoded);
                    selection = Selection::Lookahead(0);
                }
                _ => {}
            }
            if let Selection::Lookahead(bytes) = selection
                && bytes + width > 128
            {
                target.limit = Some(target.measured_decoded);
                break;
            }
            target.measured = anchors[1].source_offset;
            let Some(position) = target.measured_decoded.checked_add(ch.len_utf8() as u64) else {
                return false;
            };
            target.measured_decoded = position;
            if matches!(selection, Selection::Backward)
                && let Some(suffix) = target.suffix.as_mut()
            {
                if !suffix.push(
                    ch,
                    self.key.bytes.min(8192) as usize,
                    self.key.lines,
                    || context.is_cancelled() || self.session.closed.load(Ordering::Acquire),
                ) {
                    return false;
                }
                target.visible_start = Some(position - suffix.len as u64);
            }
            target.selection = match selection {
                Selection::Forward {
                    started,
                    mut bytes,
                    mut lines,
                } => {
                    if started {
                        bytes += width;
                        lines += u32::from(ch == '\n');
                    }
                    if started && (bytes == self.key.bytes.min(8192) || lines == self.key.lines) {
                        target.visible_end = Some(position);
                        Selection::Lookahead(0)
                    } else {
                        Selection::Forward {
                            started,
                            bytes,
                            lines,
                        }
                    }
                }
                Selection::Backward if anchors[1].source_offset == self.key.anchor => {
                    target.visible_end = Some(position);
                    Selection::Lookahead(0)
                }
                Selection::Lookahead(bytes) => {
                    if bytes + width == 128 {
                        target.limit = Some(position);
                    }
                    Selection::Lookahead(bytes + width)
                }
                selection => selection,
            };
            if target.limit.is_some() {
                break;
            }
        }
        if eof && target.limit.is_none() {
            target.visible_start.get_or_insert(target.measured_decoded);
            target.visible_end.get_or_insert(target.measured_decoded);
            target.limit = Some(target.measured_decoded);
        }
        if target.visible_end.is_some() {
            target.suffix = None;
        }
        true
    }
}

fn step(context: Context, scan: Scan) {
    if scan.stopped(&context) {
        return;
    }
    if scan.remaining == 0 || scan.replies == MAX_REPLIES {
        finish(context, scan, false);
        return;
    }
    let Some(offset) = scan.cursor.offset.checked_add(scan.cursor.tail_len as u64) else {
        let _ = context.fail(abi::INVALID, "Syntax cursor exceeds source address range");
        return;
    };
    let mut length = scan
        .remaining
        .min(READ_BYTES)
        .min((u64::MAX - offset).min(READ_BYTES as u64) as usize) as u32;
    if let Some(captured) = scan.session.length {
        if offset > captured {
            let _ = context.fail(abi::INVALID, "Syntax cursor exceeds captured length");
            return;
        }
        if offset < captured {
            length = length.min((captured - offset).min(READ_BYTES as u64) as u32);
        }
    }
    if length == 0 || scan.cursor.eof {
        let _ = context.fail(abi::INVALID, "Syntax target could not be resolved at EOF");
        return;
    }
    let read = ReadAt {
        reader: scan.session.reader,
        revision: scan.session.revision,
        offset,
        length,
    };
    let result = context.read(
        read,
        Box::new(move |context, reply, content| {
            let mut scan = scan;
            if scan.stopped(&context) {
                return;
            }
            if reply.status != abi::OK {
                let _ = context.fail(reply.status, "Syntax preparation content read failed");
                return;
            }
            let bytes = content.as_ref();
            if bytes.len() > length as usize
                || reply.bytes as usize != bytes.len()
                || reply.offset != offset
                || reply.revision != scan.session.revision
                || scan
                    .session
                    .length
                    .is_some_and(|captured| bytes.len() as u64 > captured.saturating_sub(offset))
                || (bytes.is_empty() && !reply.eof)
            {
                let _ = context.fail(
                    abi::INVALID,
                    "Syntax preparation content read made no progress",
                );
                return;
            }
            scan.remaining -= bytes.len();
            scan.replies += 1;
            let mut raw = Vec::with_capacity(scan.cursor.tail_len + bytes.len());
            raw.extend_from_slice(&scan.cursor.tail[..scan.cursor.tail_len]);
            raw.extend_from_slice(bytes);
            drop(content);
            scan.cursor.eof = reply.eof;
            consume(context, scan, raw);
        }),
    );
    if let Err(status) = result {
        let _ = context.fail(status, "Content service rejected syntax preparation");
    }
}

fn consume(context: Context, mut scan: Scan, raw: Vec<u8>) {
    let encoding = lock(&scan.session.metadata).encoding;
    let bom = lock(&scan.session.metadata).bom;
    let Some(decoded) = decode::decode(
        &raw,
        scan.cursor.offset,
        encoding,
        scan.cursor.eof,
        65536,
        usize::MAX,
        || scan.stopped(&context),
    ) else {
        if !scan.stopped(&context) {
            let _ = context.fail(abi::INVALID, "Invalid syntax decoding state");
        }
        return;
    };
    let raw_start = scan.cursor.offset;
    // A callback's raw target does not move while preceding lines are lexed.
    // Locate its scalar boundary once, then reuse it for every complete line.
    let target_boundary = scan.target.is_none().then(|| {
        let upper = decoded
            .anchors
            .partition_point(|anchor| anchor_precedes(anchor.source_offset, scan.lower));
        upper
            .checked_sub(1)
            .map_or(0, |index| decoded.anchors[index].text_offset as usize)
    });
    let mut at = 0;
    let mut checkpoints = Batch::new();
    let mut extend = false;
    loop {
        if scan.stopped(&context) {
            return;
        }
        if scan.target.is_none() && scan.cursor.offset >= scan.lower {
            scan.capture(&scan.viewport(), bom);
        }
        if !scan.measure(
            &decoded,
            scan.cursor.eof && decoded.consumed == raw.len(),
            &context,
        ) {
            if !scan.stopped(&context) {
                let _ = context.fail(
                    abi::INVALID,
                    "Syntax measurement exceeds its coordinate budget",
                );
            }
            return;
        }
        if scan.readiness() {
            scan.cursor.tail_len = 0;
            commit_checkpoints(&context, &scan, &checkpoints);
            finish(context, scan, true);
            return;
        }
        if at == decoded.text.len() {
            break;
        }
        let mut end = decoded.text.len();
        if !extend
            && scan.target.is_none()
            && let Some(before) = target_boundary
        {
            if before <= at {
                if scan.target.is_none() {
                    scan.capture(&scan.viewport(), bom);
                }
                extend = true;
                continue;
            }
            end = before;
        }
        if let Some(limit) = scan
            .target
            .as_ref()
            .filter(|target| target.watch.is_none())
            .and_then(|target| target.limit)
        {
            end = end.min(at + limit.saturating_sub(scan.cursor.decoded) as usize);
        }
        // Complete lines are the only ordinary checkpoints. The final partial
        // line uses precisely the same streaming lexer as a huge line's cursor.
        if let Some(newline) = decoded.text[at..end].find('\n') {
            end = at + newline + 1;
        }
        let mut styles = Styles::new(0..0, false);
        let Ok(origin) = usize::try_from(scan.cursor.decoded) else {
            let _ = context.fail(abi::INVALID, "Syntax coordinates exceed address range");
            return;
        };
        styles.origin(origin);
        if let Some(point) = scan.target.as_ref().and_then(|target| target.watch) {
            styles.watch(point);
        }
        let final_chunk =
            scan.cursor.eof && end == decoded.text.len() && decoded.consumed == raw.len();
        let mut state = scan.cursor.state;
        let Some(consumed) = syntax::stream(
            &decoded.text[at..end],
            scan.language,
            &mut state,
            &mut styles,
            final_chunk,
            || scan.stopped(&context),
        ) else {
            if !scan.stopped(&context) {
                let _ = context.fail(
                    abi::INVALID,
                    "Syntax continuation exceeded its state budget",
                );
            }
            return;
        };
        scan.resolution(styles.take_resolution());
        if consumed == 0 {
            if scan.target.is_none()
                && scan.lower.saturating_sub(scan.cursor.offset) <= TAIL_BYTES as u64
            {
                scan.capture(&scan.viewport(), bom);
                continue;
            }
            if let Some(target) = scan.target.as_mut()
                && target.watch.is_none()
                && target.limit.is_some()
                && target
                    .visible_end
                    .is_some_and(|end| scan.cursor.decoded >= end)
            {
                target.limit = Some(scan.cursor.decoded);
                continue;
            }
            if end < decoded.text.len() {
                extend = true;
                continue;
            }
            break;
        }
        let Some(offset) = decoded.source_at(at + consumed) else {
            let _ = context.fail(abi::INVALID, "Syntax cursor omitted a scalar boundary");
            return;
        };
        let Some(decoded_offset) = scan.cursor.decoded.checked_add(consumed as u64) else {
            let _ = context.fail(abi::INVALID, "Syntax coordinates exceed u64 range");
            return;
        };
        scan.cursor.state = state;
        scan.cursor.offset = offset;
        scan.cursor.decoded = decoded_offset;
        scan.cursor.line_start = decoded.text[..at + consumed].ends_with('\n');
        if consumed == end - at && scan.cursor.line_start {
            checkpoints.push((offset, state));
        }
        at += consumed;
        if consumed < end - (at - consumed) {
            if scan.target.is_none()
                && scan.lower.saturating_sub(scan.cursor.offset) <= TAIL_BYTES as u64
            {
                scan.capture(&scan.viewport(), bom);
            }
            if let Some(target) = scan.target.as_mut()
                && target.watch.is_none()
                && target.limit.is_some()
                && target
                    .visible_end
                    .is_some_and(|end| scan.cursor.decoded >= end)
            {
                target.limit = Some(scan.cursor.decoded);
                continue;
            }
            if end < decoded.text.len() {
                extend = true;
            } else {
                break;
            }
        } else {
            extend = false;
        }
    }
    if scan.stopped(&context) {
        return;
    }
    // The only unconsumed material is a bounded ambiguous delimiter/keyword or
    // incomplete decoder unit. Preserve received bytes, advancing the next I/O
    // offset past them even for one-byte service replies.
    let used = (scan.cursor.offset - raw_start) as usize;
    let tail = &raw[used..];
    if tail.len() > TAIL_BYTES {
        let _ = context.fail(
            abi::INVALID,
            "Syntax continuation tail exceeded its bounded budget",
        );
        return;
    }
    scan.cursor.tail[..tail.len()].copy_from_slice(tail);
    scan.cursor.tail_len = tail.len();
    commit_checkpoints(&context, &scan, &checkpoints);
    if scan.cursor.eof {
        if scan.target.is_none() && scan.cursor.offset >= scan.lower {
            scan.capture(&scan.viewport(), bom);
        }
        // Even an empty EOF read must flush deferred entity/key/opener roles.
        let mut styles = Styles::new(0..0, false);
        styles.origin(scan.cursor.decoded as usize);
        if let Some(point) = scan.target.as_ref().and_then(|target| target.watch) {
            styles.watch(point);
        }
        let flushed = syntax::stream(
            "",
            scan.language,
            &mut scan.cursor.state,
            &mut styles,
            true,
            || context.is_cancelled() || scan.session.closed.load(Ordering::Acquire),
        );
        if flushed.is_none() || scan.cursor.tail_len != 0 || scan.cursor.offset < scan.lower {
            let _ = context.fail(abi::INVALID, "Syntax target could not be resolved at EOF");
            return;
        }
        scan.resolution(styles.take_resolution());
        if scan.readiness() {
            finish(context, scan, true);
        } else {
            let _ = context.fail(abi::INVALID, "Syntax role could not be resolved at EOF");
        }
    } else {
        // Context::read gives back the worker and resumes on a fresh callback.
        // No session/index mutex or synchronous gather spans these turns.
        step(context, scan);
    }
}

fn commit_checkpoints(context: &Context, scan: &Scan, checkpoints: &Batch) {
    if !scan.stopped(context) {
        paging::commit_checkpoints(&scan.session, checkpoints);
    }
}

fn finish(context: Context, scan: Scan, ready: bool) {
    if scan.stopped(&context) {
        return;
    }
    if ready
        && scan
            .target
            .as_ref()
            .is_none_or(|target| target.snapshot.offset > scan.lower)
    {
        let _ = context.fail(abi::INVALID, "Prepared syntax starts after its target");
        return;
    }
    let result = SourcePrepared {
        revision: scan.session.revision,
        frontier: scan.cursor.offset,
        ready,
    };
    let Scan {
        session,
        cursor,
        target,
        lease,
        ..
    } = scan;
    {
        let mut index = lock(&session.preparation);
        if context.is_cancelled() || session.closed.load(Ordering::Acquire) {
            return;
        }
        index.cursor = Some(cursor);
        index.target = target;
        index.ready = ready;
    }
    let Ok(output) = Output::source_prepared(&result) else {
        let _ = context.fail(abi::INVALID, "Invalid syntax preparation result");
        return;
    };
    let result = context.send(
        output,
        Box::new(move |context| {
            let _lease = lease;
            if !context.is_cancelled() && !session.closed.load(Ordering::Acquire) {
                let _ = context.complete();
            }
        }),
    );
    if let Err(error) = result {
        let _ = context.fail(error.status, "Output queue rejected syntax preparation");
    }
}

#[cfg(test)]
#[path = "preparation_tests.rs"]
mod tests;
