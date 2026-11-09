//! Audited pointer/callback boundary; native memory is trusted but metadata is bounded.
use crate::{
    Filesystem, Reply,
    api::{self, ErrorCode, Failure, Request, Response, abi::*},
    runtime,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Cancellation is borrowed for one synchronous call, never a stored native callback.
///
/// ```compile_fail
/// use midnight_filesystem_plugin_sdk::CallContext;
/// fn retain(context:&CallContext<'_>)->Box<dyn Fn()->bool>{
///     Box::new(move||context.is_cancelled())
/// }
/// ```
pub struct CallContext<'call> {
    cancel: &'call Cancellation,
    config: Config,
}
impl CallContext<'_> {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.cancelled.is_some_and(|callback| {
            // SAFETY: the caller retains a thread-safe, non-reentrant callback/context
            // until this call returns; the safe context cannot escape that borrow.
            unsafe { callback(self.cancel.context) != 0 }
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
}
fn guarded(action: impl FnOnce() -> Status) -> Status {
    catch_unwind(AssertUnwindSafe(action)).unwrap_or(PANICKED)
}

/// Fill the v1 descriptor. All callbacks are optional pointers in its representation.
/// # Safety
/// `out` must be aligned, mapped and writable for `capacity` bytes during the call.
pub unsafe fn entry<F: Filesystem>(
    out: *mut Api,
    capacity: u32,
    id: &'static str,
    version: &'static str,
    capabilities: u64,
) -> Status {
    guarded(|| {
        if out.is_null() || !out.is_aligned() || capacity < size_of::<Api>() as u32 {
            return INVALID;
        }
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || version.is_empty()
            || version.len() > 64
            || version.contains('\0')
        {
            return INVALID;
        }
        let table = Api {
            header: Header::v1(size_of::<Api>() as u32),
            capabilities,
            id: Slice {
                data: id.as_ptr(),
                len: id.len() as u64,
            },
            version: Slice {
                data: version.as_ptr(),
                len: version.len() as u64,
            },
            create: Some(create::<F>),
            call: Some(call::<F>),
            release_buffer: Some(release::<F>),
            destroy: Some(destroy::<F>),
        };
        // SAFETY: checked alignment/capacity and caller's writable mapping contract.
        unsafe { out.write(table) };
        OK
    })
}
unsafe extern "C" fn create<F: Filesystem>(config: *const Config, out: *mut u64) -> Status {
    guarded(|| {
        if out.is_null() || !out.is_aligned() {
            return INVALID;
        }
        // SAFETY: caller owns a writable aligned u64 for the duration of this entry.
        unsafe { out.write(0) };
        if config.is_null() || !config.is_aligned() {
            return INVALID;
        }
        // SAFETY: the caller maps at least the leading struct_size field.
        if unsafe { config.cast::<u32>().read() } < size_of::<Config>() as u32 {
            return INVALID;
        }
        // SAFETY: the caller maps its declared capacity; all Config bit patterns are valid.
        let config = unsafe { config.read() };
        match runtime::create::<F>(config) {
            Ok(id) => {
                unsafe { out.write(id) };
                OK
            }
            Err(status) => status,
        }
    })
}
unsafe extern "C" fn call<F: Filesystem>(id: u64, input: *const Call, out: *mut Buffer) -> Status {
    guarded(|| {
        if out.is_null() || !out.is_aligned() {
            return INVALID;
        }
        // SAFETY: caller maps this aligned writable Buffer until return.
        unsafe { out.write(Buffer::default()) };
        if input.is_null() || !input.is_aligned() {
            return INVALID;
        }
        // SAFETY: caller maps the leading struct_size field.
        if unsafe { input.cast::<u32>().read() } < size_of::<Call>() as u32 {
            return INVALID;
        }
        // SAFETY: the Call declaration and any optional callback pointer are caller-valid.
        let input = unsafe { input.read() };
        let instance = match runtime::acquire::<F>(id) {
            Ok(instance) => instance,
            Err(status) => return status,
        };
        if input
            .payload
            .validate(instance.config.request_bytes)
            .is_err()
        {
            return INVALID;
        }
        let length = match usize::try_from(input.payload.len) {
            Ok(length) => length,
            Err(_) => return INVALID,
        };
        let bytes = if length == 0 {
            &[]
        } else {
            // SAFETY: non-null bounded slice metadata and caller's immutable mapped borrow.
            unsafe { std::slice::from_raw_parts(input.payload.data, length) }
        };
        let mut permit = match runtime::Permit::acquire(instance.clone()) {
            Ok(permit) => permit,
            Err(status) => return status,
        };
        let request = match api::decode_request(
            input.operation,
            bytes,
            instance.config.request_bytes as usize,
        ) {
            Ok(request) => request,
            Err(api::WireError::Unsupported) => return UNSUPPORTED,
            Err(_) => return INVALID,
        };
        let context = CallContext {
            cancel: &input.cancel,
            config: instance.config,
        };
        let cleanup = matches!(
            request,
            Request::CloseList { .. } | Request::CloseRead { .. } | Request::CloseMount { .. }
        );
        let response = if !cleanup && context.is_cancelled() {
            Reply::stateless(Response::Failed(Failure::new(
                ErrorCode::Cancelled,
                "Filesystem request canceled.",
            )))
            .map_err(|_| INVALID)
        } else {
            match permit.run(request, &context) {
                Ok(response) => Ok(response),
                Err(status) => return status,
            }
        };
        let response = match response {
            Ok(response) => response,
            Err(status) => return status,
        };
        let (mut response, completion) = response.into_parts();
        let mut publication = permit.completion(completion);
        if !cleanup && context.is_cancelled() {
            if let Err(status) = publication.finish(false) {
                return status;
            }
            response = Response::Failed(Failure::new(
                ErrorCode::Cancelled,
                "Filesystem request canceled.",
            ));
        }
        let bytes = match api::encode_response(&response, instance.config.reply_bytes as usize) {
            Ok(bytes) => bytes,
            Err(api::WireError::Limit) => return RESOURCE_LIMIT,
            Err(_) => return INVALID,
        };
        drop(response);
        match permit.publish(bytes) {
            Ok(buffer) => {
                if let Err(status) = publication.finish(true) {
                    let _ = runtime::release::<F>(id, buffer.id);
                    return status;
                }
                unsafe { out.write(buffer) };
                OK
            }
            Err(status) => status,
        }
    })
}
unsafe extern "C" fn release<F: Filesystem>(instance: u64, id: u64) -> Status {
    guarded(|| runtime::release::<F>(instance, id).map_or_else(|status| status, |()| OK))
}
unsafe extern "C" fn destroy<F: Filesystem>(instance: u64) -> Status {
    guarded(|| runtime::destroy::<F>(instance).map_or_else(|status| status, |()| OK))
}
