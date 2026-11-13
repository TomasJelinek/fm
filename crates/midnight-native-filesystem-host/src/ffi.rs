//! Audited synchronous C boundary; no Rust allocation or trait crosses the ABI.
use crate::{Cancel, Package, Transport, discovery::invalid};
use midnight_filesystem_plugin_api::{
    Failure, Request, Response, abi, decode_response, encode_request,
};
use midnight_plugin_host::TrustedLibrary;
use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
};

type Create = unsafe extern "C" fn(*const abi::Config, *mut u64) -> abi::Status;
type Call = unsafe extern "C" fn(u64, *const abi::Call, *mut abi::Buffer) -> abi::Status;
type Release = unsafe extern "C" fn(u64, u64) -> abi::Status;
type Destroy = unsafe extern "C" fn(u64) -> abi::Status;

#[derive(Debug)]
pub(crate) struct Instance {
    library: Option<TrustedLibrary>,
    instance: u64,
    call: Call,
    release: Release,
    destroy: Destroy,
    poisoned: AtomicBool,
    admission: crate::admission::Admission,
}

impl Instance {
    pub(crate) fn open(package: &Package) -> Result<Self, Failure> {
        let library =
            TrustedLibrary::open(&package.directory, &package.library).map_err(invalid)?;
        let symbol = library.symbol(abi::ENTRY_SYMBOL).map_err(invalid)?;
        // SAFETY: trusted library's origin-checked export promises this public C ABI.
        let entry: abi::Entry = unsafe { std::mem::transmute(symbol.as_ptr()) };
        let mut api = abi::Api::default();
        // SAFETY: exact writable caller-owned descriptor and retained module.
        status(unsafe { entry(&mut api, size_of::<abi::Api>() as u32) })?;
        validate_descriptor(&api)?;
        // SAFETY: bounded non-null immutable metadata promised by the retained plugin.
        let (id, version) = unsafe { (bytes(api.id, 128)?, bytes(api.version, 64)?) };
        if id != package.id.as_bytes()
            || version != package.version.as_bytes()
            || api.capabilities != package.capabilities
        {
            return Err(invalid("Filesystem descriptor differs from its manifest"));
        }
        let create: Create = api
            .create
            .ok_or_else(|| invalid("Missing create callback"))?;
        let call = api.call.ok_or_else(|| invalid("Missing call callback"))?;
        let release = api
            .release_buffer
            .ok_or_else(|| invalid("Missing buffer release callback"))?;
        let destroy = api
            .destroy
            .ok_or_else(|| invalid("Missing destroy callback"))?;
        for address in [
            create as *const c_void,
            call as *const c_void,
            release as *const c_void,
            destroy as *const c_void,
        ] {
            if !library.owns_address(address) {
                return Err(invalid("Filesystem callback belongs to another module"));
            }
        }
        let instance = create_instance(create)?;
        Ok(Self {
            library: Some(library),
            instance,
            call,
            release,
            destroy,
            poisoned: AtomicBool::new(false),
            admission: crate::admission::Admission::default(),
        })
    }
}

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;

struct Cancellation<'a>(Cancel<'a>);
unsafe extern "C" fn cancelled(context: u64) -> u32 {
    // SAFETY: call owns this stack context until all synchronous callbacks return.
    let cancel = unsafe { &*(context as *const Cancellation<'_>) };
    // Never unwind through C, including an unexpected caller cancellation panic.
    u32::from(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (cancel.0)())).unwrap_or(true),
    )
}

