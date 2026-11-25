use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
use midnight_media_host::{
    AudioPause, AudioSink, DeviceClock, MAX_DEVICE_FRAMES, MAX_PCM_BLOCK_FRAMES, PCM_CHANNELS,
    PCM_RATE,
};
use std::{
    ffi::{CStr, c_char, c_int, c_long, c_uint, c_ulong, c_void},
    fs, io,
    ptr::{self, NonNull},
};

pub(super) fn open_default() -> io::Result<Box<dyn AudioSink>> {
    AlsaSink::open(c"default").map(|sink| Box::new(sink) as Box<dyn AudioSink>)
}

// Public ALSA PCM ABI values. S16 follows the CPU byte order of the host slice.
const STREAM_PLAYBACK: c_int = 0;
const NONBLOCK: c_int = 1;
const RW_INTERLEAVED: c_int = 3;
const S16: c_int = if cfg!(target_endian = "little") { 2 } else { 3 };
const STATE_SETUP: c_int = 1;
const STATE_PREPARED: c_int = 2;
const STATE_RUNNING: c_int = 3;
const STATE_XRUN: c_int = 4;
const STATE_PAUSED: c_int = 6;
const STATE_SUSPENDED: c_int = 7;
const STATE_DISCONNECTED: c_int = 8;
const LATENCY_MICROS: c_uint = 100_000;

const SYSTEM_LIBASOUND: &[&str] = &[
    #[cfg(target_arch = "x86_64")]
    "/usr/lib/x86_64-linux-gnu/libasound.so.2",
    #[cfg(target_arch = "x86_64")]
    "/lib/x86_64-linux-gnu/libasound.so.2",
    #[cfg(target_arch = "aarch64")]
    "/usr/lib/aarch64-linux-gnu/libasound.so.2",
    #[cfg(target_arch = "aarch64")]
    "/lib/aarch64-linux-gnu/libasound.so.2",
    "/usr/lib64/libasound.so.2",
    "/lib64/libasound.so.2",
    "/usr/lib/libasound.so.2",
    "/lib/libasound.so.2",
];

type Handle = *mut c_void;
type PcmControl = unsafe extern "C" fn(Handle) -> c_int;
type Allocate = unsafe extern "C" fn(*mut Handle) -> c_int;
type Free = unsafe extern "C" fn(Handle);

struct Alsa {
    _library: Library,
    open: unsafe extern "C" fn(*mut Handle, *const c_char, c_int, c_int) -> c_int,
    close: PcmControl,
    prepare: PcmControl,
    drop_stream: PcmControl,
    pause: unsafe extern "C" fn(Handle, c_int) -> c_int,
    set_params: unsafe extern "C" fn(Handle, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
    get_params: unsafe extern "C" fn(Handle, *mut c_ulong, *mut c_ulong) -> c_int,
    sw_malloc: Allocate,
    sw_free: Free,
    sw_current: unsafe extern "C" fn(Handle, Handle) -> c_int,
    sw_start_threshold: unsafe extern "C" fn(Handle, Handle, c_ulong) -> c_int,
    sw_stop_threshold: unsafe extern "C" fn(Handle, Handle, c_ulong) -> c_int,
    sw_apply: unsafe extern "C" fn(Handle, Handle) -> c_int,
    status_malloc: Allocate,
    status_free: Free,
    status: unsafe extern "C" fn(Handle, Handle) -> c_int,
    status_state: unsafe extern "C" fn(*const c_void) -> c_int,
    status_delay: unsafe extern "C" fn(*const c_void) -> c_long,
    status_available: unsafe extern "C" fn(*const c_void) -> c_ulong,
    write_interleaved: unsafe extern "C" fn(Handle, *const c_void, c_ulong) -> c_long,
}

impl Alsa {
    fn load() -> io::Result<Self> {
        let library = system_library()?;
        // SAFETY: The supported libasound SONAME supplies these public C signatures.
        // The copied function pointers remain private to the owning library.
        unsafe {
            Ok(Self {
                open: symbol(&library, "snd_pcm_open")?,
                close: symbol(&library, "snd_pcm_close")?,
                prepare: symbol(&library, "snd_pcm_prepare")?,
                drop_stream: symbol(&library, "snd_pcm_drop")?,
                pause: symbol(&library, "snd_pcm_pause")?,
                set_params: symbol(&library, "snd_pcm_set_params")?,
                get_params: symbol(&library, "snd_pcm_get_params")?,
                sw_malloc: symbol(&library, "snd_pcm_sw_params_malloc")?,
                sw_free: symbol(&library, "snd_pcm_sw_params_free")?,
                sw_current: symbol(&library, "snd_pcm_sw_params_current")?,
                sw_start_threshold: symbol(&library, "snd_pcm_sw_params_set_start_threshold")?,
                sw_stop_threshold: symbol(&library, "snd_pcm_sw_params_set_stop_threshold")?,
                sw_apply: symbol(&library, "snd_pcm_sw_params")?,
                status_malloc: symbol(&library, "snd_pcm_status_malloc")?,
                status_free: symbol(&library, "snd_pcm_status_free")?,
                status: symbol(&library, "snd_pcm_status")?,
                status_state: symbol(&library, "snd_pcm_status_get_state")?,
                status_delay: symbol(&library, "snd_pcm_status_get_delay")?,
                status_available: symbol(&library, "snd_pcm_status_get_avail")?,
                write_interleaved: symbol(&library, "snd_pcm_writei")?,
                _library: library,
            })
        }
    }
}

fn system_library() -> io::Result<Library> {
    for path in SYSTEM_LIBASOUND {
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {
                // SAFETY: The PCM adapter loads the system SONAME from fixed OS library paths.
                return unsafe { Library::open(Some(path), RTLD_NOW | RTLD_LOCAL) }.map_err(
                    |failure| {
                        io::Error::other(format!(
                            "Loading system ALSA from {path} failed. {failure}"
                        ))
                    },
                );
            }
            Ok(_) => continue,
            Err(failure) if failure.kind() == io::ErrorKind::NotFound => continue,
            Err(failure) => return Err(failure),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "System ALSA libasound.so.2 is unavailable",
    ))
}

