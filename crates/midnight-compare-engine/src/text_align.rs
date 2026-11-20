//! Line alignment of two texts within a bounded time.
//!
//! Lines are interned to `u32` ids first, so the deadline covers only the alignment. The
//! optimal Myers alignment runs first. When it gives up at `MYERS_BUDGET` (many scattered
//! changes in a large file), lines that occur exactly once on each side and keep their
//! order on both sides (the longest increasing subsequence of such pairs, as in patience
//! diff) anchor a second pass that runs Myers only between neighbouring anchors. A gap
//! Myers gives up on after `ANCHORED_BUDGET` becomes one changed block, as does a pair
//! without anchors. No pass ever pairs unequal lines as equal.
//!
//! Both passes collect runs of changed lines (`Hunk`), which `compact` then slides
//! through repeated lines to join their neighbours. Collecting and compacting take time
//! linear in the lines and stop once the comparison is cancelled.
use super::TextLines;
use crate::check_cancel;
use similar::{
    DiffOp,
    algorithms::{DiffHook, myers},
};
use std::{
    collections::HashMap,
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MYERS_BUDGET: Duration = Duration::from_millis(750);
const ANCHORED_BUDGET: Duration = Duration::from_millis(750);
/// Lines handled between cancellation checks in the linear passes.
const CHECK_EVERY: usize = 1 << 16;

pub(super) struct Alignment {
    pub ops: Vec<DiffOp>,
    /// Some gap was shown as a changed block because the time ran out.
    pub approximate: bool,
}

pub(super) fn align(lines: &[TextLines; 2], cancel: &AtomicBool) -> Result<Alignment, String> {
    let ids = intern(lines, cancel)?;
    align_ids(&ids, cancel, MYERS_BUDGET, ANCHORED_BUDGET)
}

/// Equal lines get equal ids, so comparisons during the alignment are integer compares.
fn intern(lines: &[TextLines; 2], cancel: &AtomicBool) -> Result<[Vec<u32>; 2], String> {
    let mut distinct: HashMap<&str, u32> = HashMap::with_capacity(lines[0].len());
    let mut ids = [
        Vec::with_capacity(lines[0].len()),
        Vec::with_capacity(lines[1].len()),
    ];
    for (side, lines) in lines.iter().enumerate() {
        for index in 0..lines.len() {
            if index.is_multiple_of(CHECK_EVERY) {
                check_cancel(cancel)?;
            }
            let next = distinct.len() as u32;
            ids[side].push(*distinct.entry(lines.line(index)).or_insert(next));
        }
    }
    Ok(ids)
}

pub(super) fn align_ids(
    ids: &[Vec<u32>; 2],
    cancel: &AtomicBool,
    myers_budget: Duration,
    anchored_budget: Duration,
) -> Result<Alignment, String> {
    let [old, new] = ids;
    let mut hook = Collector::new(cancel);
    myers::diff_deadline(
        &mut hook,
        &old[..],
        0..old.len(),
        &new[..],
        0..new.len(),
        Some(Instant::now() + myers_budget),
    )
    .map_err(Cancelled::message)?;
    // Whether Myers gave up decides, not the clock: an alignment it finished is kept.
    if !hook.expired {
        return finish(hook, ids, cancel);
    }
    drop(hook);
    let anchors = anchors(old, new, cancel)?;
    // Without anchors the whole pair is one gap, which Myers just failed to align.
    let deadline = Instant::now()
        + if anchors.is_empty() {
            Duration::ZERO
        } else {
            anchored_budget
        };
    let mut hook = Collector::new(cancel);
    let (mut old_at, mut new_at) = (0, 0);
    for (index, &(old_anchor, new_anchor)) in anchors.iter().enumerate() {
        if index.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        let (old_anchor, new_anchor) = (old_anchor as usize, new_anchor as usize);
        gap(
            &mut hook,
            old,
            old_at..old_anchor,
            new,
            new_at..new_anchor,
            deadline,
        )?;
        hook.equal(old_anchor, new_anchor, 1)
            .map_err(Cancelled::message)?;
        (old_at, new_at) = (old_anchor + 1, new_anchor + 1);
    }
    gap(
        &mut hook,
        old,
        old_at..old.len(),
        new,
        new_at..new.len(),
        deadline,
    )?;
    finish(hook, ids, cancel)
}

/// Myers between two anchors. After the deadline it only trims the common start and end
/// of the gap and shows the rest as changed.
fn gap(
    hook: &mut Collector<'_>,
    old: &[u32],
    old_range: Range<usize>,
    new: &[u32],
    new_range: Range<usize>,
    deadline: Instant,
) -> Result<(), String> {
    if old_range.is_empty() && new_range.is_empty() {
        return Ok(());
    }
    myers::diff_deadline(hook, old, old_range, new, new_range, Some(deadline))
        .map_err(Cancelled::message)
}

/// The collected alignment, compacted, as operations; approximate when Myers gave up on
/// some gap.
fn finish(
    hook: Collector<'_>,
    ids: &[Vec<u32>; 2],
    cancel: &AtomicBool,
) -> Result<Alignment, String> {
    let approximate = hook.expired;
    let mut hunks = hook.into_hunks();
    let ends = [ids[0].len(), ids[1].len()];
    compact(&mut hunks, ids, 2 * (ends[0] + ends[1]), cancel)?;
    Ok(Alignment {
        ops: operations(&hunks, ends, cancel)?,
        approximate,
    })
}

/// Pairs of positions of lines that occur exactly once on each side, in the longest
/// order that increases on both sides.
fn anchors(old: &[u32], new: &[u32], cancel: &AtomicBool) -> Result<Vec<(u32, u32)>, String> {
    let distinct = old.iter().chain(new).max().map_or(0, |&id| id as usize + 1);
    // Saturating occurrence counts (old, new) and the position of each id in `new`.
    let mut counts = vec![[0u8; 2]; distinct];
    let mut position = vec![0u32; distinct];
    for (index, &id) in new.iter().enumerate() {
        if index.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        let count = &mut counts[id as usize][1];
        *count = count.saturating_add(1);
        position[id as usize] = index as u32;
    }
    for (index, &id) in old.iter().enumerate() {
        if index.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        let count = &mut counts[id as usize][0];
        *count = count.saturating_add(1);
    }
    let pairs: Vec<(u32, u32)> = old
        .iter()
        .enumerate()
        .filter(|&(_, &id)| counts[id as usize] == [1, 1])
        .map(|(index, &id)| (index as u32, position[id as usize]))
        .collect();
    drop((counts, position));
    check_cancel(cancel)?;
    // Patience sorting: `tails[k]` is the pair ending the best increasing run of length
    // k + 1 found so far; `previous` links each pair to its predecessor in that run.
    let mut tails: Vec<u32> = Vec::new();
    let mut previous = vec![u32::MAX; pairs.len()];
    for (index, &(_, new_index)) in pairs.iter().enumerate() {
        if index.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        let slot = tails.partition_point(|&tail| pairs[tail as usize].1 < new_index);
        if let Some(&before) = slot.checked_sub(1).and_then(|before| tails.get(before)) {
            previous[index] = before;
        }
        if slot == tails.len() {
            tails.push(index as u32);
        } else {
            tails[slot] = index as u32;
        }
    }
    let mut anchors = Vec::with_capacity(tails.len());
    let mut cursor = tails.last().copied().unwrap_or(u32::MAX);
    while let Some(&pair) = pairs.get(cursor as usize) {
        anchors.push(pair);
        cursor = previous[cursor as usize];
    }
    anchors.reverse();
    Ok(anchors)
}

/// Consecutive changed lines: File A's lines `start[0]..end[0]` and File B's
/// `start[1]..end[1]`, either of which may be empty. The lines before, between and
/// after hunks are equal in pairs, as many on each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hunk {
    start: [usize; 2],
    end: [usize; 2],
}