impl Transport for Instance {
    fn call(&self, request: &Request, cancellation: Cancel<'_>) -> Result<Response, Failure> {
        let cleanup = matches!(
            request,
            Request::CloseRead { .. } | Request::CloseList { .. } | Request::CloseMount { .. }
        );
        let control = matches!(
            request,
            Request::GetRenameState { .. } | Request::CloseRename { .. }
        );
        if !cleanup && self.poisoned.load(Ordering::Acquire) {
            return Err(invalid(
                "Native filesystem instance failed; restart required",
            ));
        }
        let preparing = matches!(request, Request::PrepareRename { .. });
        // Validate the fixed-size opening/control request before it can activate
        // persistent reservation. Other payloads remain bounded by admission.
        let opening_payload = if preparing || control {
            Some(
                encode_request(
                    request,
                    midnight_filesystem_plugin_api::DEFAULT_REQUEST_BYTES as usize,
                )
                .map_err(invalid)?,
            )
        } else {
            None
        };
        let _cleanup = cleanup.then(|| self.admission.cleanup());
        let _control = control.then(|| self.admission.control());
        let _ordinary = if cleanup || control {
            None
        } else if preparing {
            Some(self.admission.enter_rename(cancellation)?)
        } else {
            Some(self.admission.enter(cancellation)?)
        };
        // Poison may have been published while this request waited for admission.
        if !cleanup && self.poisoned.load(Ordering::Acquire) {
            return Err(invalid(
                "Native filesystem instance failed; restart required",
            ));
        }
        let payload = match opening_payload {
            Some(payload) => payload,
            None => encode_request(
                request,
                midnight_filesystem_plugin_api::DEFAULT_REQUEST_BYTES as usize,
            )
            .map_err(invalid)?,
        };
        // Controls inspect/dispose existing state even after user cancellation.
        let cancel = Cancellation(if control { &|| false } else { cancellation });
        let call = abi::Call {
            struct_size: size_of::<abi::Call>() as u32,
            operation: request.operation() as u32,
            payload: abi::Slice {
                data: payload.as_ptr(),
                len: payload.len() as u64,
            },
            cancel: abi::Cancellation {
                context: (&cancel as *const Cancellation<'_>) as u64,
                cancelled: Some(cancelled),
            },
        };
        let mut buffer = abi::Buffer::default();
        // SAFETY: retained instance/code and request/cancel storage lives through this call.
        let code = unsafe { (self.call)(self.instance, &call, &mut buffer) };
        if let Err(failure) = status(code) {
            // Failure transfers no buffer ownership. A contradictory nonzero
            // output can name somebody else's handle: never release it.
            if cleanup
                || matches!(request, Request::CloseRename { .. })
                || buffer.id != 0
                || buffer.bytes.len != 0
                || !buffer.bytes.data.is_null()
            {
                self.poisoned.store(true, Ordering::Release);
            }
            return Err(failure);
        }
        let mut guard = Buffer {
            owner: self,
            id: buffer.id,
        };
        let response = (|| {
            buffer
                .validate(midnight_filesystem_plugin_api::DEFAULT_REPLY_BYTES)
                .map_err(|_| invalid("Invalid filesystem reply buffer"))?;
            if buffer.id == 0 {
                return Err(invalid("Missing owned filesystem reply"));
            }
            // SAFETY: plugin retains this bounded immutable buffer until guard release.
            let data = unsafe {
                bytes(
                    buffer.bytes,
                    midnight_filesystem_plugin_api::DEFAULT_REPLY_BYTES,
                )?
            };
            let response = decode_response(
                data,
                midnight_filesystem_plugin_api::DEFAULT_REPLY_BYTES as usize,
            )
            .map_err(invalid)?;
            validate_rename_reply(request, &response)?;
            Ok(response)
        })();
        if code == abi::OK && response.is_err() {
            // An undecodable successful reply may have published an unknown
            // resource. Stop this instance instead of leaking another on retry;
            // its final destroy is the only safe cleanup for unknown handles.
            self.poisoned.store(true, Ordering::Release);
        }
        if cleanup && !matches!(&response, Ok(Response::Unit)) {
            // Reserved admission makes transient SDK call/buffer exhaustion
            // impossible here. Any remaining cleanup failure stops new work.
            self.poisoned.store(true, Ordering::Release);
        }
        // Every successful call transfers its buffer, even if decoding fails.
        if !guard.release() {
            return Err(invalid("Native filesystem buffer release failed"));
        }
        response
    }
}

fn validate_descriptor(api: &abi::Api) -> Result<(), Failure> {
    api.validate(size_of::<abi::Api>() as u32)
        .map_err(|code| invalid(format!("Invalid native filesystem ABI ({code})")))
}

fn validate_rename_reply(request: &Request, response: &Response) -> Result<(), Failure> {
    let valid = match request {
        Request::PrepareRename { .. } => {
            matches!(response, Response::RenamePrepared(_) | Response::Failed(_))
        }
        Request::Rename { .. } | Request::GetRenameState { .. } => {
            matches!(response, Response::RenameState(_) | Response::Failed(_))
        }
        Request::CloseRename { .. } => {
            matches!(response, Response::Unit)
                || matches!(response, Response::Failed(error)
                    if error.code == midnight_filesystem_plugin_api::ErrorCode::Busy)
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("Filesystem returned an unexpected Rename response"))
    }
}

struct Buffer<'a> {
    owner: &'a Instance,
    id: u64,
}
impl Buffer<'_> {
    fn release(&mut self) -> bool {
        let id = std::mem::take(&mut self.id);
        if id != 0 {
            // SAFETY: owned buffer ID, live instance and retained release callback.
            let result = unsafe { (self.owner.release)(self.owner.instance, id) };
            if result != abi::OK {
                self.owner.poisoned.store(true, Ordering::Release);
                return false;
            }
        }
        true
    }
}
impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        self.release();
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: last owner, no borrowed calls remain; plugin closes all retained resources.
        let status = unsafe { (self.destroy)(self.instance) };
        retire_library(self.library.take(), status);
    }
}
fn retire_library<T>(library: Option<T>, status: abi::Status) {
    if status == abi::OK {
        drop(library);
    } else {
        // A faulty plugin may still own callbacks/buffers. Quarantine its module
        // until process exit instead of unmapping code beneath live native state.
        std::mem::forget(library);
    }
}
fn create_instance(create: Create) -> Result<u64, Failure> {
    let mut instance = 0;
    // SAFETY: caller retains verified callback code and supplies exact writable storage.
    let result = unsafe { create(&abi::Config::default(), &mut instance) };
    // Failed creation never transfers an instance, even with contradictory output.
    status(result)?;
    if instance == 0 {
        return Err(invalid("Missing created native filesystem instance"));
    }
    Ok(instance)
}
fn status(value: abi::Status) -> Result<(), Failure> {
    if value == abi::OK {
        Ok(())
    } else {
        let code = match value {
            abi::BUSY => midnight_filesystem_plugin_api::ErrorCode::Busy,
            abi::RESOURCE_LIMIT => midnight_filesystem_plugin_api::ErrorCode::ResourceLimit,
            abi::UNSUPPORTED | abi::INCOMPATIBLE => {
                midnight_filesystem_plugin_api::ErrorCode::Unsupported
            }
            abi::UNKNOWN_HANDLE => midnight_filesystem_plugin_api::ErrorCode::StaleReference,
            _ => midnight_filesystem_plugin_api::ErrorCode::Invalid,
        };
        Err(Failure::new(
            code,
            format!("Native filesystem status {value}"),
        ))
    }
}
unsafe fn bytes<'a>(value: abi::Slice, maximum: u64) -> Result<&'a [u8], Failure> {
    value
        .validate(maximum)
        .map_err(|_| invalid("Invalid filesystem byte slice"))?;
    if value.len == 0 {
        return Ok(&[]);
    }
    // SAFETY: caller retains mapped plugin memory, bounded non-null length checked above.
    Ok(unsafe { std::slice::from_raw_parts(value.data, value.len as usize) })
}