unsafe fn symbol<T: Copy>(library: &Library, name: &str) -> io::Result<T> {
    // SAFETY: The caller matches T to the named ALSA C export and retains the library.
    unsafe { library.get::<T>(name.as_bytes()) }
        .map(|entry| *entry)
        .map_err(|failure| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("System ALSA does not provide {name}. {failure}"),
            )
        })
}

#[derive(Clone, Copy)]
struct PcmStatus {
    state: c_int,
    delay: c_long,
    available: c_ulong,
}

/// The PCM calls the sink's state machine makes: libasound on a device, a recorded
/// device model in tests. Results are the native status codes.
trait Pcm {
    fn status(&mut self) -> io::Result<PcmStatus>;
    fn prepare(&mut self) -> c_int;
    fn drop_stream(&mut self) -> c_int;
    fn pause(&mut self, enable: bool) -> c_int;
    /// Accepted frames of `interleaved`, or a negative error.
    fn write(&mut self, interleaved: &[i16]) -> c_long;
}

#[derive(Default)]
enum DelayEvidence {
    #[default]
    Unobserved,
    RingOnly,
    Downstream,
}

impl DelayEvidence {
    fn observe(&mut self, ring_queued: c_ulong, delay: c_ulong) {
        if delay > ring_queued {
            *self = Self::Downstream;
        } else if delay == ring_queued && matches!(self, Self::Unobserved) {
            *self = Self::RingOnly;
        }
    }

    fn exhausted(&self, status: PcmStatus, buffer_frames: c_ulong) -> bool {
        // Linux can supply a synthetic zero delay when a device is stopped.
        // Confirm an empty ring and use delay evidence from its running state.
        status.state == STATE_XRUN
            && status.available >= buffer_frames
            && (status.delay < 0 || (status.delay == 0 && matches!(self, Self::RingOnly)))
    }
}

/// Media frames since the last reset: accepted by the device, confirmed played, and
/// retired from the device queue by an underrun.
#[derive(Default)]
struct Playout {
    submitted: u64,
    played: u64,
    retired: u64,
}

impl Playout {
    fn clock(&self) -> DeviceClock {
        DeviceClock {
            played_frames: self.played,
            queued_frames: (self.submitted - self.played.max(self.retired)) as usize,
        }
    }

    fn submit(&mut self, accepted: usize) -> io::Result<()> {
        self.submitted = self
            .submitted
            .checked_add(accepted as u64)
            .ok_or_else(|| invalid("ALSA submitted frame counter overflow"))?;
        Ok(())
    }

    fn observe_delay(&mut self, delay: c_long) -> io::Result<DeviceClock> {
        let known_delay = u64::try_from(delay)
            .map_err(|_| invalid("ALSA returned a negative running playback delay"))?;
        self.played = self.played.max(self.submitted.saturating_sub(known_delay));
        Ok(self.clock())
    }

