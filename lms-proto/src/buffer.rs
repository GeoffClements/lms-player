/// Used to wrap around a reader.
/// Keeps the associates status data updated
use std::{
    io::{BufRead, BufReader, Read},
    sync::{Arc, Mutex},
};

use crate::status::StatusData;

type MaybeCallback = Option<Box<dyn FnMut() + Send + Sync + 'static>>;

pub struct SlimBuffer<R> {
    inner: BufReader<R>,
    status: Arc<Mutex<StatusData>>,
    threshold: u32,
    threshold_cb: MaybeCallback,
    prebuf: Vec<u8>,
}

impl<R> SlimBuffer<R>
where
    R: Read,
{
    pub fn new(
        inner: R,
        status: Arc<Mutex<StatusData>>,
        threshold: u32,
        threshold_cb: MaybeCallback,
    ) -> Self {
        let buf = BufReader::new(inner);
        if let Ok(mut status) = status.lock() {
            status.set_buffer_size(buf.capacity() as u32);
        }

        let mut this: SlimBuffer<R> = Self {
            inner: buf,
            status,
            threshold,
            threshold_cb,
            prebuf: Vec::with_capacity(255 * 1024),
        };

        this.pre_buf();
        this
    }

    pub fn with_capacity(
        capacity: usize,
        inner: R,
        status: Arc<Mutex<StatusData>>,
        threshold: u32,
        threshold_cb: MaybeCallback,
    ) -> Self {
        let buf = BufReader::with_capacity(capacity, inner);
        if let Ok(mut status) = status.lock() {
            status.set_buffer_size(buf.capacity() as u32);
        }

        let mut this: SlimBuffer<R> = Self {
            inner: buf,
            status,
            threshold,
            threshold_cb,
            prebuf: Vec::with_capacity(255 * 1024),
        };

        this.pre_buf();
        this
    }

    fn pre_buf(&mut self) {
        let mut buf = [0u8; 1024];
        while self.prebuf.len() < self.threshold as usize {
            if let Ok(n) = self.inner.read(&mut buf) {
                if n == 0 {
                    break;
                }
                self.prebuf.extend_from_slice(&buf[..n]);
            } else {
                break;
            }
        }

        if let Some(callback) = &mut self.threshold_cb {
            callback();
        }
    }
}

impl<R> Read for SlimBuffer<R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_read = if self.prebuf.len() > 0 {
            let n_bytes = (&self.prebuf[..]).read(buf)?;
            self.prebuf.drain(..n_bytes);
            n_bytes
        } else {
            self.inner.read(buf)?
        };
        if let Ok(mut status) = self.status.lock() {
            status.add_bytes_received(bytes_read as u64);
            status.set_fullness(self.inner.buffer().len() as u32);
        }
        Ok(bytes_read)
    }
}

impl<R> BufRead for SlimBuffer<R>
where
    R: Read,
{
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.prebuf.len() > 0 {
            return Ok(&self.prebuf[..]);
        }
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        let mut amt_left = amt;

        if self.prebuf.len() > 0 {
            let n = amt.min(self.prebuf.len());
            self.prebuf.drain(..n);
            amt_left -= n;
        }
        self.inner.consume(amt_left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::RwLock;

    #[test]
    fn prebuf() {
        const BUFLEN: usize = 1024;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source: Vec<u8> = (0u8..255).into_iter().cycle().take(BUFLEN).collect();

        let sb = SlimBuffer::new(&source[..], status, 2, None);
        assert_eq!(sb.prebuf, source);
        assert!(sb.prebuf.len() == source.len());
    }

    #[test]
    fn prebuf_overfill() {
        const BUFLEN: usize = 1024 * 2;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source: Vec<u8> = (0u8..255).into_iter().cycle().take(BUFLEN).collect();

        let mut sb = SlimBuffer::new(&source[..], status, 2, None);

        let mut buf = vec![0u8; BUFLEN];
        let n = sb.read(&mut buf).unwrap();
        sb.read(&mut buf[n..]).unwrap();
        assert_eq!(buf, source);
        assert!(sb.prebuf.len() == 0);
    }

    #[test]
    fn callback() {
        const BUFLEN: usize = 1024 * 2;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source: Vec<u8> = (0u8..255).into_iter().cycle().take(BUFLEN).collect();

        let value = Arc::new(RwLock::new(0));
        let value_ref = value.clone();
        let mut sb = SlimBuffer::new(
            &source[..],
            status,
            2,
            Some(Box::new(move || {
                if let Ok(mut value) = value_ref.write() {
                    *value += 1;
                }
            })),
        );

        let mut buf = vec![0u8; BUFLEN];
        let n = sb.read(&mut buf).unwrap();
        sb.read(&mut buf[n..]).unwrap();

        let val = value.read().unwrap();
        assert!(*val == 1);
    }
}
