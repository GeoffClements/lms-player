/// A convenience module for working with client status data.
///
/// The Logitech Media Server requires regular status messages from
/// the client. This module provides convenience types for this.
use std::{
    fmt,
    time::{Duration, Instant},
};

use crate::messages::ClientMessage;

/// A struct to hold the status data as required by the server
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
    // pub fn new(buffer_size: u32, output_buffer_size: u32) -> Self {
    //     let mut stat = StatusData::default();
    //     stat.buffer_size = buffer_size;
    //     stat.output_buffer_size = output_buffer_size;
    //     stat
    // }

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
        let stat_data = self.clone();
        ClientMessage::Stat {
            event_code: msgtype.to_string(),
            stat_data,
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

/// Status code to send as part of the status message
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
        match &self {
            StatusCode::Connect => write!(f, "STMc"),
            StatusCode::DecoderReady => write!(f, "STMd"),
            StatusCode::StreamEstablished => write!(f, "STMe"),
            StatusCode::Flushed => write!(f, "STMf"),
            StatusCode::HeadersReceived => write!(f, "STMh"),
            StatusCode::BufferThreshold => write!(f, "STMl"),
            StatusCode::NotSupported => write!(f, "STMn"),
            StatusCode::OutputUnderrun => write!(f, "STMo"),
            StatusCode::Pause => write!(f, "STMp"),
            StatusCode::Resume => write!(f, "STMr"),
            StatusCode::TrackStarted => write!(f, "STMs"),
            StatusCode::Timer => write!(f, "STMt"),
            StatusCode::Underrun => write!(f, "STMu"),
        }
    }
}