    /// An underrun ends the device queue. Each submitted frame has played, is still
    /// downstream, or stays in the ring until prepare discards it: PipeWire stops with
    /// up to a graph quantum there and repeats a stale delay. Only proof of an empty
    /// output path counts these frames as played. Otherwise the clock keeps its last
    /// observation, and the running delay after the restart passes the lost frames only
    /// when later media plays.
    fn underrun(&mut self, confirmed_empty: bool) {
        if confirmed_empty {
            self.played = self.submitted;
        }
        self.retired = self.submitted;
    }
}

#[derive(Clone, Copy)]
enum Output {
    Stopped,
    Armed,
    Paused,
}

/// A libasound playback handle with its status buffer. It owns both until Drop.
struct NativePcm {
    api: Alsa,
    pcm: NonNull<c_void>,
    status: NonNull<c_void>,
}

impl NativePcm {
    /// Open `device` for nonblocking 48 kHz stereo playback; return it with its ring
    /// buffer size in frames.
    fn open(device: &CStr) -> io::Result<(Self, c_ulong)> {
        let api = Alsa::load()?;
        let status = allocate(api.status_malloc, "allocate PCM status")?;
        let mut handle = ptr::null_mut();
        // SAFETY: The output address and device C string live through this synchronous call.
        let opened = unsafe { (api.open)(&mut handle, device.as_ptr(), STREAM_PLAYBACK, NONBLOCK) };
        let pcm = match checked("open playback device", opened).and_then(|()| {
            NonNull::new(handle).ok_or_else(|| invalid("ALSA opened a null playback handle"))
        }) {
            Ok(pcm) => pcm,
            Err(failure) => {
                // SAFETY: This status was allocated by this library and is still owned here.
                unsafe { (api.status_free)(status.as_ptr()) };
                return Err(failure);
            }
        };
        let mut native = Self { api, pcm, status };
        let buffer_frames = native.configure()?;
        Ok((native, buffer_frames))
    }

    fn configure(&mut self) -> io::Result<c_ulong> {
        let pcm = self.pcm.as_ptr();
        let mut buffer_frames = 0;
        let mut period_frames = 0;
        // SAFETY: The sink owns the configured PCM and gives ALSA correctly typed outputs.
        unsafe {
            checked(
                "configure 48 kHz stereo PCM",
                (self.api.set_params)(
                    pcm,
                    S16,
                    RW_INTERLEAVED,
                    PCM_CHANNELS as c_uint,
                    PCM_RATE,
                    1,
                    LATENCY_MICROS,
                ),
            )?;
            checked(
                "read configured buffer size",
                (self.api.get_params)(pcm, &mut buffer_frames, &mut period_frames),
            )?;
        }
        if buffer_frames == 0 || period_frames == 0 {
            return Err(invalid("ALSA configured an empty PCM buffer or period"));
        }
        let params = allocate(self.api.sw_malloc, "allocate PCM software parameters")?;
        let params = params.as_ptr();
        // SAFETY: The parameter object and PCM are live and exclusive to this worker.
        let configure = unsafe {
            checked(
                "read PCM software parameters",
                (self.api.sw_current)(pcm, params),
            )
            // Start on admitted media, including very short final blocks.
            .and_then(|()| {
                checked(
                    "set start threshold",
                    (self.api.sw_start_threshold)(pcm, params, 1),
                )
            })
            .and_then(|()| {
                checked(
                    "set underrun threshold",
                    (self.api.sw_stop_threshold)(pcm, params, buffer_frames),
                )
            })
            .and_then(|()| {
                checked(
                    "apply PCM software parameters",
                    (self.api.sw_apply)(pcm, params),
                )
            })
        };
        // SAFETY: Release the still-owned parameter object after every configure outcome.
        unsafe { (self.api.sw_free)(params) };
        configure?;
        Ok(buffer_frames)
    }
}

impl Pcm for NativePcm {
    fn status(&mut self) -> io::Result<PcmStatus> {
        let status = self.status.as_ptr();
        // SAFETY: The live worker owns the PCM and its status buffer through this call.
        unsafe {
            checked(
                "sample playback status",
                (self.api.status)(self.pcm.as_ptr(), status),
            )?;
            Ok(PcmStatus {
                state: (self.api.status_state)(status),
                delay: (self.api.status_delay)(status),
                available: (self.api.status_available)(status),
            })
        }
    }

