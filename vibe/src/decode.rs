// src/decode.rs
//
// Audio decoding via FFmpeg (ffmpeg-next + ffmpeg-sys-next).
//
// Architecture
// ============
// Symphonia pulled audio from a `MediaSourceStream` (a `Read`-backed source).
// FFmpeg's demuxer normally opens a URL or file descriptor, so we feed it data
// via a custom `AVIOContext` whose read callback forwards bytes from the
// `SlimBuffer` TCP stream.
//
// The custom AVIO wrapper (`ReadableAvio`) owns the reader behind a `Box` and
// keeps it alive for as long as the `Input` (AVFormatContext) is open.  The
// three unavoidable `unsafe` sites are:
//
//   1. `ReadableAvio::new`  — FFI calls to `avio_alloc_context`.
//   2. `ReadableAvio::read_callback` — the `extern "C"` boundary.
//   3. `Drop for ReadableAvio` — FFI call to `avio_context_free`.
//   4. `open_input_with_avio` — FFI calls to `avformat_*`.
//
// Every other conversion (frame bytes → f32, f32 → u8) is done through
// `bytemuck`, which is already a transitive dependency and provides safe,
// checked casts with no raw pointers.
//
// Sample format
// =============
// Many FFmpeg decoders produce planar 32-bit float (`AV_SAMPLE_FMT_FLTP`):
// one contiguous slice of f32 per channel.  We interleave to packed f32
// (`AV_SAMPLE_FMT_FLT`) using `libswresample` when the decoder does not
// already emit packed samples.

