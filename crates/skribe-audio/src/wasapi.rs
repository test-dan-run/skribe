//! The Windows capture backend: WASAPI in shared mode.
//!
//! The microphone is a recording device. System audio is a playback device opened for
//! loopback, which delivers what that device plays and nothing while nothing is played. Which
//! device each one is follows the caller's choice: the Windows default, the Windows default
//! communications device, or one specific device. Each stream runs on its own thread, which
//! owns every COM object it creates, and reads the device in its native mix format.
//!
//! This is the only module in the workspace that uses `unsafe`, and the only one WSL cannot
//! compile into a test. It is kept to the calls Windows requires; everything it delivers goes
//! through [`CaptureSink`], which is tested without a device.
#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioCaptureClient,
    IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eCapture, eCommunications, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

use crate::capture::{
    BackendInfo, CaptureBackend, CaptureSink, CaptureStream, DeviceChoice, DeviceInfo,
    ListedDevice, choose,
};
use crate::clock::flags;
use crate::{AudioFormat, CaptureError, CaptureErrorKind, SampleKind, SessionClock, Source};

/// How often a capture thread collects what the device has buffered.
const POLL_INTERVAL: Duration = Duration::from_millis(10);
/// The device-side buffer asked for, in 100-nanosecond units: one second, so a late poll
/// loses nothing.
const DEVICE_BUFFER_HNS: i64 = 10_000_000;

const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x0002_0000;
const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: u32 = 0x1;
const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;
const AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR: u32 = 0x4;

const E_ACCESSDENIED: u32 = 0x8007_0005;
/// `HRESULT_FROM_WIN32(ERROR_NOT_FOUND)`: there is no default device for the data flow.
const E_NOTFOUND: u32 = 0x8007_0490;
const AUDCLNT_E_DEVICE_INVALIDATED: u32 = 0x8889_0004;
const AUDCLNT_E_SERVICE_NOT_RUNNING: u32 = 0x8889_0010;
const AUDCLNT_E_RESOURCES_INVALIDATED: u32 = 0x8889_0026;

const WAVE_FORMAT_PCM: u32 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u32 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// The fields shared by every `KSDATAFORMAT_SUBTYPE_*` wave format identifier.
const SUBFORMAT_DATA4: [u8; 8] = [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WasapiBackend;

impl CaptureBackend for WasapiBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "Windows audio (WASAPI, shared mode)",
            synthetic: false,
        }
    }

    fn devices(&self, source: Source) -> Result<Vec<ListedDevice>, CaptureError> {
        // COM objects stay on the thread that creates them, so the list is read on a thread
        // of its own and only plain data comes back.
        thread::Builder::new()
            .name(format!("skribe-devices-{source}"))
            .spawn(move || list_devices(source))
            .map_err(|error| device_error(source, error.to_string()))?
            .join()
            .unwrap_or_else(|_| {
                Err(device_error(
                    source,
                    "the thread listing devices ended unexpectedly",
                ))
            })
    }

    fn open(
        &self,
        source: Source,
        choice: &DeviceChoice,
    ) -> Result<Box<dyn CaptureStream>, CaptureError> {
        let choice = choice.clone();
        let (opened_tx, opened_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name(format!("skribe-capture-{source}"))
            .spawn(move || {
                capture_thread(
                    source,
                    &choice,
                    &opened_tx,
                    &start_rx,
                    &started_tx,
                    &thread_stop,
                );
            })
            .map_err(|error| device_error(source, error.to_string()))?;

        match opened_rx.recv() {
            Ok(Ok((device, format))) => Ok(Box::new(WasapiStream {
                source,
                device,
                format,
                start: Some(start_tx),
                started: started_rx,
                stop,
                thread: Some(thread),
            })),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(device_error(
                    source,
                    "the capture thread ended before it opened the device",
                ))
            }
        }
    }
}