    fn prepare(&mut self) -> c_int {
        // SAFETY: This value owns the live PCM handle; prepare restores its configured state.
        unsafe { (self.api.prepare)(self.pcm.as_ptr()) }
    }

    fn drop_stream(&mut self) -> c_int {
        // SAFETY: Drop stops the owned live PCM and discards its buffer immediately.
        unsafe { (self.api.drop_stream)(self.pcm.as_ptr()) }
    }

    fn pause(&mut self, enable: bool) -> c_int {
        // SAFETY: This value owns the live PCM handle; ALSA refuses a pause in a wrong state.
        unsafe { (self.api.pause)(self.pcm.as_ptr(), c_int::from(enable)) }
    }

    fn write(&mut self, interleaved: &[i16]) -> c_long {
        // SAFETY: The live device is nonblocking, and the input holds every requested frame.
        // snd_pcm_writei copies only the reported accepted frames during the call.
        unsafe {
            (self.api.write_interleaved)(
                self.pcm.as_ptr(),
                interleaved.as_ptr().cast(),
                (interleaved.len() / PCM_CHANNELS) as c_ulong,
            )
        }
    }
}

impl Drop for NativePcm {
    fn drop(&mut self) {
        // SAFETY: Retire the live PCM and its independent status allocation before
        // the library field is unloaded. Every native resource has one Rust owner.
        unsafe {
            (self.api.drop_stream)(self.pcm.as_ptr());
            (self.api.close)(self.pcm.as_ptr());
            (self.api.status_free)(self.status.as_ptr());
        }
    }
}

struct AlsaSink<P = NativePcm> {
    pcm: P,
    buffer_frames: c_ulong,
    output: Output,
    playout: Playout,
    delay: DelayEvidence,
}

impl AlsaSink {
    fn open(device: &CStr) -> io::Result<Self> {
        let (pcm, buffer_frames) = NativePcm::open(device)?;
        let mut sink = Self::new(pcm, buffer_frames);
        let state = sink.pcm.status()?.state;
        if state != STATE_PREPARED {
            return Err(state_error("prepare default playback", state));
        }
        Ok(sink)
    }
}

impl<P: Pcm> AlsaSink<P> {
    fn new(pcm: P, buffer_frames: c_ulong) -> Self {
        Self {
            pcm,
            buffer_frames,
            output: Output::Stopped,
            playout: Playout::default(),
            delay: DelayEvidence::default(),
        }
    }

    fn observe(&mut self, status: PcmStatus) -> io::Result<DeviceClock> {
        match (self.output, status.state) {
            (Output::Stopped | Output::Armed, STATE_PREPARED) | (Output::Paused, STATE_PAUSED) => {
                Ok(self.playout.clock())
            }
            (Output::Armed, STATE_RUNNING) => {
                let ring_queued = self
                    .buffer_frames
                    .checked_sub(status.available)
                    .ok_or_else(|| invalid("ALSA playback availability exceeds its buffer"))?;
                let delay = c_ulong::try_from(status.delay)
                    .map_err(|_| invalid("ALSA returned a negative running playback delay"))?;
                if delay < ring_queued {
                    return Err(invalid("ALSA playback delay omits queued PCM frames"));
                }
                self.delay.observe(ring_queued, delay);
                self.playout.observe_delay(status.delay)
            }
            (Output::Armed, STATE_XRUN) => self.recover(status),
            _ => Err(state_error("sample playback clock", status.state)),
        }
    }

    /// Retire the queue an underrun ended and re-arm at once: waiting cannot prove more,
    /// since the stopped device's status no longer changes.
    fn recover(&mut self, status: PcmStatus) -> io::Result<DeviceClock> {
        let empty = self.delay.exhausted(status, self.buffer_frames);
        self.playout.underrun(empty);
        self.delay = DelayEvidence::default();
        checked("recover playback underrun", self.pcm.prepare())?;
        self.output = Output::Armed;
        Ok(self.playout.clock())
    }
}

impl<P: Pcm> AudioSink for AlsaSink<P> {
    fn start(&mut self) -> io::Result<()> {
        let expected = match self.output {
            Output::Armed => return Ok(()),
            Output::Stopped => STATE_PREPARED,
            Output::Paused => STATE_PAUSED,
        };
        let actual = self.pcm.status()?.state;
        if actual != expected {
            return Err(state_error("start playback", actual));
        }
        if expected == STATE_PAUSED {
            checked("resume playback", self.pcm.pause(false))?;
        }
        self.output = Output::Armed;
        Ok(())
    }