impl Hunk {
    /// The only side with changed lines, for a deletion (0) or an insertion (1). Such a
    /// hunk can slide through equal lines; one that changes both sides never moves.
    fn one_side(&self) -> Option<usize> {
        match [self.start[0] < self.end[0], self.start[1] < self.end[1]] {
            [true, false] => Some(0),
            [false, true] => Some(1),
            _ => None,
        }
    }

    fn shift_up(&mut self, lines: usize) {
        self.start = self.start.map(|at| at - lines);
        self.end = self.end.map(|at| at - lines);
    }

    fn shift_down(&mut self, lines: usize) {
        self.start = self.start.map(|at| at + lines);
        self.end = self.end.map(|at| at + lines);
    }

    /// This hunk and `next`, which starts where this one ends, as one.
    fn join(self, next: Self) -> Self {
        debug_assert_eq!(self.end, next.start);
        Self {
            start: self.start,
            end: next.end,
        }
    }
}

/// Slides each hunk that only deletes or only inserts through the equal lines around it,
/// first up and then down as far as they repeat its lines, and joins it to a hunk it
/// reaches, so that a change Myers split reads as one block (what `similar`'s quadratic
/// `Compact` does for insertions). A slide passes an equal line only onto an equal line,
/// so equal pairs stay equal. Hunks move `budget` lines at most in all, which bounds
/// joined hunks that slide again; the hunks after that stay where they are.
fn compact(
    hunks: &mut Vec<Hunk>,
    ids: &[Vec<u32>; 2],
    mut budget: usize,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let ends = [ids[0].len(), ids[1].len()];
    // `hunks[..done]` are compacted and `hunks[next..]` wait. Joins only shorten the
    // list, so a hunk written back at `done` never overwrites one that waits.
    let (mut done, mut next, mut turns) = (0usize, 0, 0usize);
    while let Some(&taken) = hunks.get(next) {
        if turns.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        turns += 1;
        next += 1;
        let mut hunk = taken;
        // Up while the line before the hunk repeats its last line.
        while let Some(side) = hunk.one_side() {
            let lines = &ids[side];
            let (start, end) = (hunk.start[side], hunk.end[side]);
            let floor = done
                .checked_sub(1)
                .map_or(0, |previous| hunks[previous].end[side]);
            let room = start - floor;
            let moved = slide(room, &mut budget, cancel, |step| {
                lines[start - 1 - step] == lines[end - 1 - step]
            })?;
            hunk.shift_up(moved);
            if moved < room || done == 0 {
                break;
            }
            done -= 1;
            hunk = hunks[done].join(hunk);
        }
        // Down while the line after the hunk repeats its first line.
        while let Some(side) = hunk.one_side() {
            let lines = &ids[side];
            let (start, end) = (hunk.start[side], hunk.end[side]);
            let ceiling = hunks
                .get(next)
                .map_or(ends[side], |following| following.start[side]);
            let room = ceiling - end;
            let moved = slide(room, &mut budget, cancel, |step| {
                lines[start + step] == lines[end + step]
            })?;
            hunk.shift_down(moved);
            let Some(&following) = hunks.get(next).filter(|_| moved == room) else {
                break;
            };
            next += 1;
            hunk = hunk.join(following);
        }
        hunks[done] = hunk;
        done += 1;
    }
    hunks.truncate(done);
    Ok(())
}