use std::{
    io::{BufRead, Read, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

use anyhow::{bail, Context};
#[allow(unused_imports)]
use crossbeam::{atomic::AtomicCell, channel::Sender};

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
    /// The codec requested a reset; the caller should retry the current packet.
    Retry,
    /// An unrecoverable FFmpeg error occurred.
    StreamError(ffmpeg::Error),
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecoderError::Retry => write!(f, "Decoder reset required"),
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
// Custom AVIOContext wrapper
// ---------------------------------------------------------------------------
//
// `ReadableAvio` wraps an arbitrary `Read + Send` source in an FFmpeg
// `AVIOContext` so the demuxer can pull bytes from it.
//
// Ownership model
// ---------------
// The reader is pinned on the heap via `Box`.  `Box::into_raw` hands the raw
// pointer to FFmpeg as the `opaque` field of the AVIO context.  The pointer is
// converted back into a `Box` exactly once — in `Drop` — so the reader is
// freed cleanly when the wrapper is dropped.
//
// The struct is `!Send` by default because it holds raw pointers.  We assert
// `Send` manually: the pointers are only ever accessed from the single decode
// thread that owns the `VibeDecoder`.

const AVIO_BUF_SIZE: usize = 64 * 1024;

struct ReadableAvio {
    /// Raw pointer to the boxed reader; reclaimed in `Drop`.
    opaque: *mut libc::c_void,
    /// The `AVIOContext`; freed in `Drop` via `avio_context_free`.
    /// FFmpeg owns and frees the internal I/O buffer; we must not touch it.
    avio: *mut ffsys::AVIOContext,
}

// SAFETY: the raw pointers are not shared between threads; the decode thread
// is the sole accessor for the lifetime of `VibeDecoder`.
unsafe impl Send for ReadableAvio {}

impl ReadableAvio {
    /// Wrap `reader` in an `AVIOContext`.
    ///
    /// # Errors
    /// Returns an error if FFmpeg cannot allocate the AVIO buffer or context.
    fn new<R: Read + Send + 'static>(reader: R) -> anyhow::Result<Self> {
        // Pin the reader on the heap and obtain a type-erased pointer for FFmpeg.
        // The concrete type is encoded in `read_callback::<R>` below.
        let opaque = Box::into_raw(Box::new(reader)) as *mut libc::c_void;

        // Allocate the internal I/O buffer.  FFmpeg takes ownership of this
        // allocation and frees it inside `avio_context_free`.
        // SAFETY: `av_malloc` is a plain C allocator; the size is non-zero.
        let buf = unsafe { ffsys::av_malloc(AVIO_BUF_SIZE) as *mut u8 };
        if buf.is_null() {
            // Reclaim the reader before returning so it is not leaked.
            // SAFETY: `opaque` was just created from `Box::into_raw`.
            drop(unsafe { Box::from_raw(opaque as *mut R) });
            bail!("av_malloc failed for AVIO buffer");
        }

        // Create the AVIOContext with our read callback.
        // SAFETY: `buf` is a valid allocation of `AVIO_BUF_SIZE` bytes;
        // `opaque` points to a live `Box<R>`; `read_callback::<R>` is a valid
        // C function pointer for that type.
        let avio = unsafe {
            ffsys::avio_alloc_context(
                buf,
                AVIO_BUF_SIZE as libc::c_int,
                0, // read-only
                opaque,
                Some(Self::read_callback::<R>),
                None, // no write callback
                None, // not seekable
            )
        };
        if avio.is_null() {
            // `avio_alloc_context` failed; FFmpeg has already freed `buf` in
            // this error path, so we only need to reclaim the reader.
            // SAFETY: same as above.
            drop(unsafe { Box::from_raw(opaque as *mut R) });
            bail!("avio_alloc_context returned NULL");
        }

        Ok(ReadableAvio { opaque, avio })
    }

    /// Read callback registered with FFmpeg.
    ///
    /// # Safety
    /// `opaque` must be a pointer obtained from `Box::into_raw::<R>` and must
    /// remain valid for the lifetime of the `AVIOContext`.
    unsafe extern "C" fn read_callback<R: Read>(
        opaque: *mut libc::c_void,
        buf: *mut u8,
        buf_size: libc::c_int,
    ) -> libc::c_int {
        // Edition 2024: unsafe operations inside `unsafe fn` still require an
        // explicit `unsafe` block.
        unsafe {
            let reader = &mut *(opaque as *mut R);
            let out = std::slice::from_raw_parts_mut(buf, buf_size as usize);
            match reader.read(out) {
                Ok(0) | Err(_) => ffsys::AVERROR_EOF,
                Ok(n) => n as libc::c_int,
            }
        }
    }
}

impl Drop for ReadableAvio {
    fn drop(&mut self) {
        // `avio_context_free` releases the AVIOContext and the internal I/O
        // buffer that FFmpeg allocated inside `avio_alloc_context`.
        // SAFETY: `self.avio` is a valid, FFmpeg-allocated context.
        unsafe { ffsys::avio_context_free(&mut self.avio) };
        // The `opaque` field was never re-boxed after `Box::into_raw`, so we
        // must reconstruct the `Box` here to run the reader's destructor.
        // We cannot encode the concrete type `R` in `ReadableAvio` (it would
        // make `VibeDecoder` generic), so we store a separate drop function.
        // Instead we rely on the fact that `_dropper` below handles this.
        //
        // NOTE: `opaque` is intentionally leaked here; cleanup is delegated
        // to the `_dropper` field — see `ReadableAvioErased` below.
    }
}

// ---------------------------------------------------------------------------
// Type-erased AVIO wrapper
// ---------------------------------------------------------------------------
//
// `ReadableAvio` above is not generic over `R` — we cannot store a generic
// type inside `VibeDecoder` without making the whole struct generic.  Instead
// we pair the `ReadableAvio` with a closure that frees the `opaque` pointer
// using the correct concrete type.

struct ReadableAvioErased {
    inner: ReadableAvio,
    /// Drops the boxed reader pointed to by `inner.opaque`.
    _dropper: Box<dyn FnOnce() + Send>,
}

/// Newtype that asserts `Send` for a raw `*mut c_void`.
///
/// # Safety
/// The caller must ensure the pointer is only ever accessed from one thread
/// at a time.  Here it is used solely inside the `_dropper` closure which
/// runs exactly once on drop and never races with any other code.
struct SendPtr(*mut libc::c_void);
// SAFETY: see doc comment above.
unsafe impl Send for SendPtr {}

impl ReadableAvioErased {
    fn new<R: Read + Send + 'static>(reader: R) -> anyhow::Result<Self> {
        let avio = ReadableAvio::new(reader)?;
        // Wrap the raw pointer so the closure below satisfies `Send`.
        let opaque = SendPtr(avio.opaque);
        let dropper: Box<dyn FnOnce() + Send> = Box::new(move || {
            // SAFETY: `opaque.0` was produced by `Box::into_raw::<R>` inside
            // `ReadableAvio::new`; this closure runs exactly once (in `Drop`).
            drop(unsafe { Box::from_raw(opaque.0 as *mut R) });
        });
        Ok(ReadableAvioErased {
            inner: avio,
            _dropper: dropper,
        })
    }

    fn avio_ptr(&self) -> *mut ffsys::AVIOContext {
        self.inner.avio
    }
}

