// src/decode.rs
//
// Audio decoding via FFmpeg (ffmpeg-next + ffmpeg-sys-next).
//
// Architecture
// ============
// FFmpeg's demuxer normally opens a URL or file path.  We feed it data from
// a `SlimBuffer<TcpStream>` via a custom `AVIOContext` whose read callback
// calls `reader.read()` on each request.
//
// Ownership model
// ---------------
// `OwnedAvio` is the single owner of everything allocated for the custom I/O:
//   - the boxed reader (`Box<dyn Read + Send>`)
//   - the raw AVIO I/O buffer (allocated by FFmpeg via `av_malloc`)
//   - the `AVIOContext` itself
//
// The boxed reader is stored as `Box<dyn Read + Send>` in the struct — no raw
// pointer escapes from it, so no `SendPtr` newtype is needed.  The only raw
// pointer held is `avio: *mut AVIOContext`, which is covered by a single
// `unsafe impl Send` on `OwnedAvio`.
//
// The four unavoidable `unsafe` sites are:
//   1. `OwnedAvio::new`         — `avio_alloc_context` FFI call.
//   2. `OwnedAvio::read_cb`     — `extern "C"` boundary (edition-2024 style).
//   3. `Drop for OwnedAvio`     — `avio_context_free` FFI call.
//   4. `open_input_with_avio`   — `avformat_*` FFI calls.
//
// Sample format
// =============
// Many FFmpeg decoders emit planar f32 (`FLTP`).  We convert to packed f32
// (`FLT`) via `libswresample`.  The byte→f32 and f32→byte reinterpretations
// use `bytemuck::cast_slice` (safe, checked, no raw pointers).

