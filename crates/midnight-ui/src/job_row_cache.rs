//! Derived text only: job receipts remain owned by Presenter until explicit Clear.
use crate::OperationRow;
use midnight_core::{
    CopyFailure, CopyProgress, MetadataResult, NumberLocale, OperationId, OperationProgress,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, hash_map::Entry},
    sync::{Arc, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Copy(OperationId),
    Mutation(OperationId),
}
pub(crate) enum Snapshot<'a> {
    Copy(&'a Arc<CopyProgress>, Option<&'a Arc<CopyFailure>>),
    Mutation(&'a Arc<OperationProgress>, Option<&'a Arc<MetadataResult>>),
}
impl Snapshot<'_> {
    fn key(&self) -> Key {
        match self {
            Self::Copy(progress, _) => Key::Copy(progress.id),
            Self::Mutation(progress, _) => Key::Mutation(progress.id),
        }
    }
}
// Weak keeps the allocation identity alive against address reuse without becoming
// the final owner of a core payload retired to a backend reclamation worker.
// Arc::make_mut also disassociates Weak tokens, including optimistic UI updates.
enum Token {
    Copy(Weak<CopyProgress>, Option<Weak<CopyFailure>>),
    Mutation(Weak<OperationProgress>, Option<Weak<MetadataResult>>),
}
impl Token {
    fn new(snapshot: &Snapshot<'_>) -> Self {
        match snapshot {
            Snapshot::Copy(progress, failure) => {
                Self::Copy(Arc::downgrade(progress), failure.map(Arc::downgrade))
            }
            Snapshot::Mutation(progress, metadata) => {
                Self::Mutation(Arc::downgrade(progress), metadata.map(Arc::downgrade))
            }
        }
    }
    fn same(&self, snapshot: &Snapshot<'_>) -> bool {
        match (self, snapshot) {
            (Self::Copy(old, failure), Snapshot::Copy(current, current_failure)) => {
                same(old, current) && same_optional(failure, *current_failure)
            }
            (Self::Mutation(old, metadata), Snapshot::Mutation(current, current_metadata)) => {
                same(old, current) && same_optional(metadata, *current_metadata)
            }
            _ => false,
        }
    }
}
fn same<T>(old: &Weak<T>, current: &Arc<T>) -> bool {
    Weak::ptr_eq(old, &Arc::downgrade(current))
}
fn same_optional<T>(old: &Option<Weak<T>>, current: Option<&Arc<T>>) -> bool {
    match (old, current) {
        (None, None) => true,
        (Some(old), Some(current)) => same(old, current),
        _ => false,
    }
}
struct Cached {
    snapshot: Token,
    locale: NumberLocale,
    row: OperationRow,
}
#[derive(Default)]
pub(crate) struct Rows(RefCell<HashMap<Key, Cached>>);
impl Rows {
    pub(crate) fn retain(&self, jobs: impl Iterator<Item = Key>) {
        let retained: HashSet<_> = jobs.collect();
        self.0.borrow_mut().retain(|key, _| retained.contains(key));
    }
    pub(crate) fn get(
        &self,
        snapshot: Snapshot<'_>,
        locale: &NumberLocale,
        build: impl FnOnce() -> OperationRow,
    ) -> OperationRow {
        let mut cache = self.0.borrow_mut();
        let cached = match cache.entry(snapshot.key()) {
            Entry::Vacant(slot) => slot.insert(Cached {
                snapshot: Token::new(&snapshot),
                locale: locale.clone(),
                row: build(),
            }),
            Entry::Occupied(slot) => {
                let cached = slot.into_mut();
                if !cached.snapshot.same(&snapshot) || cached.locale != *locale {
                    *cached = Cached {
                        snapshot: Token::new(&snapshot),
                        locale: locale.clone(),
                        row: build(),
                    };
                }
                cached
            }
        };
        cached.row.clone()
    }
}

#[cfg(test)]
#[path = "job_row_cache_tests.rs"]
mod tests;
