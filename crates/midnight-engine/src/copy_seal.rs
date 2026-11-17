//! Copy-local unwind boundaries. An armed owner is released only explicitly.
use std::ops::{Deref, DerefMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Quarantined;

pub(crate) fn checked<T>(operation: impl FnOnce() -> T) -> Result<T, Quarantined> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => Ok(value),
        Err(panic) => {
            // Even inspecting or destroying an opaque panic may invoke user code.
            std::mem::forget(panic);
            Err(Quarantined)
        }
    }
}

pub(crate) struct Armed<T>(Option<T>);
impl<T> Armed<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Some(value))
    }
    pub(crate) fn into_inner(mut self) -> T {
        self.0.take().expect("armed Copy value remains owned")
    }
}
impl<T> Deref for Armed<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0.as_ref().expect("armed Copy value remains owned")
    }
}
impl<T> DerefMut for Armed<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0.as_mut().expect("armed Copy value remains owned")
    }
}
impl<T> Drop for Armed<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::mem::forget(value);
        }
    }
}
