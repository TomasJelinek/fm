//! Shared-mode WASAPI output owned by the playback worker.
use ::windows::{
    Win32::{
        Foundation::{E_ACCESSDENIED, ERROR_NOT_FOUND},
        Media::Audio::{
            AUDCLNT_E_DEVICE_IN_USE, AUDCLNT_E_DEVICE_INVALIDATED,
            AUDCLNT_E_ENDPOINT_CREATE_FAILED, AUDCLNT_E_RESOURCES_INVALIDATED,
            AUDCLNT_E_SERVICE_NOT_RUNNING, AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            IAudioClient, IAudioClock, IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
            WAVE_FORMAT_PCM, WAVEFORMATEX, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
            CoInitializeEx, CoUninitialize,
        },
    },
    core::{HRESULT, Result as WindowsResult},
};
use midnight_media_host::{
    AudioPause, AudioSink, DeviceClock, MAX_DEVICE_FRAMES, PCM_CHANNELS, PCM_RATE,
};
use std::{collections::VecDeque, io, marker::PhantomData, mem::size_of, ptr, rc::Rc};

const HNS_PER_SECOND: u128 = 10_000_000;
const BUFFER_DURATION_HNS: i64 = 500_000; // 50 milliseconds
const FRAME_BYTES: usize = PCM_CHANNELS * size_of::<i16>();

pub(super) fn open_default() -> io::Result<Box<dyn AudioSink>> {
    let apartment = Apartment::initialize()?;
    // SAFETY: COM is initialized on this worker and these instances stay here.
    let enumerator: IMMDeviceEnumerator =
        wasapi("create Windows audio endpoint enumerator", unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        })?;
    // SAFETY: enumerate only the active default multimedia rendering endpoint.
    let endpoint = wasapi("open the default Windows audio output", unsafe {
        enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)
    })?;
    // SAFETY: the endpoint is live and no activation parameters are needed.
    let client: IAudioClient = wasapi("activate Windows audio output", unsafe {
        endpoint.Activate(CLSCTX_ALL, None)
    })?;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: PCM_CHANNELS as u16,
        nSamplesPerSec: PCM_RATE,
        nAvgBytesPerSec: PCM_RATE * FRAME_BYTES as u32,
        nBlockAlign: FRAME_BYTES as u16,
        wBitsPerSample: i16::BITS as u16,
        cbSize: 0,
    };
    // SAFETY: the PCM descriptor is live through this synchronous shared-mode
    // initialization. A zero periodicity lets the engine choose its period.
    wasapi("initialize shared Windows audio output", unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            BUFFER_DURATION_HNS,
            0,
            &format,
            None,
        )
    })?;
    // SAFETY: the initialized client remains owned on this thread.
    let capacity = wasapi("read Windows audio buffer size", unsafe {
        client.GetBufferSize()
    })? as usize;
    if capacity == 0 {
        return Err(invalid_clock(
            "Windows audio output reported an empty device buffer",
        ));
    }
    // SAFETY: both services belong to this live initialized render stream.
    let render: IAudioRenderClient = wasapi("open Windows audio render buffer", unsafe {
        client.GetService()
    })?;
    // SAFETY: the stream and its clock stay on this worker until release.
    let position: IAudioClock = wasapi("open Windows audio stream clock", unsafe {
        client.GetService()
    })?;
    // SAFETY: the clock and client are live on the thread that acquired them.
    let frequency = wasapi("read Windows audio clock frequency", unsafe {
        position.GetFrequency()
    })?;
    // SAFETY: latency is available after successful shared-mode initialization.
    let latency = wasapi("read Windows audio stream latency", unsafe {
        client.GetStreamLatency()
    })?;
    let latency_ticks = stream_latency_ticks(latency, frequency, capacity)?;

    Ok(Box::new(WasapiSink {
        render,
        position,
        client,
        capacity,
        submitted: 0,
        state: StreamState::Stopped,
        timeline: Timeline::new(latency_ticks),
        _apartment: apartment,
    }))
}

fn stream_latency_ticks(latency: i64, frequency: u64, capacity: usize) -> io::Result<u64> {
    // A successfully reported zero latency contributes no downstream clock ticks.
    if latency < 0 || frequency == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Windows audio output has no usable clock or latency bound: \
                 latency_hns={latency}, clock_units_per_second={frequency}, \
                 capacity_frames={capacity}, requested_buffer_hns={BUFFER_DURATION_HNS}"
            ),
        ));
    }
    u64::try_from((latency as u128 * u128::from(frequency)).div_ceil(HNS_PER_SECOND))
        .map_err(|_| invalid_clock("Windows audio latency exceeds the stream clock range"))
}

struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    fn initialize() -> io::Result<Self> {
        // SAFETY: the reserved pointer is null. This guard stays on its initializing
        // worker and balances every successful call, including S_FALSE.
        wasapi("initialize COM for Windows audio", unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok()
        })?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this thread owns the matching successful initialization.
        unsafe { CoUninitialize() };
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamState {
    Stopped,
    WaitingForPcm,
    Running,
}

struct WasapiSink {
    // Field order releases both services, then the client, then COM on this worker.
    render: IAudioRenderClient,
    position: IAudioClock,
    client: IAudioClient,
    capacity: usize,
    submitted: u64,
    state: StreamState,
    timeline: Timeline,
    _apartment: Apartment,
}

impl WasapiSink {
    fn start_stream(&mut self) -> io::Result<()> {
        // SAFETY: this initialized stream and its primed buffer belong to this worker.
        wasapi("start Windows audio output", unsafe { self.client.Start() })?;
        self.state = StreamState::Running;
        Ok(())
    }

    fn stop_stream(&mut self) -> io::Result<()> {
        // SAFETY: Stop accepts both running and stopped initialized streams.
        wasapi("stop Windows audio output", unsafe { self.client.Stop() })?;
        self.state = StreamState::Stopped;
        Ok(())
    }

    fn padding(&self) -> io::Result<usize> {
        // SAFETY: this initialized shared render stream has a live endpoint buffer.
        let padding = wasapi("read Windows audio padding", unsafe {
            self.client.GetCurrentPadding()
        })? as usize;
        if padding > self.capacity || padding as u64 > self.submitted {
            return Err(invalid_clock(
                "Windows audio output reported invalid buffer padding",
            ));
        }
        Ok(padding)
    }
}

impl AudioSink for WasapiSink {
    fn start(&mut self) -> io::Result<()> {
        if self.state != StreamState::Stopped {
            return Ok(());
        }
        if self.timeline.outstanding(self.submitted)? == 0 {
            self.padding()?;
            self.state = StreamState::WaitingForPcm;
            return Ok(());
        }
        self.start_stream()
    }

    fn pause(&mut self) -> io::Result<AudioPause> {
        self.stop_stream()?;
        self.sample_clock().map(AudioPause::Retained)
    }

    fn reset(&mut self) -> io::Result<()> {
        self.stop_stream()?;
        // SAFETY: Stop succeeded and no render packet escapes a write call.
        wasapi("flush Windows audio output", unsafe { self.client.Reset() })?;
        self.submitted = 0;
        self.timeline.reset();
        Ok(())
    }

    fn write(&mut self, interleaved: &[i16]) -> io::Result<usize> {
        if !interleaved.len().is_multiple_of(PCM_CHANNELS) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "incomplete stereo PCM frame",
            ));
        }
        let padding = self.padding()?;
        let frames = (interleaved.len() / PCM_CHANNELS)
            .min(self.capacity - padding)
            .min(MAX_DEVICE_FRAMES - self.timeline.outstanding(self.submitted)?);
        if frames == 0 {
            return Ok(0);
        }
        let next = self
            .submitted
            .checked_add(frames as u64)
            .ok_or_else(|| invalid_clock("Windows audio submitted-frame counter overflow"))?;
        // SAFETY: frames fits the observed free capacity. Only this worker writes,
        // and the engine can only free additional space before GetBuffer.
        let data = wasapi("acquire Windows render packet", unsafe {
            self.render.GetBuffer(frames as u32)
        })?;
        if data.is_null() {
            // SAFETY: zero frames returns this shared-mode reservation unused.
            wasapi("return empty Windows render packet", unsafe {
                self.render.ReleaseBuffer(0, 0)
            })?;
            return Err(invalid_clock(
                "Windows audio output returned a null render packet",
            ));
        }
        // SAFETY: the non-null packet holds frames * FRAME_BYTES writable bytes.
        // The input holds at least that many bytes and never borrows native storage.
        // Release commits exactly the copied frames on the thread that acquired them.
        wasapi("submit Windows render packet", unsafe {
            ptr::copy_nonoverlapping(
                interleaved.as_ptr().cast::<u8>(),
                data,
                frames * FRAME_BYTES,
            );
            self.render.ReleaseBuffer(frames as u32, 0)
        })?;
        self.submitted = next;
        if self.state == StreamState::WaitingForPcm {
            self.start_stream()?;
        }
        Ok(frames)
    }

    fn sample_clock(&mut self) -> io::Result<DeviceClock> {
        let padding = self.padding()?;
        let mut position = 0;
        // SAFETY: position is writable, the optional QPC pointer is omitted, and the
        // initialized clock remains live on the owning worker.
        wasapi("read Windows audio stream clock", unsafe {
            self.position.GetPosition(&mut position, None)
        })?;
        self.timeline.observe(position, self.submitted, padding)
    }
}