struct WasapiStream {
    source: Source,
    device: DeviceInfo,
    format: AudioFormat,
    /// Hands the sink to the capture thread. Dropping it tells a thread that was never
    /// started to close the device and end.
    start: Option<Sender<CaptureSink>>,
    started: Receiver<Result<(), CaptureError>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WasapiStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasapiStream")
            .field("source", &self.source)
            .field("device", &self.device)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl CaptureStream for WasapiStream {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn format(&self) -> AudioFormat {
        self.format
    }

    fn start(&mut self, sink: CaptureSink) -> Result<(), CaptureError> {
        let gone = || device_error(self.source, "the capture thread has ended");
        let start = self.start.take().ok_or_else(gone)?;
        start.send(sink).map_err(|_| gone())?;
        self.started.recv().map_err(|_| gone())?
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.start.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WasapiStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn device_error(source: Source, detail: impl Into<String>) -> CaptureError {
    CaptureError::new(source, CaptureErrorKind::Device, detail)
}

/// The unsigned form of an error's `HRESULT`, as Windows documents its codes.
fn code_of(error: &windows::core::Error) -> u32 {
    error.code().0 as u32
}

/// Names the step that failed and the Windows error code. No device data is included.
fn windows_error(source: Source, step: &str, error: &windows::core::Error) -> CaptureError {
    let kind = match code_of(error) {
        E_ACCESSDENIED => CaptureErrorKind::AccessDenied,
        E_NOTFOUND => CaptureErrorKind::NoDevice,
        _ => CaptureErrorKind::Device,
    };
    CaptureError::new(
        source,
        kind,
        format!(
            "{step} failed with 0x{:08X}: {}",
            code_of(error),
            error.message()
        ),
    )
}

/// An opened device, alive only on the thread that created it.
struct Opened {
    source: Source,
    client: IAudioClient,
    capture: IAudioCaptureClient,
    device: DeviceInfo,
    format: AudioFormat,
}

/// Starts COM on the calling thread for the life of the returned value.
struct ComThread;

impl ComThread {
    fn start(source: Source) -> Result<Self, CaptureError> {
        // SAFETY: COM is initialised here for this thread only. The matching uninitialise is
        // in `Drop`, which runs on the same thread because the value is not `Send`-ed
        // anywhere: every use creates it and drops it inside one function.
        let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if initialised.is_err() {
            return Err(device_error(
                source,
                format!("starting COM failed with 0x{:08X}", initialised.0 as u32),
            ));
        }
        Ok(Self)
    }
}

impl Drop for ComThread {
    fn drop(&mut self) {
        // SAFETY: balances the successful `CoInitializeEx` in `start`. Callers declare the
        // `ComThread` before any COM object, so those objects have been dropped by now.
        unsafe { CoUninitialize() };
    }
}

fn list_devices(source: Source) -> Result<Vec<ListedDevice>, CaptureError> {
    let _com = ComThread::start(source)?;
    let endpoints = endpoints(source)?;
    Ok(endpoints.into_iter().map(|(_, listed)| listed).collect())
}

/// The active devices for `source`, each with the COM object that opens it.
fn endpoints(source: Source) -> Result<Vec<(IMMDevice, ListedDevice)>, CaptureError> {
    let failed = |step: &'static str| move |error| windows_error(source, step, &error);
    let flow = match source {
        Source::Microphone => eCapture,
        Source::System => eRender,
    };
    // SAFETY: COM is initialised on this thread by the caller. The class and interface
    // identifiers are the ones the Windows SDK defines for the device enumerator.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(failed("creating the device enumerator"))?;
    let default = default_id(&enumerator, flow, eConsole);
    let communications = default_id(&enumerator, flow, eCommunications);

    // SAFETY: `enumerator` is a live COM object owned by this thread.
    let collection = unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }
        .map_err(failed("listing devices"))?;
    // SAFETY: `collection` is live.
    let count = unsafe { collection.GetCount() }.map_err(failed("counting devices"))?;
    let mut listed = Vec::with_capacity(count as usize);
    for index in 0..count {
        // SAFETY: `index` is below the count the collection just reported.
        let endpoint = unsafe { collection.Item(index) }.map_err(failed("reading a device"))?;
        let id = device_id(&endpoint).map_err(failed("reading a device identifier"))?;
        let device = ListedDevice {
            is_default: default.as_deref() == Some(id.as_str()),
            is_communications_default: communications.as_deref() == Some(id.as_str()),
            device: DeviceInfo {
                name: device_name(&endpoint).unwrap_or_else(|| "Unnamed device".to_owned()),
                id,
            },
        };
        listed.push((endpoint, device));
    }
    Ok(listed)
}

/// The identifier of the default device for a role, when there is one.
fn default_id(enumerator: &IMMDeviceEnumerator, flow: EDataFlow, role: ERole) -> Option<String> {
    // SAFETY: `enumerator` is a live COM object owned by this thread.
    let endpoint = unsafe { enumerator.GetDefaultAudioEndpoint(flow, role) }.ok()?;
    device_id(&endpoint).ok()
}

fn capture_thread(
    source: Source,
    choice: &DeviceChoice,
    opened: &Sender<Result<(DeviceInfo, AudioFormat), CaptureError>>,
    start: &Receiver<CaptureSink>,
    started: &Sender<Result<(), CaptureError>>,
    stop: &AtomicBool,
) {
    // Declared first, so it is dropped last: after every COM object below.
    let _com = match ComThread::start(source) {
        Ok(com) => com,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    match open(source, choice) {
        Err(error) => {
            let _ = opened.send(Err(error));
        }
        Ok(device) => {
            let _ = opened.send(Ok((device.device.clone(), device.format)));
            // An error here means the stream was dropped without being started.
            if let Ok(sink) = start.recv() {
                run(&device, &sink, started, stop);
            }
        }
    }
}

fn open(source: Source, choice: &DeviceChoice) -> Result<Opened, CaptureError> {
    let failed = |step: &'static str| move |error| windows_error(source, step, &error);

    // The chosen device is found among the active devices of the right kind, by identifier.
    // Nothing the caller supplies is handed to Windows as a device path.
    let mut endpoints = endpoints(source)?;
    let listed: Vec<ListedDevice> = endpoints.iter().map(|(_, device)| device.clone()).collect();
    let (endpoint, chosen) = endpoints.swap_remove(choose(source, &listed, choice)?);
    let device = chosen.device;

    // SAFETY: `endpoint` is a live COM object; no activation parameters are passed.
    let client: IAudioClient =
        unsafe { endpoint.Activate(CLSCTX_ALL, None) }.map_err(failed("opening the device"))?;
    // SAFETY: `client` is live. On success the returned pointer is a `WAVEFORMATEX`, possibly
    // extended, allocated by Windows; it is freed below on every path.
    let mix = unsafe { client.GetMixFormat() }.map_err(failed("reading the device format"))?;
    // SAFETY: `mix` came from `GetMixFormat` and has not been freed.
    let format = unsafe { read_format(mix) };
    let stream_flags = match source {
        Source::Microphone => 0,
        Source::System => AUDCLNT_STREAMFLAGS_LOOPBACK,
    };
    let initialised = match &format {
        // SAFETY: `client` is live and `mix` is the device's own mix format, unmodified.
        Ok(_) => unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                stream_flags,
                DEVICE_BUFFER_HNS,
                0,
                mix,
                None,
            )
        },
        Err(_) => Ok(()),
    };
    // SAFETY: `mix` was allocated by `GetMixFormat` with the COM allocator and is not used
    // after this line.
    unsafe { CoTaskMemFree(Some(mix.cast_const().cast())) };
    let format = format
        .map_err(|detail| CaptureError::new(source, CaptureErrorKind::FormatUnsupported, detail))?;
    initialised.map_err(failed("preparing the device for capture"))?;

    // SAFETY: `client` is live and initialised, which `GetService` requires.
    let capture: IAudioCaptureClient =
        unsafe { client.GetService() }.map_err(failed("opening the capture service"))?;
    Ok(Opened {
        source,
        client,
        capture,
        device,
        format,
    })
}

fn device_id(endpoint: &IMMDevice) -> windows::core::Result<String> {
    // SAFETY: `endpoint` is live. On success `GetId` returns a null-terminated string
    // allocated by Windows, which is read once and then freed with the COM allocator.
    unsafe {
        let id = endpoint.GetId()?;
        let text = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.as_ptr().cast_const().cast()));
        Ok(text)
    }
}

