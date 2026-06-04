mod buffer;
mod capability;
mod discovery;
mod frames;
mod messages;
mod proto;
mod status;

pub use buffer::SlimBuffer;
pub use capability::Capability;
pub use discovery::discover;
pub use messages::{
    AutoStart, ClientMessage, Format, PcmChannels, PcmSampleRate, PcmSampleSize, ServerMessage,
};
pub use proto::Hello;
pub use status::{StatusCode, StatusData};

pub const SLIM_PORT: u16 = 3483;
