//! Convenience types for building status messages required by LMS.
//!
//! The Lyrion Media Server expects periodic status reports from clients. This
//! module provides `StatusData`, a structured representation of the fields
//! included in those reports, together with `StatusCode` values used as the
//! event identifier.
use std::{
    fmt,
    time::{Duration, Instant},
};

use crate::messages::ClientMessage;

/// Structured status information sent to LMS.
///
/// `StatusData` contains the fields that the server expects in a STAT frame.
/// Helper methods are provided to update individual fields and to create a
/// `ClientMessage::Stat` instance ready for serialization.
#[derive(Clone, Debug)]
pub struct StatusData {
    pub(crate) crlf: u8,
    pub(crate) buffer_size: u32,
    pub(crate) fullness: u32,
    pub(crate) bytes_received: u64,
    pub(crate) sig_strength: u16,
    pub(crate) jiffies: Duration,
    pub(crate) output_buffer_size: u32,
    pub(crate) output_buffer_fullness: u32,
    pub(crate) elapsed_seconds: u32,
    pub(crate) voltage: u16,
    pub(crate) elapsed_milliseconds: u32,
    pub(crate) timestamp: Duration,
    pub(crate) error_code: u16,
    // -- Items below are not sent to the LMS
    pub(crate) start: Instant,
}

impl StatusData {
    pub fn add_crlf(&mut self, num_crlf: u8) {
        self.crlf = self.crlf.wrapping_add(num_crlf);
    }

    pub fn set_fullness(&mut self, fullness: u32) {
        self.fullness = fullness;
    }

    pub fn add_bytes_received(&mut self, bytes_received: u64) {
        self.bytes_received = self.bytes_received.wrapping_add(bytes_received);
    }

    pub fn set_jiffies(&mut self, jiffies: Duration) {
        self.jiffies = jiffies;
    }

    pub fn set_output_buffer_size(&mut self, output_buffer_size: u32) {
        self.output_buffer_size = output_buffer_size;
    }

    pub fn set_output_buffer_fullness(&mut self, output_buffer_fullness: u32) {
        self.output_buffer_fullness = output_buffer_fullness;
    }

    pub fn set_elapsed_seconds(&mut self, elapsed_seconds: u32) {
        self.elapsed_seconds = elapsed_seconds;
    }

    pub fn set_elapsed_milli_seconds(&mut self, elapsed_milli_seconds: u32) {
        self.elapsed_milliseconds = elapsed_milli_seconds;
    }

    pub fn set_buffer_size(&mut self, size: u32) {
        self.buffer_size = size;
    }

    pub fn set_timestamp(&mut self, timestamp: Duration) {
        self.timestamp = timestamp;
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received
    }

    pub fn jiffies(&self) -> Duration {
        self.jiffies
    }

    /// Create a status message for sending to the server
    pub fn make_status_message(&mut self, msgtype: StatusCode) -> ClientMessage {
        self.set_jiffies(Instant::now() - self.start);
        ClientMessage::Stat {
            event_code: msgtype.to_string(),
            stat_data: self.clone(),
        }
    }
}

impl Default for StatusData {
    fn default() -> Self {
        Self {
            crlf: 0,
            buffer_size: 0,
            fullness: 0,
            bytes_received: 0,
            sig_strength: 0,
            jiffies: Duration::default(),
            output_buffer_size: 0,
            output_buffer_fullness: 0,
            elapsed_seconds: 0,
            voltage: 0,
            elapsed_milliseconds: 0,
            timestamp: Duration::default(),
            error_code: 0,
            start: Instant::now(),
        }
    }
}

/// Codes used to identify the status event being sent.
///
/// Each variant maps to the short token used by LMS (e.g. `STMc` for
/// `Connect`). `StatusData::make_status_message` accepts a `StatusCode` to
/// produce the correct event string.
pub enum StatusCode {
    Connect,
    DecoderReady,
    StreamEstablished,
    Flushed,
    HeadersReceived,
    BufferThreshold,
    NotSupported,
    OutputUnderrun,
    Pause,
    Resume,
    TrackStarted,
    Timer,
    Underrun,
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_str = match &self {
            StatusCode::Connect => "STMc",
            StatusCode::DecoderReady => "STMd",
            StatusCode::StreamEstablished => "STMe",
            StatusCode::Flushed => "STMf",
            StatusCode::HeadersReceived => "STMh",
            StatusCode::BufferThreshold => "STMl",
            StatusCode::NotSupported => "STMn",
            StatusCode::OutputUnderrun => "STMo",
            StatusCode::Pause => "STMp",
            StatusCode::Resume => "STMr",
            StatusCode::TrackStarted => "STMs",
            StatusCode::Timer => "STMt",
            StatusCode::Underrun => "STMu",
        };
        write!(f, "{status_str}")
    }
}