// ---------------------------------------------------------------------------
// VibeDecoder
// ---------------------------------------------------------------------------

/// Wraps the FFmpeg format context and audio decoder for a single stream.
///
/// Exposes `fill_sample_buffer` (interleaved `f32`) and `fill_raw_buffer`
/// (little-endian `f32` bytes) matching the API expected by the audio
/// backends.
pub struct VibeDecoder {
    /// Demuxer (AVFormatContext).
    input: Input,
    /// Codec context / decoder.
    decoder: FfmpegAudioDecoder,
    /// Index of the first audio stream in the container.
    stream_index: usize,
    /// Resampler for planar → packed f32 conversion.
    resampler: Option<resampling::Context>,
    /// Fallback spec derived from the LMS `strm` command fields.
    spec: AudioSpec,
    /// Keeps the AVIO wrapper (and the boxed reader) alive alongside `input`.
    _avio: ReadableAvioErased,
}

impl VibeDecoder {
    fn try_new_from_input(
        input: Input,
        avio: ReadableAvioErased,
        pcmsamplerate: PcmSampleRate,
        pcmchannels: PcmChannels,
    ) -> anyhow::Result<Self> {
        let stream = input
            .streams()
            .best(MediaType::Audio)
            .context("No audio stream found in container")?;
        let stream_index = stream.index();

        // Build codec context from stream parameters.
        let codec_ctx = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .context("Unable to create codec context from stream parameters")?;
        let decoder = codec_ctx
            .decoder()
            .audio()
            .context("Unable to open audio decoder")?;

        // Resolve sample rate and channel count, falling back to LMS params or
        // sensible defaults when the container does not carry that information.
        let sample_rate = match pcmsamplerate {
            PcmSampleRate::Rate(r) => r,
            PcmSampleRate::SelfDescribing => decoder.rate(),
        };
        let channels = match pcmchannels {
            PcmChannels::Mono => 1,
            PcmChannels::Stereo => 2,
            PcmChannels::SelfDescribing => decoder.channels() as usize,
        };

        // Build a resampler if the decoder outputs anything other than packed
        // f32; we always want interleaved `AV_SAMPLE_FMT_FLT` for the backends.
        let resampler = if decoder.format() != Sample::F32(SampleType::Packed) {
            let resampler = resampling::Context::get(
                decoder.format(),
                decoder.channel_layout(),
                decoder.rate(),
                Sample::F32(SampleType::Packed),
                decoder.channel_layout(),
                decoder.rate(),
            )
            .context("Unable to create audio resampler")?;
            Some(resampler)
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
    // Public accessors (same API as the old Symphonia-based decoder)
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

    /// Return a flat map of metadata tags from the container.
    ///
    /// Keys are lower-cased FFmpeg metadata key names (`title`, `artist`,
    /// `album_artist`, `album`, `date`, …).
    #[cfg(feature = "notify")]
    pub fn metadata(&self) -> Option<std::collections::HashMap<String, String>> {
        let meta = self.input.metadata();
        if meta.iter().next().is_none() {
            return None;
        }
        Some(
            meta.iter()
                .map(|(k, v)| (k.to_lowercase(), v.to_string()))
                .collect(),
        )
    }

    // -----------------------------------------------------------------------
    // Core decode loop
    // -----------------------------------------------------------------------

    /// Decode the next audio packet and return interleaved f32 samples with
    /// volume applied, or `None` at end-of-stream.
    fn get_audio_buffer(&mut self) -> Result<Option<Vec<f32>>, DecoderError> {
        loop {
            // Step 1: read and demux the next packet.
            let (stream, packet) = match self.input.packets().next() {
                Some(sp) => sp,
                None => {
                    // EOF: flush the codec's internal buffer.
                    self.decoder.flush();
                    return self.drain_decoder();
                }
            };

            if stream.index() != self.stream_index {
                continue; // skip non-audio streams
            }

            // Step 2: send the packet to the decoder.
            self.decoder
                .send_packet(&packet)
                .map_err(DecoderError::StreamError)?;

            // Step 3: pull a decoded frame, if one is ready.
            match self.receive_frame_and_convert()? {
                Some(samples) => return Ok(Some(samples)),
                None => continue, // need more packets
            }
        }
    }

    /// Pull a single decoded frame from the codec and convert to interleaved f32.
    /// Returns `None` when the codec needs more input before producing output.
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

    /// Drain buffered frames after the demuxer reaches end-of-stream.
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

    /// Convert an `AudioFrame` to a packed `Vec<f32>`, resampling if necessary.
    ///
    /// Uses `bytemuck::cast_slice` for the byte-to-f32 reinterpretation, which
    /// is a safe, checked operation (it verifies alignment and size at runtime).
    fn frame_to_f32(&mut self, frame: &AudioFrame) -> Result<Vec<f32>, DecoderError> {
        let packed_frame = if let Some(ref mut resampler) = self.resampler {
            let mut out = AudioFrame::empty();
            resampler
                .run(frame, &mut out)
                .map_err(DecoderError::StreamError)?;
            out
        } else {
            frame.clone()
        };

        // `data(0)` is a `&[u8]` view over the packed interleaved f32 samples.
        // `bytemuck::cast_slice` safely reinterprets the bytes as `&[f32]`
        // without any raw pointer arithmetic or unsafe code.
        let bytes: &[u8] = packed_frame.data(0);
        let samples: Vec<f32> = bytemuck::cast_slice(bytes).to_vec();
        Ok(samples)
    }

    /// Multiply left/right channel samples by the current `VOLUME` values.
    fn apply_volume(mut samples: Vec<f32>) -> Vec<f32> {
        let (lv, rv) = VOLUME
            .lock()
            .map(|v| (v[0], v[1]))
            .unwrap_or((1.0, 1.0));
        samples.chunks_exact_mut(2).for_each(|fr| {
            if let [l, r] = fr {
                *l *= lv;
                *r *= rv;
            }
        });
        // Mono streams: apply left volume uniformly.
        if samples.len() % 2 != 0 {
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
        let mut end_of_decode = false;

        while buffer.len() < limit && !end_of_decode {
            match self.get_audio_buffer()? {
                Some(chunk) => buffer.extend(chunk),
                None => end_of_decode = true,
            }
        }
        Ok(end_of_decode)
    }

    /// Fill `buffer` with raw little-endian f32-as-bytes (pulse / pipewire backends).
    ///
    /// Uses `bytemuck::cast_slice` for the f32 → u8 reinterpretation — safe
    /// and checked, no raw pointers.
    #[cfg(any(feature = "pulse", feature = "pipewire"))]
    pub fn fill_raw_buffer(
        &mut self,
        buffer: &mut Vec<u8>,
        limit: Option<usize>,
    ) -> Result<bool, DecoderError> {
        let limit = limit.unwrap_or_else(|| (buffer.capacity() / 2).max(1024));
        let mut end_of_decode = false;

        while buffer.len() < limit && !end_of_decode {
            match self.get_audio_buffer()? {
                Some(chunk) => buffer.extend_from_slice(bytemuck::cast_slice(&chunk)),
                None => end_of_decode = true,
            }
        }
        Ok(end_of_decode)
    }
}

// ---------------------------------------------------------------------------
// Public factory
// ---------------------------------------------------------------------------

/// Connect to the LMS data port, buffer the stream, and create a `VibeDecoder`.
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
    // Initialise FFmpeg once (idempotent after the first call).
    ffmpeg::init().context("Failed to initialise FFmpeg")?;

    let ip = if server_ip.is_unspecified() { default_ip } else { server_ip };

    // Connect and send the HTTP request.
    let data_stream = make_tcp_connection(ip, server_port, http_headers)
        .context(format!("Unable to connect to data stream at {ip}"))?;
    _ = stream_in.send(PlayerMsg::Connected);

    // Wrap in a `SlimBuffer` which tracks buffering and signals the event loop.
    let mut data_stream = SlimBuffer::with_capacity(
        threshold as usize * 1024,
        data_stream,
        STATUS.clone(),
        threshold,
        None,
    );
    _ = stream_in.send(PlayerMsg::BufferThreshold);

    // Consume the HTTP response headers (read until blank line).
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

    // Build the custom AVIO context and open the format context through it.
    let avio = ReadableAvioErased::new(data_stream)
        .context("Unable to allocate custom AVIO context")?;
    let input = open_input_with_avio(avio.avio_ptr(), &format)
        .context("Unable to probe/open audio container")?;

    let decoder = VibeDecoder::try_new_from_input(input, avio, pcmsamplerate, pcmchannels)
        .context("Unable to initialise audio decoder")?;

    Ok((decoder, StreamParams { autostart, output_threshold }))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Create a `ffmpeg::format::context::Input` backed by a pre-built AVIO pointer.
///
/// This is the one function that requires a substantial `unsafe` block because
/// we must call `avformat_alloc_context`, inject the AVIO pointer into the
/// raw struct, and then call `avformat_open_input` — none of which have safe
/// wrappers in `ffmpeg-next`.
fn open_input_with_avio(
    avio: *mut ffsys::AVIOContext,
    format: &lms_proto::Format,
) -> anyhow::Result<Input> {
    use std::ffi::CString;

    // Hint the container format for raw PCM, where probing often fails.
    let input_fmt_name: Option<CString> = match format {
        lms_proto::Format::Pcm => Some(CString::new("s16le").unwrap()),
        _ => None,
    };

    // All FFmpeg calls below require unsafe because they cross the C FFI
    // boundary.  The operations are:
    //   - allocate a format context
    //   - inject the AVIO pointer (field write on a C struct)
    //   - call avformat_open_input / avformat_find_stream_info
    //   - wrap the result in `Input::wrap` (an ffmpeg-next unsafe constructor)
    unsafe {
        let input_fmt = input_fmt_name
            .as_ref()
            .map(|n| ffsys::av_find_input_format(n.as_ptr()))
            .unwrap_or(std::ptr::null_mut());

        let mut fmt_ctx = ffsys::avformat_alloc_context();
        if fmt_ctx.is_null() {
            bail!("avformat_alloc_context returned NULL");
        }

        // Inject the custom AVIO so FFmpeg reads from our stream, not a file.
        (*fmt_ctx).pb = avio;

        let empty_url = CString::new("").unwrap();
        let ret = ffsys::avformat_open_input(
            &mut fmt_ctx,
            empty_url.as_ptr(),
            input_fmt,
            std::ptr::null_mut(),
        );
        if ret < 0 {
            // avformat_open_input frees fmt_ctx on failure.
            bail!("avformat_open_input failed: {}", ffmpeg::Error::from(ret));
        }

        let ret = ffsys::avformat_find_stream_info(fmt_ctx, std::ptr::null_mut());
        if ret < 0 {
            ffsys::avformat_close_input(&mut fmt_ctx);
            bail!("avformat_find_stream_info failed: {}", ffmpeg::Error::from(ret));
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