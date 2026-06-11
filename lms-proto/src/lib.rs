//! Lightweight Lyrion Media Server (LMS) protocol primitives
//!
//! This crate provides a small, focused implementation of the portions of the
//! LMS Protocol necessary for client-side interaction with a Lyrion Media
//! Server. It offers:
//!
//! - Frame encoding/decoding helpers (`frames`)
//! - Message types for client/server communication (`messages`)
//! - Convenience types for status reporting (`status`)
//! - A small builder to create and connect a client (`proto::Hello`)
//! - Utilities for capability representation and discovery on the LAN
//!
//! The crate is intentionally minimal and designed for embedding in players
//! and other lightweight clients that speak the LMS Protocol.
//!
//! Example (connect to server):
//!
//! ```no_run
//! use lms_proto::Hello;
//! let hello = Hello::new().device_id(1).revision(1);
//! let (mut rx, mut tx) = hello.connect(("127.0.0.1", lms_proto::SLIM_PORT)).unwrap();
//! ```

pub mod buffer;
pub mod capability;
pub mod discovery;
pub mod frames;
pub mod messages;
pub mod proto;
pub mod status;

pub use buffer::SlimBuffer;
pub use capability::Capability;
pub use discovery::discover;
pub use messages::{
    AutoStart, ClientMessage, Format, PcmChannels, PcmSampleRate, PcmSampleSize, ServerMessage,
};
pub use proto::Hello;
pub use status::{StatusCode, StatusData};

/// Default port used by Lyrion Media Server for Slim Protocol traffic.
pub const SLIM_PORT: u16 = 3483;