/// The name Windows shows for the device, when it has one.
fn device_name(endpoint: &IMMDevice) -> Option<String> {
    // SAFETY: `endpoint` is live. The property store and the value are COM-managed and are
    // released when they go out of scope.
    let name = unsafe {
        let store = endpoint.OpenPropertyStore(STGM_READ).ok()?;
        store.GetValue(&PKEY_Device_FriendlyName).ok()?
    };
    let text = name.to_string();
    (!text.is_empty()).then_some(text)
}

/// Reads the mix format Windows returned.
///
/// # Safety
///
/// `mix` must point to a `WAVEFORMATEX` returned by `IAudioClient::GetMixFormat` that has not
/// been freed. When its tag is `WAVE_FORMAT_EXTENSIBLE` and `cbSize` is at least 22, Windows
/// guarantees the allocation holds a full `WAVEFORMATEXTENSIBLE`.
unsafe fn read_format(mix: *const WAVEFORMATEX) -> Result<AudioFormat, String> {
    // SAFETY: guaranteed by the caller. The struct is packed, so it is read unaligned.
    let base = unsafe { mix.read_unaligned() };
    let (tag, bits, channels, rate) = (
        base.wFormatTag,
        base.wBitsPerSample,
        base.nChannels,
        base.nSamplesPerSec,
    );
    let (kind_tag, valid_bits, channel_mask) = if tag == WAVE_FORMAT_EXTENSIBLE && base.cbSize >= 22
    {
        // SAFETY: guaranteed by the caller for this tag and size.
        let extended = unsafe { mix.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
        let subformat = extended.SubFormat;
        if subformat.data2 != 0 || subformat.data3 != 0x0010 || subformat.data4 != SUBFORMAT_DATA4 {
            return Err(format!("the device's sample encoding {subformat:?}"));
        }
        // SAFETY: every member of the union is a `u16`, so any of them can be read.
        let valid_bits = unsafe { extended.Samples.wValidBitsPerSample };
        (subformat.data1, valid_bits, extended.dwChannelMask)
    } else {
        (u32::from(tag), bits, 0)
    };
    let kind = match kind_tag {
        WAVE_FORMAT_PCM => SampleKind::Int,
        WAVE_FORMAT_IEEE_FLOAT => SampleKind::Float,
        other => return Err(format!("the device's sample encoding (format tag {other})")),
    };
    let format = AudioFormat {
        sample_rate: rate,
        channels,
        bits_per_sample: bits,
        // Some drivers report zero valid bits to mean all of them.
        valid_bits: if valid_bits == 0 { bits } else { valid_bits },
        kind,
        channel_mask,
    };
    format.validate().map_err(|error| error.to_string())?;
    if u32::from(base.nBlockAlign) != format.bytes_per_frame() {
        return Err(format!("a frame size of {} bytes for {format}", {
            base.nBlockAlign
        }));
    }
    Ok(format)
}

/// Converts device timestamps, which count 100-nanosecond units on the performance counter,
/// to microseconds on the recording's timeline. `Instant` on Windows reads the same counter,
/// so one paired reading of both fixes the offset between them.
struct Anchor {
    counter_hns: i64,
    timeline_us: i64,
}

impl Anchor {
    fn take(clock: SessionClock) -> Option<Self> {
        let (mut frequency, mut counter) = (0i64, 0i64);
        // SAFETY: both calls only write to the `i64` they are given.
        unsafe {
            QueryPerformanceFrequency(&mut frequency).ok()?;
            QueryPerformanceCounter(&mut counter).ok()?;
        }
        let timeline_us = clock.now_us();
        if frequency <= 0 {
            return None;
        }
        let counter_hns = i128::from(counter) * 10_000_000 / i128::from(frequency);
        Some(Self {
            counter_hns: i64::try_from(counter_hns).ok()?,
            timeline_us,
        })
    }

    fn timeline_us(&self, position_hns: u64) -> i64 {
        let position = i64::try_from(position_hns).unwrap_or(i64::MAX);
        self.timeline_us + (position - self.counter_hns) / 10
    }
}

fn run(
    device: &Opened,
    sink: &CaptureSink,
    started: &Sender<Result<(), CaptureError>>,
    stop: &AtomicBool,
) {
    let source = device.source;
    let Some(anchor) = Anchor::take(sink.clock()) else {
        let _ = started.send(Err(device_error(
            source,
            "the performance counter could not be read",
        )));
        return;
    };
    // SAFETY: `client` is live and initialised on this thread.
    if let Err(error) = unsafe { device.client.Start() } {
        let _ = started.send(Err(windows_error(source, "starting capture", &error)));
        return;
    }
    let _ = started.send(Ok(()));

    let mut silence = Vec::new();
    let mut ended_by_device = false;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(POLL_INTERVAL);
        if let Err(error) = collect(device, sink, &anchor, &mut silence) {
            let detail = format!(
                "capture failed with 0x{:08X}: {}",
                code_of(&error),
                error.message()
            );
            match code_of(&error) {
                AUDCLNT_E_DEVICE_INVALIDATED
                | AUDCLNT_E_SERVICE_NOT_RUNNING
                | AUDCLNT_E_RESOURCES_INVALIDATED => sink.device_lost(&detail),
                _ => sink.failed(&detail),
            }
            ended_by_device = true;
            break;
        }
    }
    if !ended_by_device {
        // What the device buffered before the stop was asked for is still part of the track.
        let _ = collect(device, sink, &anchor, &mut silence);
    }
    // SAFETY: `client` is live. A failure to stop a device that has gone away is expected
    // and changes nothing.
    let _ = unsafe { device.client.Stop() };
}

