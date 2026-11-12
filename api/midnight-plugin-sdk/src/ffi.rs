//! The only raw-pointer adapter. The caller must supply live ABI-owned memory;
//! validation checks shape, bounds and lifetime protocol, not arbitrary addresses.
use crate::{
    guard::contain,
    host::Host,
    outbox::Origin,
    plugin::{self, Request, Viewer},
};
use midnight_plugin_api::{abi, codec, messages, wire};

fn boundary(work: impl FnOnce() -> Result<(), abi::Status>) -> abi::Status {
    match contain(work) {
        Ok(Ok(())) => abi::OK,
        Ok(Err(status)) => status,
        Err(()) => abi::PANICKED,
    }
}

fn pointer<T>(value: *const T) -> Result<(), abi::Status> {
    if value.is_null() || !value.is_aligned() {
        Err(abi::INVALID)
    } else {
        Ok(())
    }
}

unsafe fn read<T: Copy>(value: *const T) -> Result<T, abi::Status> {
    pointer(value)?;
    // SAFETY: callers supply a live complete ABI value, checked for null/alignment.
    Ok(unsafe { value.read() })
}

unsafe fn slice<'a>(value: abi::ByteSlice, maximum: u32) -> Result<&'a [u8], abi::Status> {
    value.validate(maximum)?;
    if value.len == 0 {
        return Ok(&[]);
    }
    // SAFETY: caller provides mapped immutable memory through the current call;
    // accepted maximum is <=1 MiB, below isize::MAX on every supported platform.
    Ok(unsafe { std::slice::from_raw_parts(value.data, value.len as usize) })
}

/// # Safety
/// `output` must reference writable aligned memory covering `capacity` bytes,
/// exclusively borrowed during the call. Keep the library loaded for its use.
pub unsafe fn entry<T: Viewer>(output: *mut abi::PluginApi, capacity: u32) -> abi::Status {
    boundary(|| {
        pointer(output)?;
        if capacity < size_of::<abi::PluginApi>() as u32 {
            return Err(abi::INVALID);
        }
        let table = abi::PluginApi {
            header: abi::Header {
                struct_size: size_of::<abi::PluginApi>() as u32,
                abi_major: abi::ABI_MAJOR,
                abi_minor: abi::ABI_MINOR,
                kind: abi::KIND_VIEWER,
                minimum_host_minor: 0,
                reserved: 0,
            },
            id: metadata(T::ID),
            version: metadata(T::VERSION),
            create: Some(create::<T>),
            submit: Some(submit),
            cancel: Some(cancel),
            drain: Some(drain),
            release_buffer: Some(release_buffer),
            begin_shutdown: Some(begin_shutdown),
            destroy: Some(destroy),
        };
        table.validate(capacity, abi::KIND_VIEWER)?;
        // SAFETY: caller grants writable capacity and pointer alignment was checked.
        unsafe { output.write(table) };
        Ok(())
    })
}

fn metadata(text: &'static str) -> abi::ByteSlice {
    abi::ByteSlice {
        data: text.as_ptr(),
        len: u32::try_from(text.len()).unwrap_or(u32::MAX),
        reserved: 0,
    }
}

unsafe extern "C" fn create<T: Viewer>(
    host: *const abi::HostApi,
    configuration: abi::ByteSlice,
    output: *mut u64,
) -> abi::Status {
    boundary(|| {
        pointer(output)?;
        // SAFETY: ABI caller owns one writable u64, checked for null/alignment.
        unsafe { output.write(0) };
        // SAFETY: the ABI always supplies a readable header first.
        let header = unsafe { read(host.cast::<abi::Header>()) }?;
        header.validate(
            header.struct_size,
            size_of::<abi::HostApi>() as u32,
            abi::KIND_HOST,
        )?;
        // SAFETY: validated size covers the required table; caller keeps host live
        // through quiescent instance destruction, including all jobs and leases.
        let host = unsafe { Host::new(read(host)?) }?;
        let configuration = unsafe { slice(configuration, 65536) }?;
        if !configuration.is_empty() {
            wire::validate(configuration).map_err(|_| abi::INVALID)?;
        }
        let id = plugin::create::<T>(host, configuration)?;
        // SAFETY: same live output as initialized above; no callbacks retain it.
        unsafe { output.write(id) };
        Ok(())
    })
}

unsafe extern "C" fn submit(id: u64, request: *const abi::Request) -> abi::Status {
    boundary(|| {
        // SAFETY: caller borrows a full request and its payload for this call.
        let request = unsafe { read(request) }?;
        if request.flags != 0 || request.id == 0 || request.session == 0 {
            return Err(abi::INVALID);
        }
        let payload = unsafe { slice(request.payload, wire::MAX_MESSAGE_BYTES as u32) }?;
        let origin = Origin {
            id: request.id,
            session: request.session,
            generation: request.generation,
        };
        plugin::with_instance(id, |instance| {
            let result = if request.kind == abi::REQUEST_SERVICE_REPLY {
                let reply: messages::ReadReply =
                    codec::decode(payload).map_err(|_| abi::INVALID)?;
                // SAFETY: host supplies its immutable allocation and owns the initial
                // reference through submit. SDK retains before returning or scheduling.
                let content = unsafe { instance.host.retain(request.content) }?;
                instance.runtime.reply(origin, reply, content)
            } else {
                request.content.validate(0)?;
                if request.content.id != 0 {
                    return Err(abi::INVALID);
                }
                let owned = Request::new(request.kind, payload, instance.input_bytes.clone())?;
                let handler = instance.handler.clone();
                instance.runtime.submit(
                    origin,
                    Box::new(move |context| handler.handle(owned, context)),
                )
            };
            if result == abi::OK {
                Ok(())
            } else {
                Err(result)
            }
        })
    })
}

extern "C" fn cancel(id: u64, request: u64) -> abi::Status {
    boundary(|| plugin::with_instance(id, |instance| instance.runtime.cancel(request)))
}

unsafe extern "C" fn drain(
    id: u64,
    capacity: u32,
    events: *mut abi::Event,
    written: *mut u32,
) -> abi::Status {
    boundary(|| {
        pointer(written)?;
        // SAFETY: caller supplies a writable non-overlapping count value.
        unsafe { written.write(0) };
        if capacity > 64 {
            return Err(abi::INVALID);
        }
        if capacity != 0 {
            pointer(events)?;
        }
        plugin::with_instance(id, |instance| {
            for index in 0..capacity {
                let Some(event) = instance.runtime.drain()? else {
                    break;
                };
                // SAFETY: caller exclusively owns capacity aligned Event slots;
                // index is bounded. Count remains valid even on a later error.
                unsafe {
                    events.add(index as usize).write(event);
                    written.write(index + 1);
                }
            }
            Ok(())
        })
    })
}

extern "C" fn release_buffer(id: u64, buffer: u64) -> abi::Status {
    boundary(|| plugin::with_instance(id, |instance| instance.runtime.release(buffer)))
}

extern "C" fn begin_shutdown(id: u64) -> abi::Status {
    boundary(|| {
        plugin::with_instance(id, |instance| {
            instance.runtime.shutdown();
            Ok(())
        })
    })
}

extern "C" fn destroy(id: u64) -> abi::Status {
    boundary(|| plugin::destroy(id))
}

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;