#[cfg(feature = "notify")]
use std::collections::HashMap;
use std::{
    io::{BufRead, Read, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

use anyhow::{Context, bail};
#[allow(unused_imports)]
use crossbeam::{atomic::AtomicCell, channel::Sender};
use log::{debug, info, trace, warn};

use ffmpeg_next::{
    self as ffmpeg,
    codec::decoder::Audio as FfmpegAudioDecoder,
    format::context::Input,
    frame::Audio as AudioFrame,
    media::Type as MediaType,
    software::resampling,
    util::format::sample::{Sample, Type as SampleType},
};
use ffmpeg_sys_next as ffsys;

use lms_proto::{AutoStart, PcmChannels, PcmSampleRate, SlimBuffer};

use crate::{
    message::PlayerMsg,
    state::{STATUS, VOLUME},
};

// ---------------------------------------------------------------------------
// Stream parameters
// ---------------------------------------------------------------------------

/// Parameters supplied by the LMS `Stream` command that affect playback startup.
pub struct StreamParams {
    pub autostart: AutoStart,
    pub output_threshold: Duration,
}

// ---------------------------------------------------------------------------
// Decoder error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DecoderError {
    /// An unrecoverable FFmpeg error.
    StreamError(ffmpeg::Error),
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderError::StreamError(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for DecoderError {}

// ---------------------------------------------------------------------------
// Audio specification helper
// ---------------------------------------------------------------------------

struct AudioSpec {
    channels: usize,
    sample_rate: u32,
}

// ---------------------------------------------------------------------------
// OwnedAvio — custom AVIOContext backed by a boxed reader
// ---------------------------------------------------------------------------

const AVIO_BUF_SIZE: usize = 64 * 1024;

/// Owns a heap-pinned reader, the AVIO I/O buffer, and the `AVIOContext`.
///
/// # Why the reader is double-boxed
///
/// FFmpeg stores `opaque` — a `*mut c_void` — inside the `AVIOContext` and
/// calls our `read_cb` with it on every read.  The pointer must remain valid
/// and stable for the lifetime of the context.
///
/// If we stored the reader as a field of `OwnedAvio` directly, moving the
/// struct (e.g. returning it from `new`, or storing it inside `VibeDecoder`)
/// would invalidate the pointer → segfault.
///
/// The solution: allocate the reader separately with `Box::into_raw` *before*
/// creating the AVIO context.  The raw pointer is stable regardless of where
/// `OwnedAvio` itself lives.  Ownership is reclaimed in `Drop` via
/// `Box::from_raw`, which also runs the reader's destructor.
struct OwnedAvio {
    /// Heap-pinned reader.  Kept here so it is dropped after `avio_context_free`.
    /// The raw pointer passed to FFmpeg as `opaque` aliases this allocation;
    /// we must not move or drop it while the AVIO context is alive.
    _reader: *mut Box<dyn Read + Send>,
    /// The `AVIOContext`.  FFmpeg owns and frees the internal I/O buffer.
    avio: *mut ffsys::AVIOContext,
}

// SAFETY: both raw pointers are only ever accessed from the single decode
// thread that owns `VibeDecoder`.  The underlying reader is `Send` by bound.
unsafe impl Send for OwnedAvio {}

impl OwnedAvio {
    fn new(reader: impl Read + Send + 'static) -> anyhow::Result<Self> {
        // Heap-allocate the reader and immediately convert to a raw pointer.
        // We box it twice: the inner Box<dyn Read+Send> is the trait object;
        // the outer Box pins *that* fat pointer on the heap, giving us a stable
        // thin pointer (`*mut Box<dyn Read+Send>`) we can store as `*mut c_void`
        // and safely cast back in `read_cb`.
        // SAFETY: we reclaim ownership in `Drop` via `Box::from_raw`.
        let reader_ptr: *mut Box<dyn Read + Send> =
            Box::into_raw(Box::new(Box::new(reader) as Box<dyn Read + Send>));

        // Allocate the internal I/O buffer.  FFmpeg takes ownership and frees
        // it inside `avio_context_free`.
        let buf = unsafe { ffsys::av_malloc(AVIO_BUF_SIZE) as *mut u8 };
        if buf.is_null() {
            // Reclaim the reader before returning.
            drop(unsafe { Box::from_raw(reader_ptr) });
            bail!("av_malloc failed for AVIO buffer");
        }

        // `opaque` is the stable heap address of the boxed reader.
        let opaque = reader_ptr as *mut libc::c_void; // thin pointer → safe to cast

        // SAFETY: `buf` is a valid allocation; `opaque` is a stable heap
        // address that outlives the AVIO context; `read_cb` is a valid
        // C-compatible function pointer for the concrete type.
        let avio = unsafe {
            ffsys::avio_alloc_context(
                buf,
                AVIO_BUF_SIZE as libc::c_int,
                0, // read-only
                opaque,
                Some(Self::read_cb),
                None, // no write callback
                None, // not seekable
            )
        };
        if avio.is_null() {
            // FFmpeg freed `buf` on failure; we still own the reader.
            drop(unsafe { Box::from_raw(reader_ptr) });
            bail!("avio_alloc_context returned NULL");
        }

        Ok(OwnedAvio {
            _reader: reader_ptr,
            avio,
        })
    }

    /// Read callback invoked by FFmpeg when it needs more bytes.
    ///
    /// # Safety
    /// `opaque` must be a pointer produced by `Box::into_raw::<Box<dyn Read+Send>>`
    /// and must remain valid for the lifetime of the `AVIOContext`.
    unsafe extern "C" fn read_cb(
        opaque: *mut libc::c_void,
        buf: *mut u8,
        buf_size: libc::c_int,
    ) -> libc::c_int {
        // Edition 2024: unsafe operations inside `unsafe fn` require an
        // explicit inner `unsafe` block.
        unsafe {
            // Cast back to the same type used in `new`: thin pointer to the fat pointer.
            let reader: &mut dyn Read = &mut **(opaque as *mut Box<dyn Read + Send>);
            let out = std::slice::from_raw_parts_mut(buf, buf_size as usize);
            match reader.read(out) {
                Ok(0) | Err(_) => ffsys::AVERROR_EOF,
                Ok(n) => n as libc::c_int,
            }
        }
    }
}

impl Drop for OwnedAvio {
    fn drop(&mut self) {
        if !self.avio.is_null() {
            // Free the AVIOContext (and its internal I/O buffer) first.
            // SAFETY: `self.avio` is a valid FFmpeg-allocated context.
            unsafe { ffsys::avio_context_free(&mut self.avio) };
        }
        if !self._reader.is_null() {
            // Reclaim the boxed reader and run its destructor.
            // SAFETY: `self._reader` was produced by `Box::into_raw` in `new`;
            // this runs exactly once.
            drop(unsafe { Box::from_raw(self._reader) });
        }
    }
}

// ---------------------------------------------------------------------------
// VibeDecoder
// ---------------------------------------------------------------------------

/// Wraps the FFmpeg format context and audio decoder for a single stream.
pub struct VibeDecoder {
    /// Demuxer (AVFormatContext).
    input: Input,
    /// Codec context / decoder.
    decoder: FfmpegAudioDecoder,
    /// Index of the first audio stream in the container.
    stream_index: usize,
    /// Resampler for planar → packed f32 conversion.
    resampler: Option<resampling::Context>,
    /// Fallback spec from the LMS `strm` command.
    spec: AudioSpec,
    /// Keeps the AVIO wrapper (and its boxed reader) alive alongside `input`.
    _avio: OwnedAvio,
}

impl VibeDecoder {
    fn try_new_from_input(
        input: Input,
        avio: OwnedAvio,
        pcmsamplerate: PcmSampleRate,
        pcmchannels: PcmChannels,
    ) -> anyhow::Result<Self> {
        let stream = input
            .streams()
            .best(MediaType::Audio)
            .context("No audio stream found in container")?;
        let stream_index = stream.index();

        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .context("Unable to create codec context from stream parameters")?;
        let decoder = codec_ctx
            .decoder()
            .audio()
            .context("Unable to open audio decoder")?;

        let sample_rate = match pcmsamplerate {
            PcmSampleRate::Rate(r) => r,
            PcmSampleRate::SelfDescribing => decoder.rate(),
        };
        let channels = match pcmchannels {
            PcmChannels::Mono => 1,
            PcmChannels::Stereo => 2,
            PcmChannels::SelfDescribing => decoder.channels() as usize,
        };

        // Build a resampler if the decoder does not already emit packed f32.
        let resampler = if decoder.format() != Sample::F32(SampleType::Packed) {
            Some(
                resampling::Context::get(
                    decoder.format(),
                    decoder.channel_layout(),
                    decoder.rate(),
                    Sample::F32(SampleType::Packed),
                    decoder.channel_layout(),
                    decoder.rate(),
                )
                .context("Unable to create audio resampler")?,
            )
        } else {
            None
        };

        Ok(VibeDecoder {
            input,
            decoder,
            stream_index,
            resampler,
            spec: AudioSpec {
                channels,
                sample_rate,
            },
            _avio: avio,
        })
    }

    // -----------------------------------------------------------------------
    // Public accessors
    // -----------------------------------------------------------------------

    pub fn channels(&self) -> usize {
        let c = self.decoder.channels() as usize;
        if c > 0 { c } else { self.spec.channels }
    }

    pub fn sample_rate(&self) -> u32 {
        let r = self.decoder.rate();
        if r > 0 { r } else { self.spec.sample_rate }
    }

    #[allow(unused)]
    pub fn samples_to_dur(&self, samples: u64) -> Duration {
        Duration::from_millis(
            samples * 1_000 / (self.sample_rate() as u64 * self.channels() as u64),
        )
    }

    pub fn dur_to_samples(&self, dur: Duration) -> u64 {
        self.sample_rate() as u64 * self.channels() as u64 * dur.as_micros() as u64 / 1_000_000
    }

    // -----------------------------------------------------------------------
    // Metadata (notify feature)
    // -----------------------------------------------------------------------

    #[cfg(feature = "notify")]
    pub fn metadata(&self) -> Option<HashMap<String, String>> {
        let meta = self.input.metadata();
        _ = meta.iter().next()?;

        Some(
            meta.iter()
                .map(|(k, v)| (k.to_lowercase(), v.to_string()))
                .collect(),
        )
    }

    // -----------------------------------------------------------------------
    // Core decode loop
    // -----------------------------------------------------------------------

    fn get_audio_buffer(&mut self) -> Result<Option<Vec<f32>>, DecoderError> {
        loop {
            let (stream, packet) = match self.input.packets().next() {
                Some(sp) => sp,
                None => {
                    self.decoder.flush();
                    return self.drain_decoder();
                }
            };

            if stream.index() != self.stream_index {
                continue;
            }

            self.decoder
                .send_packet(&packet)
                .map_err(DecoderError::StreamError)?;

            match self.receive_frame_and_convert()? {
                Some(samples) => return Ok(Some(samples)),
                None => continue,
            }
        }
    }

    fn receive_frame_and_convert(&mut self) -> Result<Option<Vec<f32>>, DecoderError> {
        let mut frame = AudioFrame::empty();
        match self.decoder.receive_frame(&mut frame) {
            Ok(_) => {}
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                return Ok(None);
            }
            Err(e) => return Err(DecoderError::StreamError(e)),
        }
        let samples = self.frame_to_f32(&frame)?;
        Ok(Some(Self::apply_volume(samples)))
    }

    fn drain_decoder(&mut self) -> Result<Option<Vec<f32>>, DecoderError> {
        let mut frame = AudioFrame::empty();
        match self.decoder.receive_frame(&mut frame) {
            Ok(_) => {
                let samples = self.frame_to_f32(&frame)?;
                Ok(Some(Self::apply_volume(samples)))
            }
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => Ok(None),
            Err(ffmpeg::Error::Eof) => Ok(None),
            Err(e) => Err(DecoderError::StreamError(e)),
        }
    }

    /// Convert an `AudioFrame` to packed f32, resampling if necessary.
    ///
    /// `bytemuck::cast_slice` safely reinterprets the raw frame bytes as
    /// `&[f32]` — no raw pointer arithmetic needed.
    fn frame_to_f32(&mut self, frame: &AudioFrame) -> Result<Vec<f32>, DecoderError> {
        let packed = if let Some(ref mut resampler) = self.resampler {
            let mut out = AudioFrame::empty();
            resampler
                .run(frame, &mut out)
                .map_err(DecoderError::StreamError)?;
            out
        } else {
            frame.clone()
        };

        Ok(bytemuck::cast_slice(packed.data(0)).to_vec())
    }

    fn apply_volume(mut samples: Vec<f32>) -> Vec<f32> {
        let (lv, rv) = VOLUME.lock().map(|v| (v[0], v[1])).unwrap_or((1.0, 1.0));
        samples.chunks_exact_mut(2).for_each(|fr| {
            if let [l, r] = fr {
                *l *= lv;
                *r *= rv;
            }
        });
        if !samples.len().is_multiple_of(2) {
            samples.iter_mut().for_each(|s| *s *= lv);
        }
        samples
    }

    // -----------------------------------------------------------------------
    // Backend-facing buffer-fill methods
    // -----------------------------------------------------------------------

    /// Fill `buffer` with interleaved f32 samples (rodio backend).
    #[cfg(feature = "rodio")]
    pub fn fill_sample_buffer(
        &mut self,
        buffer: &mut Vec<f32>,
        limit: Option<usize>,
    ) -> Result<bool, DecoderError> {
        let limit = limit.unwrap_or_else(|| (buffer.capacity() / 2).max(1024));
        let mut eof = false;
        while buffer.len() < limit && !eof {
            match self.get_audio_buffer()? {
                Some(chunk) => buffer.extend(chunk),
                None => eof = true,
            }
        }
        Ok(eof)
    }

    /// Fill `buffer` with raw little-endian f32-as-bytes (pulse / pipewire backends).
    ///
    /// `bytemuck::cast_slice` handles the f32 → u8 reinterpretation safely.
    #[cfg(any(feature = "pulse", feature = "pipewire"))]
    pub fn fill_raw_buffer(
        &mut self,
        buffer: &mut Vec<u8>,
        limit: Option<usize>,
    ) -> Result<bool, DecoderError> {
        let limit = limit.unwrap_or_else(|| (buffer.capacity() / 2).max(1024));
        let mut eof = false;
        while buffer.len() < limit && !eof {
            match self.get_audio_buffer()? {
                Some(chunk) => buffer.extend_from_slice(bytemuck::cast_slice(&chunk)),
                None => eof = true,
            }
        }
        Ok(eof)
    }
}

// ---------------------------------------------------------------------------
// FFmpeg → log crate bridge
// ---------------------------------------------------------------------------

/// Route FFmpeg's internal log output through Rust's `log` crate.
///
/// Called once at startup (inside `make_decoder`).  FFmpeg's log levels map
/// to `log` levels as follows:
///
/// | FFmpeg              | log    |
/// |---------------------|--------|
/// | AV_LOG_PANIC/FATAL  | error  |
/// | AV_LOG_ERROR        | warn   |
/// | AV_LOG_WARNING      | info   |
/// | AV_LOG_INFO         | debug  |
/// | AV_LOG_VERBOSE/DEBUG| trace  |
///
/// FFmpeg's "invalid concatenated file" and "Could not update timestamps"
/// messages are at AV_LOG_WARNING and AV_LOG_ERROR respectively — they
/// appear as `info!` and `warn!` through this bridge, so `--loglevel warn`
/// or higher silences them entirely.
fn install_ffmpeg_log_callback() {
    unsafe extern "C" fn log_cb(
        _avcl: *mut libc::c_void,
        level: libc::c_int,
        fmt: *const libc::c_char,
        vl: *mut ffsys::__va_list_tag,
    ) {
        unsafe {
            // Format the message the same way FFmpeg would to stderr.
            let mut buf = [0i8; 1024];
            let mut print_prefix: libc::c_int = 1;
            ffsys::av_log_format_line2(
                _avcl,
                level,
                fmt,
                vl,
                buf.as_mut_ptr(),
                buf.len() as libc::c_int,
                &mut print_prefix,
            );
            // Convert to a Rust string, trimming the trailing newline FFmpeg adds.
            let msg = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .trim_end_matches('\n')
                .to_owned();
            if msg.is_empty() {
                return;
            }
            // Map FFmpeg level constants to log levels.
            // AV_LOG_PANIC=0, FATAL=8, ERROR=16, WARNING=24, INFO=32,
            // VERBOSE=40, DEBUG=48, TRACE=56.
            match level {
                l if l <= ffsys::AV_LOG_ERROR => warn!(target: "ffmpeg", "{}", msg),
                l if l <= ffsys::AV_LOG_WARNING => info!(target: "ffmpeg", "{}", msg),
                l if l <= ffsys::AV_LOG_INFO => debug!(target: "ffmpeg", "{}", msg),
                _ => trace!(target: "ffmpeg", "{}", msg),
            }
        }
    }

    // Install the callback.  This is process-global and idempotent.
    // SAFETY: `log_cb` is a valid C function pointer with the right signature.
    unsafe { ffsys::av_log_set_callback(Some(log_cb)) };
}

// ---------------------------------------------------------------------------
// Public factory
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn make_decoder(
    server_ip: Ipv4Addr,
    default_ip: Ipv4Addr,
    server_port: u16,
    http_headers: String,
    stream_in: Sender<PlayerMsg>,
    threshold: u32,
    format: lms_proto::Format,
    _pcmsamplesize: lms_proto::PcmSampleSize,
    pcmsamplerate: lms_proto::PcmSampleRate,
    pcmchannels: lms_proto::PcmChannels,
    autostart: AutoStart,
    output_threshold: Duration,
) -> anyhow::Result<(VibeDecoder, StreamParams)> {
    ffmpeg::init().context("Failed to initialise FFmpeg")?;
    install_ffmpeg_log_callback();

    let ip = if server_ip.is_unspecified() {
        default_ip
    } else {
        server_ip
    };

    let data_stream = make_tcp_connection(ip, server_port, http_headers)
        .context(format!("Unable to connect to data stream at {ip}"))?;
    _ = stream_in.send(PlayerMsg::Connected);

    let mut data_stream = SlimBuffer::with_capacity(
        threshold as usize * 1024,
        data_stream,
        STATUS.clone(),
        threshold,
        None,
    );
    _ = stream_in.send(PlayerMsg::BufferThreshold);

    {
        let mut line = String::new();
        loop {
            line.clear();
            let n = data_stream.read_line(&mut line)?;
            if n == 0 || line == "\r\n" || line.len() > 8 * 1024 {
                break;
            }
        }
    }

    let avio = OwnedAvio::new(data_stream).context("Unable to allocate custom AVIO context")?;
    let input =
        open_input_with_avio(avio.avio, &format).context("Unable to probe/open audio container")?;
    let decoder = VibeDecoder::try_new_from_input(input, avio, pcmsamplerate, pcmchannels)
        .context("Unable to initialise audio decoder")?;

    Ok((
        decoder,
        StreamParams {
            autostart,
            output_threshold,
        },
    ))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn open_input_with_avio(
    avio: *mut ffsys::AVIOContext,
    format: &lms_proto::Format,
) -> anyhow::Result<Input> {
    use std::ffi::CString;

    let input_fmt_name: Option<CString> = match format {
        lms_proto::Format::Pcm => Some(CString::new("s16le").unwrap()),
        _ => None,
    };

    // All operations below cross the C FFI boundary and are therefore unsafe.
    unsafe {
        let input_fmt = input_fmt_name
            .as_ref()
            .map(|n| ffsys::av_find_input_format(n.as_ptr()))
            .unwrap_or(std::ptr::null_mut());

        let mut fmt_ctx = ffsys::avformat_alloc_context();
        if fmt_ctx.is_null() {
            bail!("avformat_alloc_context returned NULL");
        }

        (*fmt_ctx).pb = avio;

        let empty_url = CString::new("").unwrap();
        let ret = ffsys::avformat_open_input(
            &mut fmt_ctx,
            empty_url.as_ptr(),
            input_fmt,
            std::ptr::null_mut(),
        );
        if ret < 0 {
            bail!("avformat_open_input failed: {}", ffmpeg::Error::from(ret));
        }

        let ret = ffsys::avformat_find_stream_info(fmt_ctx, std::ptr::null_mut());
        if ret < 0 {
            ffsys::avformat_close_input(&mut fmt_ctx);
            bail!(
                "avformat_find_stream_info failed: {}",
                ffmpeg::Error::from(ret)
            );
        }

        Ok(Input::wrap(fmt_ctx))
    }
}

fn make_tcp_connection(ip: Ipv4Addr, port: u16, http_headers: String) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect((ip, port))?;
    stream.write_all(format!("{}\r\n", http_headers.trim()).as_bytes())?;
    stream.write_all(b"\r\n\r\n")?;
    stream.flush()?;
    Ok(stream)
}
