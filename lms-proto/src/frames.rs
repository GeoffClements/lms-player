use std::{
    io::{Error, ErrorKind, Read, Write},
    time::Duration,
};

use bytes::{Buf, Bytes, BytesMut};

use crate::messages::{ClientMessage, ServerMessage, ServerMessages};

const INITIAL_CAPACITY: usize = 4 * 1024;

pub struct LmsRecv<R> {
    inner: R,
    buf: BytesMut,
}

impl<R> LmsRecv<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            buf: BytesMut::with_capacity(INITIAL_CAPACITY),
        }
    }
}

impl<R: Read> LmsRecv<R> {
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

pub struct LmsSend<W> {
    inner: W,
}

impl<W> LmsSend<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> LmsSend<W> {
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
