/// Shared global player state, accessed by the decoder, audio outputs, and message handlers.
///
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crossbeam::atomic::AtomicCell;
use lms_proto::StatusData;

/// Per-channel linear volume, square-root-encoded (to give a perceptually linear taper).
/// Index 0 = left, 1 = right.
pub static VOLUME: LazyLock<Mutex<Vec<f32>>> = LazyLock::new(|| Mutex::new(vec![1.0, 1.0]));

/// Requested skip-ahead amount signalled by the LMS Skip command.
pub static SKIP: LazyLock<AtomicCell<Duration>> = LazyLock::new(|| AtomicCell::new(Duration::ZERO));

/// Shared LMS status block, updated by message handlers and read by the timer tick.
pub static STATUS: LazyLock<Arc<Mutex<StatusData>>> =
    LazyLock::new(|| Arc::new(Mutex::new(StatusData::default())));
