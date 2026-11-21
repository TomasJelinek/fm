//! The only bytes Media Foundation sees: a read-only, seekable `IMFByteStream` over the
//! host-brokered capture. It has no URL or path, so no source can reopen, follow or fetch
//! anything else; writes are refused.
use super::remote::Reads;
use std::{collections::HashMap, sync::Mutex};
use windows::{
    Win32::{
        Foundation::{E_ACCESSDENIED, E_FAIL, E_INVALIDARG, E_POINTER},
        Media::MediaFoundation::{
            IMFAsyncCallback, IMFAsyncResult, IMFByteStream, IMFByteStream_Impl,
            MFBYTESTREAM_DOES_NOT_USE_NETWORK, MFBYTESTREAM_IS_READABLE, MFBYTESTREAM_IS_SEEKABLE,
            MFBYTESTREAM_SEEK_ORIGIN, MFCreateAsyncResult, MFInvokeCallback, msoBegin, msoCurrent,
        },
    },
    core::{BOOL, Error, IUnknown, Interface, Ref, Result, implement},
};

#[implement(IMFByteStream)]
pub(super) struct Stream {
    reads: Reads,
    position: Mutex<u64>,
    /// Completed asynchronous reads, by their result object, until `EndRead` collects them.
    completed: Mutex<HashMap<usize, u32>>,
}

impl Stream {
    pub(super) fn create(reads: Reads) -> IMFByteStream {
        Self {
            reads,
            position: Mutex::new(0),
            completed: Mutex::new(HashMap::new()),
        }
        .into()
    }

    fn length(&self) -> u64 {
        self.reads.length()
    }

    fn position(&self) -> Result<std::sync::MutexGuard<'_, u64>> {
        self.position.lock().map_err(|_| Error::from(E_FAIL))
    }

    /// Read at the current position into `buffer`, advancing past what was read.
    fn read_into(&self, buffer: *mut u8, capacity: u32) -> Result<u32> {
        if buffer.is_null() && capacity != 0 {
            return Err(E_POINTER.into());
        }
        // Hold the position across the brokered read, so concurrent callers stay ordered.
        let mut position = self.position()?;
        let wanted = (self.length() - *position).min(u64::from(capacity)) as usize;
        if wanted == 0 {
            return Ok(0);
        }
        let bytes = self
            .reads
            .read_at(*position, wanted)
            .map_err(|_| Error::from(E_FAIL))?;
        if bytes.len() > wanted {
            return Err(E_FAIL.into());
        }
        // SAFETY: Media Foundation passes a writable buffer of `capacity` bytes for this call,
        // and `bytes` holds at most `wanted <= capacity` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len()) };
        *position += bytes.len() as u64;
        Ok(bytes.len() as u32)
    }

    fn move_to(&self, position: i128) -> Result<u64> {
        let position = u64::try_from(position)
            .ok()
            .filter(|position| *position <= self.length())
            .ok_or_else(|| Error::from(E_INVALIDARG))?;
        *self.position()? = position;
        Ok(position)
    }
}

impl IMFByteStream_Impl for Stream_Impl {
    fn GetCapabilities(&self) -> Result<u32> {
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_SEEKABLE | MFBYTESTREAM_DOES_NOT_USE_NETWORK)
    }

    fn GetLength(&self) -> Result<u64> {
        Ok(self.length())
    }

    fn SetLength(&self, _: u64) -> Result<()> {
        Err(E_ACCESSDENIED.into())
    }

    fn GetCurrentPosition(&self) -> Result<u64> {
        Ok(*self.position()?)
    }

    fn SetCurrentPosition(&self, position: u64) -> Result<()> {
        self.move_to(i128::from(position)).map(|_| ())
    }

    fn IsEndOfStream(&self) -> Result<BOOL> {
        Ok((*self.position()? >= self.length()).into())
    }

    fn Read(&self, buffer: *mut u8, capacity: u32, read: *mut u32) -> Result<()> {
        if read.is_null() {
            return Err(E_POINTER.into());
        }
        let count = self.read_into(buffer, capacity)?;
        // SAFETY: checked non-null; Media Foundation passes a writable count.
        unsafe { *read = count };
        Ok(())
    }

    fn BeginRead(
        &self,
        buffer: *mut u8,
        capacity: u32,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> Result<()> {
        let callback = callback.ok()?;
        // The read completes before the callback is queued, like a synchronous file stream.
        let count = self.read_into(buffer, capacity);
        // SAFETY: valid callback and optional state objects for a new result object.
        let result = unsafe { MFCreateAsyncResult(None::<&IUnknown>, callback, state.as_ref())? };
        match count {
            Ok(count) => {
                self.completed
                    .lock()
                    .map_err(|_| Error::from(E_FAIL))?
                    .insert(result.as_raw() as usize, count);
            }
            // SAFETY: a live result object owned by this call.
            Err(error) => unsafe { result.SetStatus(error.code())? },
        }
        // SAFETY: the result names the caller's callback; Media Foundation queues its Invoke.
        unsafe { MFInvokeCallback(&result) }
    }

    fn EndRead(&self, result: Ref<IMFAsyncResult>) -> Result<u32> {
        let result = result.ok()?;
        let count = self
            .completed
            .lock()
            .map_err(|_| Error::from(E_FAIL))?
            .remove(&(result.as_raw() as usize));
        // SAFETY: a live result object passed back by Media Foundation.
        unsafe { result.GetStatus()? };
        count.ok_or_else(|| E_INVALIDARG.into())
    }

    fn Write(&self, _: *const u8, _: u32) -> Result<u32> {
        Err(E_ACCESSDENIED.into())
    }

    fn BeginWrite(
        &self,
        _: *const u8,
        _: u32,
        _: Ref<IMFAsyncCallback>,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Err(E_ACCESSDENIED.into())
    }

    fn EndWrite(&self, _: Ref<IMFAsyncResult>) -> Result<u32> {
        Err(E_ACCESSDENIED.into())
    }

    fn Seek(&self, origin: MFBYTESTREAM_SEEK_ORIGIN, offset: i64, _: u32) -> Result<u64> {
        let base = if origin == msoBegin {
            0
        } else if origin == msoCurrent {
            *self.position()?
        } else {
            return Err(E_INVALIDARG.into());
        };
        self.move_to(i128::from(base) + i128::from(offset))
    }

    fn Flush(&self) -> Result<()> {
        Ok(())
    }

    fn Close(&self) -> Result<()> {
        Ok(())
    }
}
