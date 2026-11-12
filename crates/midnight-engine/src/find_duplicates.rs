//! Bounded duplicate discovery: size pruning, SHA-256, then full byte comparison.
//! Native identities/revisions are checked on every reopen and after every pass.
use super::{CurrentDirectory, Work, entry_cost};
use midnight_core::*;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io, mem::size_of, path::Path, sync::Arc};

const READ_BYTES: usize = 64 * 1024;

struct Candidate {
    entry: FindEntry,
    digest: Option<[u8; 32]>,
}

#[derive(Default)]
pub(super) struct Candidates {
    entries: Vec<Candidate>,
    identities: HashSet<TransferIdentity>,
}

impl Candidates {
    pub(super) fn collect(&mut self, entry: &FindEntry, work: &mut Work<'_>) -> bool {
        if self.identities.contains(&entry.identity) {
            return true;
        }
        if entry.metadata.size.is_none() {
            work.error(find_error(
                &entry.path,
                io::ErrorKind::InvalidData,
                "Duplicate search requires a known regular-file length.",
            ));
            return true;
        }
        if self.entries.len() >= work.request.limits.max_duplicate_candidates {
            work.budget(FindBudget::DuplicateCandidates);
            return false;
        }
        // Charge vector capacity before growing it, plus owned path/text and
        // conservative hash-table capacity/control-byte overhead per identity.
        // Candidate snapshots never retain native handles.
        let capacity = self.entries.capacity();
        let growth = if self.entries.len() == capacity {
            capacity
                .max(4)
                .min(work.request.limits.max_duplicate_candidates - capacity)
        } else {
            0
        };
        let cost = growth
            .checked_mul(size_of::<Candidate>())
            .and_then(|cost| cost.checked_add(entry_cost(entry) - size_of::<FindEntry>()))
            .and_then(|cost| cost.checked_add(4 * size_of::<TransferIdentity>() + 64));
        if cost.is_none_or(|cost| !work.reserve(cost)) {
            work.budget(FindBudget::RetainedBytes);
            return false;
        }
        if self.entries.try_reserve_exact(growth).is_err()
            || self.identities.try_reserve(1).is_err()
        {
            work.error(find_error(
                &entry.path,
                io::ErrorKind::OutOfMemory,
                "Duplicate candidate allocation failed.",
            ));
            return false;
        }
        self.identities.insert(entry.identity);
        self.entries.push(Candidate {
            entry: entry.clone(),
            digest: None,
        });
        work.progress.duplicate_candidates += 1;
        true
    }

    pub(super) fn examine(
        mut self,
        service: &dyn FindService,
        work: &mut Work<'_>,
        location: &CurrentDirectory,
    ) {
        if self.entries.len() < 2 || work.stopped() {
            return;
        }
        // Sorting is in-place. Hash grouping needs at most four index arrays;
        // compare/hash buffers are shared across all candidates and passes.
        let scratch = self
            .entries
            .len()
            .checked_mul(6 * size_of::<usize>())
            .and_then(|size| size.checked_add(2 * READ_BYTES));
        if scratch.is_none_or(|size| !work.reserve(size)) {
            work.budget(FindBudget::RetainedBytes);
            return;
        }
        self.entries.sort_unstable_by(|a, b| {
            a.entry
                .metadata
                .size
                .cmp(&b.entry.metadata.size)
                .then_with(|| a.entry.path.cmp(&b.entry.path))
        });
        let mut first = 0;
        let mut left_buffer = vec![0; READ_BYTES];
        let mut right_buffer = vec![0; READ_BYTES];
        phase(work, FindPhase::HashingDuplicates);
        while first < self.entries.len() && !work.stopped() {
            let mut end = first + 1;
            while end < self.entries.len()
                && self.entries[end].entry.metadata.size == self.entries[first].entry.metadata.size
            {
                end += 1;
            }
            if end - first > 1 {
                for candidate in &mut self.entries[first..end] {
                    if work.stopped() {
                        return;
                    }
                    if !set_location(location, &candidate.entry.path, work) {
                        return;
                    }
                    match hash(service, &candidate.entry, &mut left_buffer, work) {
                        Ok(digest) => candidate.digest = Some(digest),
                        Err(issue) => report(issue, work),
                    }
                }
            }
            first = end;
        }
        if work.stopped() {
            return;
        }
        self.entries.sort_unstable_by(|a, b| {
            a.entry
                .metadata
                .size
                .cmp(&b.entry.metadata.size)
                .then_with(|| a.digest.cmp(&b.digest))
                .then_with(|| a.entry.path.cmp(&b.entry.path))
        });
        phase(work, FindPhase::ComparingDuplicates);
        first = 0;
        while first < self.entries.len() && !work.stopped() {
            let mut end = first + 1;
            while end < self.entries.len()
                && self.entries[end].entry.metadata.size == self.entries[first].entry.metadata.size
                && self.entries[end].digest == self.entries[first].digest
            {
                end += 1;
            }
            if end - first > 1
                && self.entries[first].digest.is_some()
                && !compare_bucket(
                    &self.entries[first..end],
                    service,
                    work,
                    location,
                    &mut left_buffer,
                    &mut right_buffer,
                )
            {
                return;
            }
            first = end;
        }
    }
}

