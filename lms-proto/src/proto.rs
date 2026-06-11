//! High-level protocol helpers.
//!
//! The `Hello` builder simplifies constructing the initial HELO message and
//! connecting to an LMS instance. It returns a pair of frame-oriented helpers
//! (`LMSRecv` and `LMSSend`) ready for use.
use std::net::{TcpStream, ToSocketAddrs};

use mac_address::MacAddress;

use crate::{
    Capability,
    capability::CapList,
    frames::{LMSRecv, LMSSend},
    messages::ClientMessage,
};

/// Builder for the HELO/initial connection sequence.
///
/// `Hello` collects the fields required by the Slim Protocol HELO message and
/// provides a `connect` method that opens a TCP connection, sends the HELO
/// frame, and returns framed reader/writer helpers.
///
/// Note: that the two `char`s in the `language` field should be ASCII.
#[derive(Default)]
pub struct Hello {
    device_id: u8,
    revision: u8,
    mac: MacAddress,
    uuid: [u8; 16],
    wlan_channel_list: u16,
    bytes_received: u64,
    language: [char; 2],
    reconnect: bool,
    caps: CapList,
}

impl Hello {
    /// Create a new empty `Hello` builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the device id field.
    pub fn device_id(mut self, device_id: u8) -> Self {
        self.device_id = device_id;
        self
    }

    /// Set the revision field.
    pub fn revision(mut self, revision: u8) -> Self {
        self.revision = revision;
        self
    }

    /// Set the MAC address to report to the server.
    pub fn mac(mut self, mac: MacAddress) -> Self {
        self.mac = mac;
        self
    }

    /// Set the client's UUID.
    pub fn uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    /// Set the WLAN channel list.
    pub fn wlan_channel_list(mut self, wlan_channel_list: u16) -> Self {
        self.wlan_channel_list = wlan_channel_list;
        self
    }

    /// Set the number of bytes already received by the client (for resume).
    pub fn bytes_received(mut self, bytes_received: u64) -> Self {
        self.bytes_received = bytes_received;
        self
    }

    /// Set the language (two-character code in ASCII) reported to the server.
    pub fn language(mut self, language: [char; 2]) -> Self {
        self.language = language;
        self
    }

    pub fn reconnect(mut self, reconnect: bool) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Set the capabilities the client will announce.
    pub fn capabilities(mut self, capabilities: Vec<Capability>) -> Self {
        self.caps = CapList::new(capabilities);
        self
    }

    /// Connect to an LMS server and perform the initial HELO handshake.
    ///
    /// On success returns a `(LMSRecv, LMSSend)` pair ready for receiving and
    /// sending framed messages.
    pub fn connect<A: ToSocketAddrs>(
        self,
        socket: A,
    ) -> std::io::Result<(LMSRecv<TcpStream>, LMSSend<TcpStream>)> {
        let stream = TcpStream::connect(socket)?;
        stream.set_nodelay(true)?;

        let wlan_channel_list = if self.reconnect {
            self.wlan_channel_list & 0x0400
        } else {
            self.wlan_channel_list
        };

        let helo = ClientMessage::Helo {
            device_id: self.device_id,
            revision: self.revision,
            mac: self.mac,
            uuid: self.uuid,
            wlan_channel_list,
            bytes_received: self.bytes_received,
            language: self.language,
            capabilities: self.caps.to_string(),
        };

        let rx = LMSRecv::new(stream.try_clone()?);
        let mut tx = LMSSend::new(stream);
        tx.send(helo)?;

        Ok((rx, tx))
    }
}
