// mod buffer;
pub mod capability;
// mod codec;
mod discovery;
mod frames;
pub mod messages;
pub mod proto;
pub mod status;

pub use capability::Capability;
pub use discovery::discover;
pub use messages::{ClientMessage, ServerMessage};
pub use proto::Hello;
pub use status::StatusData;

pub const SLIM_PORT: u16 = 3483;
