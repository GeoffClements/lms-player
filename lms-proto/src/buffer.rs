//! Utilities for buffered reading tailored to the LMS Protocol.
//!
//! This module exposes `SlimBuffer`, a wrapper over any `Read` source that
//! spawns a background thread to keep an internal buffer filled, and updates a
//! shared `StatusData` instance as bytes are consumed. `SlimBuffer` is
//! intended for use by players that need to report buffer fullness and
//! throughput back to the server.
//!
//! `SlimBuffer` implements `Read` and `BufRead` so it can be used wherever a
//! buffered reader is expected. Dropping it stops and joins the fill thread.
use std::{
    collections::VecDeque,
    io::{self, BufRead, Read},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::{self, JoinHandle},
};

use crate::status::StatusData;

type MaybeCallback = Option<Box<dyn FnMut() + Send + Sync + 'static>>;

const DEFAULT_CAPACITY: usize = 256 * 1024;
/// Size of each read the fill thread issues against the source.
const READ_CHUNK: usize = 8 * 1024;
/// Max bytes moved into the consumer-side staging buffer at once.
const STAGING_SIZE: usize = 8 * 1024;

struct State {
    buf: VecDeque<u8>,
    eof: bool,
    error: Option<io::Error>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Signalled when data, EOF or an error becomes available to the reader.
    readable: Condvar,
    /// Signalled when space frees up, or on shutdown.
    writable: Condvar,
    capacity: usize,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Don't propagate poisoning; this matters inside `Drop`.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A buffered reader that keeps itself filled from a background thread and
/// updates `StatusData` as data is read.
///
/// The fill thread reads ahead until the buffer is full, then sleeps until the
/// consumer makes room. An optional callback is invoked (once, on the fill
/// thread) when the buffered amount first reaches the threshold, or when the
/// source ends or errors before reaching it.
///
/// Dropping the `SlimBuffer` signals the thread to stop and joins it.
pub struct SlimBuffer {
    shared: Arc<Shared>,
    status: Arc<Mutex<StatusData>>,
    handle: Option<JoinHandle<()>>,
    /// Consumer-side staging area, so `fill_buf` can hand out a slice.
    staging: Vec<u8>,
    pos: usize,
}

impl SlimBuffer {
    pub fn new<R>(
        inner: R,
        status: Arc<Mutex<StatusData>>,
        threshold: u32,
        threshold_cb: MaybeCallback,
    ) -> Self
    where
        R: Read + Send + 'static,
    {
        Self::with_capacity(DEFAULT_CAPACITY, inner, status, threshold, threshold_cb)
    }

    /// `capacity` is raised to `threshold` if smaller, otherwise the
    /// threshold could never be reached.
    pub fn with_capacity<R>(
        capacity: usize,
        inner: R,
        status: Arc<Mutex<StatusData>>,
        threshold: u32,
        threshold_cb: MaybeCallback,
    ) -> Self
    where
        R: Read + Send + 'static,
    {
        let capacity = capacity.max(threshold as usize).max(1);

        if let Ok(mut status) = status.lock() {
            status.set_buffer_size(capacity as u32);
        }

        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                buf: VecDeque::with_capacity(capacity),
                eof: false,
                error: None,
                shutdown: false,
            }),
            readable: Condvar::new(),
            writable: Condvar::new(),
            capacity,
        });

        let handle = thread::Builder::new()
            .name("slim-buf-fill".into())
            .spawn({
                let shared = Arc::clone(&shared);
                let status = Arc::clone(&status);
                move || fill_loop(inner, shared, status, threshold, threshold_cb)
            })
            .expect("failed to spawn SlimBuffer fill thread");

        Self {
            shared,
            status,
            handle: Some(handle),
            staging: Vec::with_capacity(STAGING_SIZE),
            pos: 0,
        }
    }

    /// Bytes currently held in the shared buffer (excludes the small
    /// consumer-side staging area).
    pub fn buffered(&self) -> usize {
        self.shared.lock().buf.len()
    }

    /// Ensure the staging buffer has data unless the source is exhausted.
    fn refill(&mut self) -> io::Result<()> {
        if self.pos < self.staging.len() {
            return Ok(());
        }
        self.staging.clear();
        self.pos = 0;

        let mut st = self.shared.lock();
        while st.buf.is_empty() && !st.eof {
            st = self
                .shared
                .readable
                .wait(st)
                .unwrap_or_else(|e| e.into_inner());
        }

        if st.buf.is_empty() {
            // EOF or error with nothing left to hand out.
            return match st.error.take() {
                Some(e) => Err(e),
                None => Ok(()),
            };
        }

        let n = st.buf.len().min(STAGING_SIZE);
        self.staging.extend(st.buf.drain(..n));

        if let Ok(mut status) = self.status.lock() {
            status.set_fullness(st.buf.len() as u32);
        }
        drop(st);
        self.shared.writable.notify_all();
        Ok(())
    }

    fn advance(&mut self, n: usize) {
        self.pos += n;
        if let Ok(mut status) = self.status.lock() {
            status.add_bytes_received(n as u64);
        }
    }
}