impl Drop for WasapiSink {
    fn drop(&mut self) {
        // SAFETY: COM and the stream still belong to this thread. Release of all
        // the fields closes the stream even if Stop reports device loss.
        let _ = unsafe { self.client.Stop() };
    }
}

struct PlaybackFence {
    through_frame: u64,
    confirm_at: u64,
}

struct Timeline {
    latency_ticks: u64,
    last_position: u64,
    consumed: u64,
    played: u64,
    pending: VecDeque<PlaybackFence>,
}

impl Timeline {
    fn new(latency_ticks: u64) -> Self {
        Self {
            latency_ticks,
            last_position: 0,
            consumed: 0,
            played: 0,
            pending: VecDeque::new(),
        }
    }

    fn reset(&mut self) {
        self.last_position = 0;
        self.consumed = 0;
        self.played = 0;
        self.pending.clear();
    }

    fn outstanding(&self, submitted: u64) -> io::Result<usize> {
        submitted
            .checked_sub(self.played)
            .and_then(|frames| usize::try_from(frames).ok())
            .filter(|frames| *frames <= MAX_DEVICE_FRAMES)
            .ok_or_else(|| invalid_clock("Windows audio output exceeded the playback queue bound"))
    }

    fn observe(
        &mut self,
        position: u64,
        submitted: u64,
        padding: usize,
    ) -> io::Result<DeviceClock> {
        let consumed = submitted
            .checked_sub(padding as u64)
            .ok_or_else(|| invalid_clock("Windows audio padding exceeds submitted frames"))?;
        if position < self.last_position || consumed < self.consumed {
            return Err(invalid_clock(
                "Windows audio stream clock or consumed frames moved backwards",
            ));
        }
        if consumed > self.consumed {
            // Padding shows when media left the application-facing buffer. Retain
            // those frames through the driver's maximum downstream latency. The
            // native position only ages observed media, including after starvation.
            let confirm_at = position
                .checked_add(self.latency_ticks)
                .ok_or_else(|| invalid_clock("Windows audio stream clock overflow"))?;
            self.pending.push_back(PlaybackFence {
                through_frame: consumed,
                confirm_at,
            });
        }
        while let Some(ready) = self
            .pending
            .front()
            .filter(|fence| fence.confirm_at <= position)
        {
            self.played = ready.through_frame;
            let _ = self.pending.pop_front();
        }
        self.consumed = consumed;
        self.last_position = position;
        Ok(DeviceClock {
            played_frames: self.played,
            queued_frames: self.outstanding(submitted)?,
        })
    }
}

fn invalid_clock(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn wasapi<T>(operation: &'static str, result: WindowsResult<T>) -> io::Result<T> {
    result.map_err(|cause| {
        let kind = match cause.code() {
            AUDCLNT_E_DEVICE_INVALIDATED
            | AUDCLNT_E_RESOURCES_INVALIDATED
            | AUDCLNT_E_SERVICE_NOT_RUNNING
            | AUDCLNT_E_ENDPOINT_CREATE_FAILED => io::ErrorKind::NotConnected,
            code if code == HRESULT::from_win32(ERROR_NOT_FOUND.0) => io::ErrorKind::NotConnected,
            AUDCLNT_E_DEVICE_IN_USE => io::ErrorKind::ResourceBusy,
            AUDCLNT_E_UNSUPPORTED_FORMAT => io::ErrorKind::Unsupported,
            E_ACCESSDENIED => io::ErrorKind::PermissionDenied,
            _ => io::ErrorKind::Other,
        };
        io::Error::new(kind, format!("{operation}: {cause}"))
    })
}

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