fn compare_bucket(
    bucket: &[Candidate],
    service: &dyn FindService,
    work: &mut Work<'_>,
    location: &CurrentDirectory,
    left_buffer: &mut [u8],
    right_buffer: &mut [u8],
) -> bool {
    // SHA collisions must still split into actual equal-byte groups. On a read
    // or revision error, discard this entire bucket before publishing any of it:
    // an invalidated representative must not leave previously compared pairs.
    let mut representatives: Vec<usize> = Vec::new();
    let mut groups = vec![0usize; bucket.len()];
    for (index, candidate) in bucket.iter().enumerate() {
        if work.stopped() {
            return false;
        }
        if !set_location(location, &candidate.entry.path, work) {
            return false;
        }
        let mut group = None;
        for (group_index, &representative) in representatives.iter().enumerate() {
            match equal(
                service,
                &bucket[representative].entry,
                &candidate.entry,
                left_buffer,
                right_buffer,
                work,
            ) {
                Ok(true) => {
                    group = Some(group_index);
                    break;
                }
                Ok(false) => {}
                Err(issue) => {
                    report(issue, work);
                    return !work.stopped();
                }
            }
        }
        groups[index] = group.unwrap_or_else(|| {
            representatives.push(index);
            representatives.len() - 1
        });
    }
    // Recheck all members after the last comparison. These are live validated
    // reads, not a point-in-time filesystem snapshot or exclusion of writers.
    for candidate in bucket {
        if let Err(issue) = open(service, &candidate.entry, work) {
            report(issue, work);
            return !work.stopped();
        }
    }
    for group in 0..representatives.len() {
        if work.stopped() {
            return false;
        }
        let members: Vec<_> = groups
            .iter()
            .enumerate()
            .filter_map(|(index, &number)| (number == group).then_some(index))
            .collect();
        if members.len() > 1 && !publish_group(bucket, &members, work) {
            return false;
        }
    }
    true
}

fn publish_group(bucket: &[Candidate], members: &[usize], work: &mut Work<'_>) -> bool {
    let remaining = work
        .request
        .limits
        .max_results
        .saturating_sub(work.progress.results as usize);
    if members.len() > remaining {
        work.budget(FindBudget::Results);
        return false;
    }
    let bytes = members.iter().try_fold(0usize, |cost, &index| {
        cost.checked_add(entry_cost(&bucket[index].entry))
            .and_then(|cost| {
                cost.checked_add(size_of::<FindResult>() + 4 * size_of::<Arc<FindResult>>())
            })
    });
    if bytes.is_none_or(|bytes| !work.reserve(bytes)) {
        work.budget(FindBudget::RetainedBytes);
        return false;
    }
    let group = work.progress.duplicate_groups + 1;
    let batch: Arc<[_]> = members
        .iter()
        .enumerate()
        .map(|(index, &member)| {
            Arc::new(FindResult {
                id: FindResultId(work.progress.results + index as u64 + 1),
                entry: bucket[member].entry.clone(),
                duplicate_group: Some(group),
                duplicate_group_size: members.len(),
            })
        })
        .collect();
    // A duplicate group is one atomic bounded event (up to max_results), unlike
    // ordinary 64-row batches. Stop cannot expose only half of a large group.
    if work.stopped() || !(work.emit)(FindUpdate::Batch(batch)) {
        work.cancelled = true;
        return false;
    }
    work.progress.results += members.len() as u64;
    work.progress.duplicate_groups += 1;
    work.progress_if_due();
    true
}

enum Issue {
    Failed(FsError),
    Limited(FindBudget),
    Cancelled,
}

fn report(issue: Issue, work: &mut Work<'_>) {
    match issue {
        Issue::Failed(error) => work.error(error),
        Issue::Limited(budget) => {
            work.progress.limited_files += 1;
            work.budget(budget);
        }
        Issue::Cancelled => work.cancelled = true,
    }
}