    fn pause(&mut self) -> io::Result<AudioPause> {
        let status = self.pcm.status()?;
        let last = self.observe(status)?;
        if status.state == STATE_PAUSED {
            return Ok(AudioPause::Retained(last));
        }
        if matches!(status.state, STATE_PREPARED | STATE_XRUN) && last.queued_frames == 0 {
            self.output = Output::Stopped;
            return Ok(AudioPause::Retained(last));
        }
        if status.state == STATE_RUNNING {
            let native = self.pcm.pause(true);
            if native >= 0 && self.pcm.status()?.state == STATE_PAUSED {
                // Linux status can report zero delay for a paused PCM. Freeze the
                // last running observation until fresh progress is sampled on resume.
                self.output = Output::Paused;
                return Ok(AudioPause::Retained(last));
            }
        }
        self.reset()?;
        Ok(AudioPause::Discarded(DeviceClock {
            queued_frames: 0,
            ..last
        }))
    }

    fn reset(&mut self) -> io::Result<()> {
        // A prior reset may already have discarded the stream before prepare failed.
        let discarded = self
            .pcm
            .status()
            .is_ok_and(|status| status.state == STATE_SETUP);
        if !discarded {
            checked("discard playback buffer", self.pcm.drop_stream())?;
        }
        self.output = Output::Stopped;
        self.playout = Playout::default();
        self.delay = DelayEvidence::default();
        checked("prepare playback after flush", self.pcm.prepare())
    }

    fn write(&mut self, interleaved: &[i16]) -> io::Result<usize> {
        if !interleaved.len().is_multiple_of(PCM_CHANNELS) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ALSA input has an incomplete stereo frame",
            ));
        }
        if !matches!(self.output, Output::Armed) {
            return Ok(0);
        }
        let frames = (interleaved.len() / PCM_CHANNELS)
            .min(MAX_PCM_BLOCK_FRAMES)
            .min(MAX_DEVICE_FRAMES - self.playout.clock().queued_frames);
        if frames == 0 {
            return Ok(0);
        }
        let accepted = self.pcm.write(&interleaved[..frames * PCM_CHANNELS]);
        if accepted < 0 {
            let failure = native_error(
                "write PCM",
                c_int::try_from(accepted)
                    .map_err(|_| invalid("ALSA returned an invalid write status"))?,
            );
            if temporary(&failure) {
                return Ok(0);
            }
            if failure.kind() == io::ErrorKind::BrokenPipe {
                let status = self.pcm.status()?;
                if status.state == STATE_XRUN {
                    self.recover(status)?;
                    return Ok(0);
                }
            }
            return Err(failure);
        }
        let accepted = usize::try_from(accepted)
            .map_err(|_| invalid("ALSA returned an invalid accepted-frame count"))?;
        if accepted > frames {
            return Err(invalid("ALSA accepted more frames than supplied"));
        }
        self.playout.submit(accepted)?;
        Ok(accepted)
    }

    fn sample_clock(&mut self) -> io::Result<DeviceClock> {
        match self.pcm.status() {
            Ok(status) => self.observe(status),
            Err(failure) if temporary(&failure) => Ok(self.playout.clock()),
            Err(failure) => Err(failure),
        }
    }
}

fn allocate(function: Allocate, operation: &str) -> io::Result<NonNull<c_void>> {
    let mut pointer = ptr::null_mut();
    // SAFETY: Both configured ALSA allocators take the address of an output pointer.
    checked(operation, unsafe { function(&mut pointer) })?;
    NonNull::new(pointer).ok_or_else(|| invalid("ALSA returned a null allocation"))
}

fn checked(operation: &str, status: c_int) -> io::Result<()> {
    if status < 0 {
        Err(native_error(operation, status))
    } else {
        Ok(())
    }
}

fn native_error(operation: &str, status: c_int) -> io::Error {
    let failure = io::Error::from_raw_os_error(status.saturating_neg());
    io::Error::new(
        failure.kind(),
        format!("ALSA could not {operation}. {failure}"),
    )
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn state_error(operation: &str, state: c_int) -> io::Error {
    match state {
        STATE_DISCONNECTED => io::Error::new(
            io::ErrorKind::NotConnected,
            "ALSA playback device disconnected",
        ),
        STATE_SUSPENDED => io::Error::other("ALSA playback device suspended"),
        _ => io::Error::new(
            io::ErrorKind::InvalidData,
            format!("ALSA could not {operation} in PCM state {state}"),
        ),
    }
}

fn temporary(failure: &io::Error) -> bool {
    matches!(
        failure.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