/// The steps, at most `room`, that a hunk slides while `same(step)`: the equal line it
/// passes repeats the line it leaves. Each step spends one line of `budget`.
fn slide(
    room: usize,
    budget: &mut usize,
    cancel: &AtomicBool,
    same: impl Fn(usize) -> bool,
) -> Result<usize, String> {
    let mut step = 0;
    while step < room && *budget > 0 && same(step) {
        step += 1;
        *budget -= 1;
        if budget.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
    }
    Ok(step)
}

/// The hunks as operations: each one a deletion, an insertion or a replacement, with the
/// equal lines between them.
fn operations(
    hunks: &[Hunk],
    ends: [usize; 2],
    cancel: &AtomicBool,
) -> Result<Vec<DiffOp>, String> {
    let mut ops = Vec::with_capacity(2 * hunks.len() + 1);
    let mut at = [0; 2];
    for (index, hunk) in hunks.iter().enumerate() {
        if index.is_multiple_of(CHECK_EVERY) {
            check_cancel(cancel)?;
        }
        push_equal(&mut ops, at, hunk.start);
        let [old_index, new_index] = hunk.start;
        let (old_len, new_len) = (hunk.end[0] - old_index, hunk.end[1] - new_index);
        ops.push(match (old_len, new_len) {
            (_, 0) => DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            },
            (0, _) => DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            },
            _ => DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            },
        });
        at = hunk.end;
    }
    push_equal(&mut ops, at, ends);
    Ok(ops)
}