fn open(
    service: &dyn FindService,
    entry: &FindEntry,
    work: &mut Work<'_>,
) -> Result<Box<dyn FindSource>, Issue> {
    if work.stopped() {
        return Err(Issue::Cancelled);
    }
    let source = service
        .open_find_source(&entry.path, work.cancel)
        .map_err(Issue::Failed)?;
    if !source.entry().matches_capture(entry) {
        return Err(Issue::Failed(find_error(
            &entry.path,
            io::ErrorKind::InvalidData,
            "Duplicate candidate identity or revision changed; search again.",
        )));
    }
    source.revalidate(work.cancel).map_err(Issue::Failed)?;
    Ok(source)
}

fn hash(
    service: &dyn FindService,
    entry: &FindEntry,
    buffer: &mut [u8],
    work: &mut Work<'_>,
) -> Result<[u8; 32], Issue> {
    let length = entry.metadata.size.unwrap_or(0);
    budget(length, work)?;
    let mut source = open(service, entry, work)?;
    let mut remaining = length;
    let mut digest = Sha256::new();
    while remaining != 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        read_full(source.as_mut(), &mut buffer[..count], work)?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    source.revalidate(work.cancel).map_err(Issue::Failed)?;
    if work.stopped() {
        return Err(Issue::Cancelled);
    }
    Ok(digest.finalize().into())
}

fn equal(
    service: &dyn FindService,
    left: &FindEntry,
    right: &FindEntry,
    left_buffer: &mut [u8],
    right_buffer: &mut [u8],
    work: &mut Work<'_>,
) -> Result<bool, Issue> {
    let length = left.metadata.size.unwrap_or(0);
    budget(length, work)?;
    let total = length
        .checked_mul(2)
        .ok_or(Issue::Limited(FindBudget::TotalContent))?;
    if total
        > work
            .request
            .limits
            .max_content_bytes
            .saturating_sub(work.progress.content_bytes)
    {
        return Err(Issue::Limited(FindBudget::TotalContent));
    }
    let mut a = open(service, left, work)?;
    let mut b = open(service, right, work)?;
    let mut remaining = length;
    let mut same = true;
    while remaining != 0 {
        let count = remaining
            .min(left_buffer.len() as u64)
            .min(right_buffer.len() as u64) as usize;
        read_full(a.as_mut(), &mut left_buffer[..count], work)?;
        read_full(b.as_mut(), &mut right_buffer[..count], work)?;
        if left_buffer[..count] != right_buffer[..count] {
            same = false;
            break;
        }
        remaining -= count as u64;
    }
    a.revalidate(work.cancel).map_err(Issue::Failed)?;
    b.revalidate(work.cancel).map_err(Issue::Failed)?;
    if work.stopped() {
        return Err(Issue::Cancelled);
    }
    Ok(same)
}

fn budget(length: u64, work: &Work<'_>) -> Result<(), Issue> {
    if length > work.request.limits.max_content_bytes_per_file as u64 {
        Err(Issue::Limited(FindBudget::FileContent))
    } else if length
        > work
            .request
            .limits
            .max_content_bytes
            .saturating_sub(work.progress.content_bytes)
    {
        Err(Issue::Limited(FindBudget::TotalContent))
    } else {
        Ok(())
    }
}

fn read_full(
    source: &mut dyn FindSource,
    mut buffer: &mut [u8],
    work: &mut Work<'_>,
) -> Result<(), Issue> {
    while !buffer.is_empty() {
        if work.stopped() {
            return Err(Issue::Cancelled);
        }
        let read = source
            .read_chunk(buffer, work.cancel)
            .map_err(Issue::Failed)?;
        if read == 0 || read > buffer.len() {
            return Err(Issue::Failed(find_error(
                &source.entry().path,
                io::ErrorKind::InvalidData,
                "Duplicate candidate ended early or returned an invalid read count.",
            )));
        }
        work.progress.content_bytes += read as u64;
        work.progress.duplicate_bytes += read as u64;
        buffer = &mut buffer[read..];
        work.progress_if_due();
    }
    Ok(())
}

fn phase(work: &mut Work<'_>, phase: FindPhase) {
    work.progress.phase = phase;
    if !(work.emit)(FindUpdate::Progress(work.progress.clone())) {
        work.cancelled = true;
    }
}

fn set_location(location: &CurrentDirectory, path: &Path, work: &mut Work<'_>) -> bool {
    if location.set(path) {
        true
    } else {
        work.budget(FindBudget::RetainedBytes);
        false
    }
}