/// Moves every packet the device has ready into the sink.
fn collect(
    device: &Opened,
    sink: &CaptureSink,
    anchor: &Anchor,
    silence: &mut Vec<u8>,
) -> windows::core::Result<()> {
    let frame_bytes = device.format.bytes_per_frame() as usize;
    loop {
        // SAFETY: `capture` is live on this thread.
        if unsafe { device.capture.GetNextPacketSize() }? == 0 {
            return Ok(());
        }
        let mut data = std::ptr::null_mut();
        let (mut frames, mut buffer_flags, mut position_hns) = (0u32, 0u32, 0u64);
        // SAFETY: every pointer refers to a local that outlives the call. On success the
        // buffer stays valid until the matching `ReleaseBuffer` below.
        unsafe {
            device.capture.GetBuffer(
                &mut data,
                &mut frames,
                &mut buffer_flags,
                None,
                Some(&mut position_hns),
            )
        }?;

        let mut packet_flags = 0;
        if buffer_flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY != 0 {
            packet_flags |= flags::DISCONTINUITY;
        }
        let timeline_us = if buffer_flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR != 0 {
            packet_flags |= flags::TIME_ESTIMATED;
            sink.clock().now_us() - device.format.duration_us(u64::from(frames))
        } else {
            anchor.timeline_us(position_hns)
        };
        let bytes = frames as usize * frame_bytes;
        if buffer_flags & AUDCLNT_BUFFERFLAGS_SILENT != 0 || data.is_null() {
            // Windows says to treat the packet as silence whatever the buffer holds.
            packet_flags |= flags::SILENT;
            silence.clear();
            silence.resize(bytes, 0);
            sink.packet(timeline_us, frames, packet_flags, silence);
        } else {
            // SAFETY: `GetBuffer` succeeded with a non-null pointer, so it addresses `frames`
            // frames of `frame_bytes` bytes each, readable until `ReleaseBuffer`.
            let packet = unsafe { std::slice::from_raw_parts(data, bytes) };
            sink.packet(timeline_us, frames, packet_flags, packet);
        }
        // SAFETY: releases exactly the frames `GetBuffer` returned; `packet` is not used again.
        unsafe { device.capture.ReleaseBuffer(frames) }?;
    }
}
