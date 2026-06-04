use std::net::{TcpStream, ToSocketAddrs};

use mac_address::MacAddress;

use crate::{
    Capability,
    capability::CapList,
    frames::{LmsRecv, LmsSend},
    messages::ClientMessage,
};

#[derive(Default)]
pub struct Hello {
    device_id: u8,
    revision: u8,
    mac: MacAddress,
    uuid: [u8; 16],
    wlan_channel_list: u16,
    bytes_received: u64,
    language: [char; 2],
    caps: CapList,
}

impl Hello {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn device_id(mut self, device_id: u8) -> Self {
        self.device_id = device_id;
        self
    }

    pub fn revision(mut self, revision: u8) -> Self {
        self.revision = revision;
        self
    }

    pub fn mac(mut self, mac: MacAddress) -> Self {
        self.mac = mac;
        self
    }

    pub fn uuid(mut self, uuid: [u8; 16]) -> Self {
        self.uuid = uuid;
        self
    }

    pub fn wlan_channel_list(mut self, wlan_channel_list: u16) -> Self {
        self.wlan_channel_list = wlan_channel_list;
        self
    }

    pub fn bytes_received(mut self, bytes_received: u64) -> Self {
        self.bytes_received = bytes_received;
        self
    }

    pub fn language(mut self, language: [char; 2]) -> Self {
        self.language = language;
        self
    }

    pub fn capabilities(mut self, capabilities: Vec<Capability>) -> Self {
        self.caps = CapList::new(capabilities);
        self
    }

    pub fn connect<A: ToSocketAddrs>(
        self,
        socket: A,
    ) -> std::io::Result<(LmsRecv<TcpStream>, LmsSend<TcpStream>)> {
        let stream = TcpStream::connect(socket)?;
        stream.set_nodelay(true)?;

        let helo = ClientMessage::Helo {
            device_id: self.device_id,
            revision: self.revision,
            mac: self.mac,
            uuid: self.uuid,
            wlan_channel_list: self.wlan_channel_list,
            bytes_received: self.bytes_received,
            language: self.language,
            capabilities: self.caps.to_string(),
        };

        let rx = LmsRecv::new(stream.try_clone()?);
        let mut tx = LmsSend::new(stream);
        tx.send(helo)?;

        Ok((rx, tx))
    }
}
