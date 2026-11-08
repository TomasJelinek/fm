//! Optional coordinates use their own bounded read chain, never a viewport barrier.
use crate::{
    decode,
    plugin::{self, Session, lock},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{abi, messages::Locate, viewer::TextPosition},
};
use std::{collections::BTreeMap, sync::Arc};

const READ_BYTES: u64 = 32768;
const MAX_CHECKPOINTS: usize = 256;

#[derive(Clone, Copy)]
struct Coordinate {
    line: u64,
    column: u64,
}

pub(crate) struct Index {
    checkpoints: BTreeMap<u64, Coordinate>,
    spacing: u64,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            checkpoints: BTreeMap::new(),
            spacing: READ_BYTES,
        }
    }
}

impl Index {
    fn before(&self, target: u64, bom: u64) -> (u64, Coordinate) {
        self.checkpoints.range(..=target).next_back().map_or(
            (bom, Coordinate { line: 1, column: 1 }),
            |(&offset, &position)| (offset, position),
        )
    }

    fn record(&mut self, offset: u64, position: Coordinate) {
        if self
            .checkpoints
            .range(..=offset)
            .next_back()
            .is_some_and(|(&previous, _)| offset - previous < self.spacing)
        {
            return;
        }
        if self.checkpoints.len() >= MAX_CHECKPOINTS {
            // Coarsen the existing index uniformly instead of losing all early
            // anchors. It stays bounded even while scanning a multi-TiB file.
            self.spacing = self.spacing.saturating_mul(2);
            let mut keep = false;
            self.checkpoints.retain(|_, _| {
                keep = !keep;
                keep
            });
        }
        self.checkpoints.insert(offset, position);
    }
}

struct Scan {
    session: Arc<Session>,
    target: u64,
    offset: u64,
    position: Coordinate,
    _permit: Box<dyn FnOnce() + Send>,
}

pub(crate) fn start(
    context: Context,
    session: Arc<Session>,
    request: Locate,
    permit: Box<dyn FnOnce() + Send>,
) {
    if session
        .length
        .is_some_and(|length| request.source_anchor > length)
    {
        let _ = context.fail(abi::INVALID, "Position is outside the captured source");
        return;
    }
    let bom = lock(&session.metadata).bom;
    let target = request.source_anchor.max(bom);
    let (offset, position) = lock(&session.positions).before(target, bom);
    step(
        context,
        Scan {
            session,
            target,
            offset,
            position,
            _permit: permit,
        },
    );
}

fn step(context: Context, mut scan: Scan) {
    if context.is_cancelled() {
        return;
    }
    if scan.offset >= scan.target {
        finish(context, &scan);
        return;
    }
    // Four bytes beyond a target disambiguate UTF-8, surrogate pairs and CRLF.
    let count = scan
        .target
        .saturating_sub(scan.offset)
        .saturating_add(4)
        .min(READ_BYTES)
        .min(u64::MAX - scan.offset) as u32;
    if count == 0 {
        let _ = context.fail(abi::INVALID, "Position exceeds source address range");
        return;
    }
    plugin::read(
        context,
        scan.session.clone(),
        scan.offset,
        count,
        Box::new(move |context, data, eof| {
            let encoding = lock(&scan.session.metadata).encoding;
            let Some(decoded) =
                decode::decode(&data, scan.offset, encoding, eof, 65536, usize::MAX, || {
                    context.is_cancelled()
                })
            else {
                return;
            };
            for (index, ch) in decoded.text.chars().enumerate() {
                if index.is_multiple_of(128) && context.is_cancelled() {
                    return;
                }
                let next = if matches!(ch, '\n' | '\u{2028}' | '\u{2029}') {
                    scan.position
                        .line
                        .checked_add(1)
                        .map(|line| Coordinate { line, column: 1 })
                } else {
                    scan.position
                        .column
                        .checked_add(1)
                        .map(|column| Coordinate {
                            line: scan.position.line,
                            column,
                        })
                };
                let Some(position) = next else {
                    let _ = context.fail(abi::INVALID, "Logical coordinates exceed u64 range");
                    return;
                };
                scan.position = position;
                // The decoder records exactly one boundary for each displayed
                // scalar, including replacements and normalized CRLF.
                let Some(anchor) = decoded.anchors.get(index + 1) else {
                    let _ = context.fail(abi::INVALID, "Position decoder omitted a boundary");
                    return;
                };
                scan.offset = anchor.source_offset;
                if scan.offset >= scan.target {
                    lock(&scan.session.positions).record(scan.offset, scan.position);
                    finish(context, &scan);
                    return;
                }
            }
            if decoded.consumed == 0 || (eof && decoded.consumed == data.len()) {
                let _ = context.fail(abi::INVALID, "Position was not reached in captured source");
                return;
            }
            lock(&scan.session.positions).record(scan.offset, scan.position);
            // read() yields the worker permit before the next bounded chunk;
            // viewport/search continuations can run between these steps.
            step(context, scan);
        }),
    );
}

fn finish(context: Context, scan: &Scan) {
    if !context.is_cancelled() {
        plugin::send(
            context,
            Output::text_position(&TextPosition {
                revision: scan.session.revision,
                source_offset: scan.offset,
                line: scan.position.line,
                column: scan.position.column,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_remains_bounded_and_retains_exact_large_offsets_when_compacted() {
        let mut index = Index::default();
        for number in 1..=100_000_u64 {
            let offset = number * READ_BYTES;
            index.record(
                offset,
                Coordinate {
                    line: number + 1,
                    column: 7,
                },
            );
            assert!(index.checkpoints.len() <= MAX_CHECKPOINTS);
        }
        for (&offset, coordinate) in &index.checkpoints {
            assert_eq!(coordinate.line, offset / READ_BYTES + 1);
            assert_eq!(coordinate.column, 7);
        }
        let (offset, _) = index.before(1 << 40, 3);
        assert!(offset > 3_000_000_000);
        assert_eq!(index.before(0, 3).0, 3);
    }
}
