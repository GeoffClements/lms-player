//! Frame encoder/decoder utilities.
//!
//! This module provides `LMSRecv` and `LMSSend` which handle the framed
//! messaging format used by the Slim Protocol: each message is length-prefixed
//! and may contain one or more logical server messages.
use std::{
    io::{Error, ErrorKind, Read, Write},
    time::Duration,
};

use bytes::{Buf, Bytes, BytesMut};

use crate::messages::{ClientMessage, ServerMessage, ServerMessages};

const INITIAL_CAPACITY: usize = 4 * 1024;

/// A reader which collects bytes from an underlying source and decodes
/// LMS Protocol frames into `ServerMessage` values.
pub struct LMSRecv<R> {
    inner: R,
    buf: BytesMut,
}

impl<R> LMSRecv<R> {
    /// Create a new frame receiver wrapping `inner`.
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            buf: BytesMut::with_capacity(INITIAL_CAPACITY),
        }
    }
}

impl<R: Read> LMSRecv<R> {
    /// Block until at least one [`ServerMessage`] is recived.
    /// Because multiple messages can be received the return value
    /// is a vector of `ServerMessage`s.
    /// A failure to read will return [`std::io::Error`], and this
    /// usually indicates a server disconnection.
    pub fn recv(&mut self) -> std::io::Result<ServerMessages> {
        let mut src = [0u8; INITIAL_CAPACITY];

        loop {
            let bytes_read = loop {
                match self.inner.read(&mut src) {
                    Ok(0) => {
                        return Err(Error::new(
                            ErrorKind::ConnectionReset,
                            "Server connection reset",
                        ));
                    }

                    Ok(n) => break n,

                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }

                    Err(e) if e.kind() == ErrorKind::Interrupted => {
                        continue;
                    }

                    Err(e) => {
                        return Err(Error::new(
                            e.kind(),
                            format!("Failed to read from stream: {}", e),
                        ));
                    }
                }
            };

            self.buf.extend_from_slice(&src[..bytes_read]);
            match self.decode() {
                Ok(Some(item)) => return Ok(item),
                Ok(None) => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn decode(&mut self) -> std::io::Result<Option<ServerMessages>> {
        if self.buf.len() <= 2 {
            return Ok(None);
        };

        let mut messages = Vec::new();
        while self.buf.len() > 2 {
            let frame_size = u16::from_be_bytes([self.buf[0], self.buf[1]]) as usize;

            if self.buf.len() < frame_size + 2 {
                if self.buf.capacity() < frame_size + 2 {
                    self.buf.reserve(frame_size);
                }

                if messages.is_empty() {
                    // Not enough data for a complete message, wait for more
                    return Ok(None);
                } else {
                    // We have some messages, but not enough data for the next one
                    break;
                }
            }

            self.buf.advance(2);
            let msg = self.buf.split_to(frame_size);

            match msg.into() {
                ServerMessage::Error => {
                    self.buf.clear();
                    return Err(Error::new(ErrorKind::InvalidData, "Server data corrupted"));
                }

                msg => {
                    messages.push(msg);
                }
            }
        }

        Ok(Some(messages))
    }
}

/// A writer which takes `ClientMessage` values and encodes them into
/// LMS Protocol frames.
pub struct LMSSend<W> {
    inner: W,
}

impl<W> LMSSend<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> LMSSend<W> {
    /// Send a [`ClientMessage`] to the LMS. Returns `Ok(())` on success,
    /// [`std::io::Error`] on failure.
    pub fn send(&mut self, msg: ClientMessage) -> std::io::Result<()> {
        let mut dst: Bytes = msg.into();

        loop {
            match self.inner.write(&dst[..]) {
                Ok(0) => {
                    return Err(Error::new(
                        ErrorKind::ConnectionReset,
                        "Server connection reset",
                    ));
                }

                Ok(n) => {
                    if n < dst.len() {
                        dst.advance(n);
                    } else {
                        break;
                    }
                }

                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }

                Err(e) if e.kind() == ErrorKind::Interrupted => {}

                Err(e) => {
                    return Err(Error::new(
                        e.kind(),
                        format!("Failed to write to stream: {}", e),
                    ));
                }
            }
        }

        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send() {
        let out_msg = ClientMessage::Name(String::from("abc"));
        let mut sender = LMSSend::new(Vec::new());
        _ = sender.send(out_msg);

        assert_eq!(&sender.inner, &[83, 69, 84, 68, 0, 0, 0, 4, 0, 97, 98, 99]);
    }

    #[test]
    fn test_receive_single() {
        let in_msg = [0u8, 6, b'a', b'u', b'd', b'e', 0, 1];
        let mut receiver = LMSRecv::new(&in_msg[..]);
        let rx = receiver.recv().unwrap();

        assert_eq!(rx.len(), 1);
        assert!(matches!(rx[0], ServerMessage::Enable(false, true)));
    }

    #[test]
    fn test_receive_multi() {
        let in_msg = [
            0u8, 6, b'a', b'u', b'd', b'e', 0, 1, 0, 22, b'a', b'u', b'd', b'g', 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 128, 0,
        ];
        let mut receiver = LMSRecv::new(&in_msg[..]);
        let rx = receiver.recv().unwrap();

        assert_eq!(rx.len(), 2);
        assert!(
            matches!(rx[0], ServerMessage::Enable(false, true))
                && matches!(rx[1], ServerMessage::Gain(1.0, 0.5))
        );
    }
}
