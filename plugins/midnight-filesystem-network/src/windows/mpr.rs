//! The only unsafe boundary: owned MPR handles and initialized aligned output buffers.
use super::*;
use resource::Resource;
use std::{collections::VecDeque, sync::Arc};
mod buffer;
use buffer::Buffer;
use windows_sys::Win32::{Foundation::*, NetworkManagement::WNet::*};
mod system;
const MAX_NATIVE_CALLS: usize = 128;
const MAX_RESOURCES: usize = 4096;
trait Api: Send + Sync {
    fn open(&self, scope: u32, parent: Option<&Resource>) -> (u32, usize);
    fn next(&self, handle: usize, count: &mut u32, buffer: &mut Buffer, bytes: &mut u32) -> u32;
    fn close(&self, handle: usize);
    fn extended_error(&self) -> Option<u32> {
        None
    }
}
pub struct Cursor {
    handle: Option<Enumeration>,
    buffer: Buffer,
    pending: VecDeque<Record>,
    failed: Option<Failure>,
    root: bool,
    calls: usize,
    resources: usize,
}
struct Enumeration {
    raw: usize,
    api: Arc<dyn Api>,
}
impl Drop for Enumeration {
    fn drop(&mut self) {
        self.api.close(self.raw);
    }
}
impl std::fmt::Debug for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WindowsNetworkCursor")
    }
}
pub(super) fn begin(target: &Target, context: &Context<'_>) -> Result<Cursor, Failure> {
    begin_with(target, context, Arc::new(system::System))
}
fn begin_with(
    target: &Target,
    context: &Context<'_>,
    api: Arc<dyn Api>,
) -> Result<Cursor, Failure> {
    context.check()?;
    Windows::validate(target)?;
    if target.kind == NodeKind::Share {
        return Err(unsupported());
    }
    let root = target.kind == NodeKind::Root;
    let parent = if root {
        None
    } else {
        Some(Resource::decode(&target.key)?)
    };
    let buffer = Buffer::new(16 * 1024)?;
    context.check()?;
    // The global root lists installed provider containers without first asking
    // every provider to discover the caller's network neighborhood. On current
    // Windows, RESOURCE_CONTEXT can block on an unavailable browser service and
    // return only an unnamed Entire Network wrapper. Open its provider children
    // directly; selected containers still retain their exact native identity.
    let (status, raw) = api.open(RESOURCE_GLOBALNET, parent.as_ref());
    if status != NO_ERROR {
        return Err(native_error(api.as_ref(), status, root, context));
    }
    if raw == 0 || raw == usize::MAX {
        context.check()?;
        return Err(invalid());
    }
    // Ownership starts only after native success. Cancellation after open drops
    // this guard, while failed native calls never grant ownership of output bits.
    let handle = Enumeration { raw, api };
    context.check()?;
    Ok(Cursor {
        handle: Some(handle),
        buffer,
        pending: VecDeque::new(),
        failed: None,
        root,
        calls: 0,
        resources: 0,
    })
}
impl Cursor {
    pub fn next(&mut self, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        let result = self.advance(context);
        if let Err(error) = &result {
            self.failed = Some(error.clone());
            self.pending.clear();
            drop(self.handle.take()); // Native cleanup stays on this admitted caller.
        }
        result
    }
    fn advance(&mut self, context: &Context<'_>) -> Result<Option<Record>, Failure> {
        loop {
            context.check()?;
            if let Some(error) = &self.failed {
                return Err(error.clone());
            }
            if let Some(record) = self.pending.pop_front() {
                return Ok(Some(record));
            }
            let Some(handle) = &self.handle else {
                return Ok(None);
            };
            if self.calls >= MAX_NATIVE_CALLS {
                return Err(limit());
            }
            self.calls += 1;
            let mut count = crate::api::MAX_BATCH;
            let mut bytes = self.buffer.len() as u32;
            self.buffer.clear();
            let status = handle
                .api
                .next(handle.raw, &mut count, &mut self.buffer, &mut bytes);
            match status {
                NO_ERROR => {
                    context.check()?;
                    self.resources = self.resources.saturating_add(count as usize);
                    if self.resources > MAX_RESOURCES {
                        return Err(limit());
                    }
                    let values = self.buffer.parse(count)?;
                    self.pending
                        .try_reserve(values.len())
                        .map_err(|_| limit())?;
                    let mut retained = self
                        .pending
                        .capacity()
                        .saturating_mul(std::mem::size_of::<Record>());
                    for value in values {
                        context.check()?;
                        let Some(record) = value.record()? else {
                            continue;
                        };
                        // Ordinary discovery does not advertise hidden/system shares.
                        // Explicit manual C$/ADMIN$ targets remain available to activation.
                        if record.target.kind == NodeKind::Share
                            && value
                                .remote
                                .as_ref()
                                .is_some_and(|name| name.last() == Some(&36))
                        {
                            continue;
                        }
                        retained = retained
                            .saturating_add(record.target.key.capacity())
                            .saturating_add(record.display.capacity());
                        if retained > buffer::MAX_BUFFER {
                            return Err(limit());
                        }
                        self.pending.push_back(record);
                    }
                }
                ERROR_MORE_DATA => {
                    context.check()?;
                    // Only successful calls define returned records/count. A failed
                    // buffer is discarded even when the provider left nonzero count.
                    let required = (bytes as usize).max(self.buffer.len().saturating_mul(2));
                    self.buffer = Buffer::new(required)?;
                }
                ERROR_NO_MORE_ITEMS => {
                    drop(self.handle.take());
                    context.check()?;
                    return Ok(None);
                }
                error => return Err(native_error(handle.api.as_ref(), error, self.root, context)),
            }
        }
    }
}
fn native_error(api: &dyn Api, code: u32, root: bool, context: &Context<'_>) -> Failure {
    // A completed native failure is more specific than the elapsed request
    // budget. Preserve it even after a slow OS call; cancellation still wins.
    // Successful results and retries remain subject to the normal deadline.
    if let Some(error) = context
        .check()
        .err()
        .filter(|error| error.code == ErrorCode::Cancelled)
    {
        return error;
    }
    if code == ERROR_EXTENDED_ERROR {
        let extended = api.extended_error();
        if let Some(error) = context
            .check()
            .err()
            .filter(|error| error.code == ErrorCode::Cancelled)
        {
            return error;
        }
        let mut error = failure(code, root);
        if let Some(number) = extended {
            // The extended namespace belongs to the provider, not WinError.h.
            // Do not turn a coincident number into credential or mount authority.
            error.message = format!("Windows network provider reported extended error {number}.");
        }
        return error;
    }
    failure(code, root)
}
fn failure(code: u32, root: bool) -> Failure {
    let category = match code {
        ERROR_BAD_PROVIDER | ERROR_NOT_SUPPORTED | ERROR_SERVICE_NOT_ACTIVE => {
            ErrorCode::Unavailable
        }
        ERROR_NO_NETWORK if root => ErrorCode::Unavailable,
        ERROR_NO_NETWORK
        | ERROR_BAD_NETPATH
        | ERROR_NETWORK_UNREACHABLE
        | ERROR_HOST_UNREACHABLE
        | ERROR_CONNECTION_UNAVAIL
        | ERROR_NO_LOGON_SERVERS => ErrorCode::Offline,
        ERROR_ACCESS_DENIED | ERROR_NETWORK_ACCESS_DENIED | ERROR_LOGON_TYPE_NOT_GRANTED => {
            ErrorCode::PermissionDenied
        }
        ERROR_LOGON_FAILURE | ERROR_NOT_AUTHENTICATED | ERROR_SESSION_CREDENTIAL_CONFLICT => {
            ErrorCode::AuthenticationRequired
        }
        ERROR_BAD_NET_NAME | ERROR_PATH_NOT_FOUND | ERROR_FILE_NOT_FOUND => ErrorCode::NotFound,
        ERROR_TIMEOUT | ERROR_SEM_TIMEOUT => ErrorCode::TimedOut,
        ERROR_CANCELLED | ERROR_OPERATION_ABORTED => ErrorCode::Cancelled,
        ERROR_NOT_ENOUGH_MEMORY | ERROR_OUTOFMEMORY => ErrorCode::ResourceLimit,
        ERROR_INVALID_PARAMETER | ERROR_NOT_CONTAINER | ERROR_INVALID_ADDRESS => ErrorCode::Invalid,
        _ => ErrorCode::Io,
    };
    let mut value = Failure::new(category, "Windows network enumeration failed.");
    value.os_code = Some(code as i32);
    value
}
#[cfg(test)]
mod buffer_tests;
#[cfg(test)]
mod tests;