fn fill_loop<R: Read>(
    mut source: R,
    shared: Arc<Shared>,
    status: Arc<Mutex<StatusData>>,
    threshold: u32,
    mut threshold_cb: MaybeCallback,
) {
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut signalled = false;

    loop {
        // Sleep until there is room, or we're told to stop.
        let want = {
            let mut st = shared.lock();
            while !st.shutdown && st.buf.len() >= shared.capacity {
                st = shared.writable.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            if st.shutdown {
                return;
            }
            (shared.capacity - st.buf.len()).min(chunk.len())
        };

        // Read without holding the lock so the consumer never waits on I/O.
        let result = source.read(&mut chunk[..want]);

        let mut st = shared.lock();
        if st.shutdown {
            return;
        }

        let reached = match result {
            Ok(0) => {
                st.eof = true;
                true
            }
            Ok(n) => {
                st.buf.extend(&chunk[..n]);
                if let Ok(mut status) = status.lock() {
                    status.set_fullness(st.buf.len() as u32);
                }
                st.buf.len() >= threshold as usize
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => {
                st.error = Some(e);
                st.eof = true;
                true
            }
        };

        let finished = st.eof;
        drop(st);
        shared.readable.notify_all();

        if reached && !signalled {
            signalled = true;
            if let Some(cb) = threshold_cb.as_mut() {
                cb();
            }
        }

        if finished {
            return;
        }
    }
}

impl Read for SlimBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.refill()?;

        let avail = &self.staging[self.pos..];
        let n = avail.len().min(buf.len());
        buf[..n].copy_from_slice(&avail[..n]);
        self.advance(n);
        Ok(n)
    }
}

impl BufRead for SlimBuffer {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.refill()?;
        Ok(&self.staging[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        let n = amt.min(self.staging.len() - self.pos);
        self.advance(n);
    }
}

impl Drop for SlimBuffer {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.writable.notify_all();
        self.shared.readable.notify_all();

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{
        io::Cursor,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    fn test_data(len: usize) -> Vec<u8> {
        (0u8..255).cycle().take(len).collect()
    }

    fn wait_until(mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn fills_ahead_of_reader() {
        const BUFLEN: usize = 1024;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source = test_data(BUFLEN);

        let sb = SlimBuffer::new(Cursor::new(source), status, 2, None);
        wait_until(|| sb.buffered() == BUFLEN);
    }

    #[test]
    fn reads_everything_in_order() {
        const BUFLEN: usize = 1024 * 20;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source = test_data(BUFLEN);

        let mut sb = SlimBuffer::with_capacity(4096, Cursor::new(source.clone()), status, 2, None);

        let mut out = Vec::new();
        sb.read_to_end(&mut out).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn callback_fires_once_when_threshold_reached() {
        const BUFLEN: usize = 1024 * 2;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let source = test_data(BUFLEN);

        let count = Arc::new(AtomicUsize::new(0));
        let count_ref = Arc::clone(&count);
        let (tx, rx) = mpsc::channel();

        let mut sb = SlimBuffer::new(
            Cursor::new(source),
            status,
            512,
            Some(Box::new(move || {
                count_ref.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(());
            })),
        );

        rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let mut out = Vec::new();
        sb.read_to_end(&mut out).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn bytes_received_is_tracked_for_both_read_and_consume() {
        const BUFLEN: usize = 100;

        let status = Arc::new(Mutex::new(StatusData::default()));
        let mut sb = SlimBuffer::new(Cursor::new(test_data(BUFLEN)), Arc::clone(&status), 0, None);

        let mut buf = [0u8; 10];
        sb.read_exact(&mut buf).unwrap();
        let n = sb.fill_buf().unwrap().len();
        sb.consume(n);

        let mut rest = Vec::new();
        sb.read_to_end(&mut rest).unwrap();
        assert_eq!(10 + n + rest.len(), BUFLEN);
    }

    /// An endless source that records whether it is still alive.
    struct Endless(#[allow(dead_code)] Arc<()>);

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(0xAB);
            Ok(buf.len())
        }
    }

    #[test]
    fn drop_stops_the_fill_thread() {
        let status = Arc::new(Mutex::new(StatusData::default()));
        let token = Arc::new(());

        let sb = SlimBuffer::with_capacity(4096, Endless(Arc::clone(&token)), status, 0, None);
        wait_until(|| sb.buffered() == 4096);
        assert_eq!(Arc::strong_count(&token), 2);

        // The thread is parked on a full buffer; drop must wake and join it.
        drop(sb);

        // The source is owned by the thread, so it's only released if the
        // thread actually exited.
        assert_eq!(Arc::strong_count(&token), 1);
    }
}