/// The equal lines from `from` to `to`, as many on each side.
fn push_equal(ops: &mut Vec<DiffOp>, from: [usize; 2], to: [usize; 2]) {
    let len = to[0] - from[0];
    debug_assert_eq!(len, to[1] - from[1]);
    if len > 0 {
        ops.push(DiffOp::Equal {
            old_index: from[0],
            new_index: from[1],
            len,
        });
    }
}

struct Cancelled;

impl Cancelled {
    fn message(self) -> String {
        "Comparison cancelled.".into()
    }
}

/// Collects the alignment as hunks of consecutive changes, notes whether Myers gave up
/// at its deadline, and stops it once the comparison is cancelled. Each operation takes
/// constant time; the equal runs are implied by the hunks.
struct Collector<'a> {
    cancel: &'a AtomicBool,
    hunks: Vec<Hunk>,
    /// The changes since the last equal run.
    open: Option<Hunk>,
    /// Where the last operation deleted, when it was a deletion.
    deleted_at: Option<usize>,
    /// Myers gave up on some range at its deadline.
    expired: bool,
}

impl<'a> Collector<'a> {
    fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            cancel,
            hunks: Vec::new(),
            open: None,
            deleted_at: None,
            expired: false,
        }
    }

    fn check(&self) -> Result<(), Cancelled> {
        if self.cancel.load(Ordering::Acquire) {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// `old_len` lines of File A at `old` and `new_len` of File B at `new` changed.
    fn change(&mut self, old: usize, old_len: usize, new: usize, new_len: usize) {
        debug_assert!(old_len + new_len > 0);
        let hunk = self.open.get_or_insert(Hunk {
            start: [old, new],
            end: [old, new],
        });
        hunk.end[0] += old_len;
        hunk.end[1] += new_len;
    }

    fn into_hunks(mut self) -> Vec<Hunk> {
        self.hunks.extend(self.open.take());
        self.hunks
    }
}

impl DiffHook for Collector<'_> {
    type Error = Cancelled;

    fn equal(&mut self, _: usize, _: usize, _: usize) -> Result<(), Cancelled> {
        self.check()?;
        self.deleted_at = None;
        self.hunks.extend(self.open.take());
        Ok(())
    }

    fn delete(&mut self, old: usize, old_len: usize, new: usize) -> Result<(), Cancelled> {
        self.check()?;
        self.change(old, old_len, new, 0);
        self.deleted_at = Some(old);
        Ok(())
    }

    fn insert(&mut self, old: usize, new: usize, new_len: usize) -> Result<(), Cancelled> {
        self.check()?;
        // `similar` 2.7's Myers gives up on a range at its deadline by deleting all of
        // it and inserting the other side at the same position. Every other insertion
        // that follows a deletion starts where the deletion ended.
        self.expired |= self.deleted_at == Some(old);
        self.deleted_at = None;
        self.change(old, 0, new, new_len);
        Ok(())
    }

    fn replace(
        &mut self,
        old: usize,
        old_len: usize,
        new: usize,
        new_len: usize,
    ) -> Result<(), Cancelled> {
        self.check()?;
        self.deleted_at = None;
        self.change(old, old_len, new, new_len);
        Ok(())
    }
}

#[cfg(test)]
#[path = "text_align_tests.rs"]
mod tests;
