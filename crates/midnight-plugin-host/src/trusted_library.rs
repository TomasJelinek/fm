//! Shared native module ownership for independently versioned plugin interfaces.
use crate::{Error, native::Library};
use std::{
    ffi::{CStr, c_void},
    path::Path,
    ptr::NonNull,
};

/// A permission-checked native module. Loading, lookup and final drop are backend work.
/// This is a trusted-code boundary, not a sandbox for malicious native libraries.
pub struct TrustedLibrary(Library);

impl std::fmt::Debug for TrustedLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrustedLibrary")
            .finish_non_exhaustive()
    }
}

impl TrustedLibrary {
    /// Validate the complete package before loading its directly contained library.
    /// Paths must already be canonical; redirected dependencies are refused.
    pub fn open(package: &Path, library: &Path) -> Result<Self, Error> {
        if library.parent() != Some(package) || !package.is_absolute() {
            return Err(Error("Library must be directly inside its package".into()));
        }
        crate::validate_plugin_path(package)?;
        crate::validate_plugin_path(library)?;
        crate::package_trust::check(package)?;
        Ok(Self(Library::open(library)?))
    }

    /// Find an export defined by this module, rejecting dependency/forwarded exports.
    /// The pointer is valid only while this library is retained. Calling it requires
    /// the caller to establish the signature and the independently versioned ABI.
    pub fn symbol(&self, name: &CStr) -> Result<NonNull<c_void>, Error> {
        let symbol = self.0.symbol(name)?;
        if !self.owns_address(symbol.as_ptr()) {
            return Err(Error("Plugin export belongs to a different module".into()));
        }
        Ok(symbol)
    }

    /// Check callback provenance without dereferencing the supplied pointer.
    pub fn owns_address(&self, address: *const c_void) -> bool {
        !address.is_null() && self.0.owns_address(address)
    }
}
