use midnight_plugin_api::abi;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PoolError {
    Full,
    Unknown,
    InvalidState,
    Exhausted,
}

type BatchInsert<const N: usize> = Result<[u64; N], (PoolError, [Box<[u8]>; N])>;

struct Allocation {
    bytes: Box<[u8]>,
    leased: bool,
    terminal: bool,
}

pub(crate) struct BufferPool {
    allocations: HashMap<u64, Allocation>,
    ids: Arc<AtomicU64>,
    normal_bytes: usize,
    terminal_bytes: usize,
    normal_count: usize,
    terminal_count: usize,
    normal_limit: usize,
    terminal_limit: usize,
}

impl BufferPool {
    pub(crate) fn new(normal_limit: usize, terminal_limit: usize, ids: Arc<AtomicU64>) -> Self {
        Self {
            allocations: HashMap::new(),
            ids,
            normal_bytes: 0,
            terminal_bytes: 0,
            normal_count: 0,
            terminal_count: 0,
            normal_limit,
            terminal_limit,
        }
    }

    pub(crate) fn insert_batch<const N: usize>(
        &mut self,
        bytes: [Box<[u8]>; N],
        terminal: bool,
    ) -> BatchInsert<N> {
        let (used, limit, count, maximum) = if terminal {
            (
                self.terminal_bytes,
                self.terminal_limit,
                self.terminal_count,
                64,
            )
        } else {
            (self.normal_bytes, self.normal_limit, self.normal_count, 128)
        };
        if count.checked_add(N).is_none_or(|sum| sum > maximum)
            || bytes
                .iter()
                .any(|bytes| u32::try_from(bytes.len()).is_err())
        {
            return Err((PoolError::Full, bytes));
        }
        let Some(total) = bytes
            .iter()
            .try_fold(0usize, |sum, bytes| sum.checked_add(bytes.len()))
            .filter(|&total| used.checked_add(total).is_some_and(|sum| sum <= limit))
        else {
            return Err((PoolError::Full, bytes));
        };
        // This counter is shared by all instances in one loaded library. Relaxed
        // ordering provides uniqueness; allocation ownership stays behind &mut self.
        let Ok(first) = self
            .ids
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                if next == 0 {
                    None
                } else {
                    next.checked_add(N as u64)
                }
            })
        else {
            return Err((PoolError::Exhausted, bytes));
        };
        if terminal {
            self.terminal_bytes += total;
            self.terminal_count += N;
        } else {
            self.normal_bytes += total;
            self.normal_count += N;
        }
        let mut output = [0; N];
        for (index, bytes) in bytes.into_iter().enumerate() {
            let id = first + index as u64;
            self.allocations.insert(
                id,
                Allocation {
                    bytes,
                    leased: false,
                    terminal,
                },
            );
            output[index] = id;
        }
        Ok(output)
    }

    pub(crate) fn insert(
        &mut self,
        bytes: Box<[u8]>,
        terminal: bool,
    ) -> Result<u64, (PoolError, Box<[u8]>)> {
        match self.insert_batch([bytes], terminal) {
            Ok([id]) => Ok(id),
            Err((error, [bytes])) => Err((error, bytes)),
        }
    }

    pub(crate) fn lease(&mut self, id: u64) -> Result<abi::Buffer, PoolError> {
        self.lease_batch([id]).map(|[buffer]| buffer)
    }

    pub(crate) fn lease_batch<const N: usize>(
        &mut self,
        ids: [u64; N],
    ) -> Result<[abi::Buffer; N], PoolError> {
        for (index, id) in ids.iter().enumerate() {
            let allocation = self.allocations.get(id).ok_or(PoolError::Unknown)?;
            if allocation.leased || ids[..index].contains(id) {
                return Err(PoolError::InvalidState);
            }
        }
        let mut output = [abi::Buffer {
            id: 0,
            bytes: abi::ByteSlice {
                data: std::ptr::null(),
                len: 0,
                reserved: 0,
            },
        }; N];
        for (index, id) in ids.into_iter().enumerate() {
            // The validation pass and exclusive borrow guarantee all entries remain present.
            let allocation = self.allocations.get_mut(&id).ok_or(PoolError::Unknown)?;
            allocation.leased = true;
            output[index] = abi::Buffer {
                id,
                bytes: abi::ByteSlice {
                    data: allocation.bytes.as_ptr(),
                    len: allocation.bytes.len() as u32,
                    reserved: 0,
                },
            };
        }
        Ok(output)
    }

    pub(crate) fn replace_reserved(
        &mut self,
        id: u64,
        bytes: Box<[u8]>,
    ) -> Result<(), (PoolError, Box<[u8]>)> {
        let Some(allocation) = self.allocations.get_mut(&id) else {
            return Err((PoolError::Unknown, bytes));
        };
        if allocation.leased || !allocation.terminal {
            return Err((PoolError::InvalidState, bytes));
        }
        let Some(total) = self
            .terminal_bytes
            .checked_sub(allocation.bytes.len())
            .and_then(|remaining| remaining.checked_add(bytes.len()))
            .filter(|&total| total <= self.terminal_limit && u32::try_from(bytes.len()).is_ok())
        else {
            return Err((PoolError::Full, bytes));
        };
        self.terminal_bytes = total;
        allocation.bytes = bytes;
        Ok(())
    }
    pub(crate) fn release(&mut self, id: u64) -> Result<(), PoolError> {
        self.remove(id, true)
    }
    pub(crate) fn discard(&mut self, id: u64) -> Result<(), PoolError> {
        self.remove(id, false)
    }

    fn remove(&mut self, id: u64, leased: bool) -> Result<(), PoolError> {
        let allocation = self.allocations.get(&id).ok_or(PoolError::Unknown)?;
        if allocation.leased != leased {
            return Err(PoolError::InvalidState);
        }
        let allocation = self.allocations.remove(&id).ok_or(PoolError::Unknown)?;
        if allocation.terminal {
            self.terminal_bytes -= allocation.bytes.len();
            self.terminal_count -= 1;
        } else {
            self.normal_bytes -= allocation.bytes.len();
            self.normal_count -= 1;
        }
        Ok(())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.allocations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(normal: usize, terminal: usize) -> BufferPool {
        BufferPool::new(normal, terminal, Arc::new(AtomicU64::new(1)))
    }

    #[test]
    fn only_leased_buffers_can_be_released_and_ids_never_alias() {
        let mut pool = pool(16, 8);
        let first = pool
            .insert(vec![1, 2, 3].into_boxed_slice(), false)
            .unwrap();
        assert_eq!(pool.release(first), Err(PoolError::InvalidState));
        let leased = pool.lease(first).unwrap();
        assert_eq!(leased.bytes.len, 3);
        assert!(!leased.bytes.data.is_null());
        assert_eq!(pool.discard(first), Err(PoolError::InvalidState));
        assert!(matches!(pool.lease(first), Err(PoolError::InvalidState)));
        pool.release(first).unwrap();
        assert_eq!(pool.release(first), Err(PoolError::Unknown));
        let second = pool.insert(vec![4].into_boxed_slice(), false).unwrap();
        assert_ne!(first, second);
        pool.discard(second).unwrap();
        assert!(pool.is_empty());
    }

    #[test]
    fn normal_backpressure_preserves_input_and_terminal_reserve() {
        let mut pool = pool(4, 4);
        let first = pool.insert(vec![1; 4].into_boxed_slice(), false).unwrap();
        let (error, rejected) = pool
            .insert(vec![2; 3].into_boxed_slice(), false)
            .unwrap_err();
        assert_eq!(error, PoolError::Full);
        assert_eq!(&*rejected, &[2, 2, 2]);
        let terminal = pool.insert(vec![9; 4].into_boxed_slice(), true).unwrap();
        pool.lease(terminal).unwrap();
        pool.release(terminal).unwrap();
        pool.lease(first).unwrap();
        pool.release(first).unwrap();
        assert!(pool.is_empty());
        assert!(pool.insert(rejected, false).is_ok());
    }

    #[test]
    fn empty_allocations_and_handle_exhaustion_cannot_grow_without_bound() {
        let mut pool = pool(128, 64);
        for _ in 0..128 {
            pool.insert(Box::new([]), false).unwrap();
        }
        assert!(matches!(
            pool.insert(Box::new([]), false),
            Err((PoolError::Full, _))
        ));
        let mut pool = BufferPool::new(128, 64, Arc::new(AtomicU64::new(u64::MAX)));
        assert!(matches!(
            pool.insert(Box::new([]), false),
            Err((PoolError::Exhausted, _))
        ));
        assert!(pool.is_empty());
    }

    #[test]
    fn paired_output_allocation_is_atomic_and_preserves_rejected_payloads() {
        let mut pool = pool(4, 4);
        let (error, [first, second]) = pool
            .insert_batch(
                [vec![1; 3].into_boxed_slice(), vec![2; 3].into_boxed_slice()],
                false,
            )
            .unwrap_err();
        assert_eq!(error, PoolError::Full);
        assert_eq!(&*first, &[1, 1, 1]);
        assert_eq!(&*second, &[2, 2, 2]);
        assert!(pool.is_empty());
        let ids = pool
            .insert_batch(
                [vec![1; 2].into_boxed_slice(), vec![2; 2].into_boxed_slice()],
                false,
            )
            .unwrap();
        for id in ids {
            pool.lease(id).unwrap();
            pool.release(id).unwrap();
        }
        assert!(pool.is_empty());
    }

    #[test]
    fn buffer_handles_cannot_release_another_instances_live_allocation() {
        let ids = Arc::new(AtomicU64::new(1));
        let mut left = BufferPool::new(16, 8, ids.clone());
        let mut right = BufferPool::new(16, 8, ids);
        let left_id = left.insert(vec![1].into_boxed_slice(), false).unwrap();
        let right_id = right.insert(vec![2].into_boxed_slice(), false).unwrap();
        left.lease(left_id).unwrap();
        right.lease(right_id).unwrap();
        assert_ne!(left_id, right_id);
        assert_eq!(right.release(left_id), Err(PoolError::Unknown));
        right.release(right_id).unwrap();
        left.release(left_id).unwrap();
    }

    #[test]
    fn failed_batch_lease_changes_neither_buffer_state() {
        let mut pool = pool(16, 16);
        let ids = pool
            .insert_batch(
                [vec![1].into_boxed_slice(), vec![2].into_boxed_slice()],
                false,
            )
            .unwrap();
        assert!(pool.lease_batch([ids[0], u64::MAX]).is_err());
        assert!(pool.lease_batch([ids[0], ids[0]]).is_err());
        let leased = pool.lease_batch(ids).unwrap();
        assert_eq!(leased[0].id, ids[0]);
        assert_eq!(leased[1].id, ids[1]);
        for id in ids {
            pool.release(id).unwrap();
        }
    }

    #[test]
    fn accepted_request_can_finish_even_when_new_buffer_ids_are_exhausted() {
        let mut pool = pool(16, 16);
        let reserved = pool.insert(Box::new([]), true).unwrap();
        pool.ids.store(u64::MAX, Ordering::Relaxed);
        pool.replace_reserved(reserved, vec![7; 8].into_boxed_slice())
            .unwrap();
        assert_eq!(pool.lease(reserved).unwrap().bytes.len, 8);
        assert!(matches!(
            pool.replace_reserved(reserved, Box::new([])),
            Err((PoolError::InvalidState, _))
        ));
        pool.release(reserved).unwrap();
        assert!(pool.is_empty());
    }
}
