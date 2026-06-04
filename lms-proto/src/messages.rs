use std::{net::Ipv4Addr, time::Duration};

use bitflags::bitflags;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use mac_address::MacAddress;

use crate::status::StatusData;

/// A type that describes all messages that are sent from the client to
/// the server.
#[derive(Debug)]
pub enum ClientMessage {
    Helo {
        device_id: u8,
        revision: u8,
        mac: MacAddress,
        uuid: [u8; 16],
        wlan_channel_list: u16,
        bytes_received: u64,
        language: [char; 2],
        capabilities: String,
    },
    Stat {
        event_code: String,
        stat_data: StatusData,
    },
    Bye(u8),
    Name(String),
}

impl From<ClientMessage> for Bytes {
    fn from(src: ClientMessage) -> Bytes {
        const FRAMESIZE: usize = 1024;

        let mut msg = Vec::with_capacity(FRAMESIZE + 8);
        let mut frame_size = Vec::with_capacity(4);
        let mut frame = Vec::with_capacity(FRAMESIZE);

        match src {
            ClientMessage::Helo {
                device_id,
                revision,
                mac,
                uuid,
                wlan_channel_list,
                bytes_received,
                language,
                capabilities,
            } => {
                msg.put("HELO".as_bytes());
                frame.put_u8(device_id);
                frame.put_u8(revision);
                frame.put(mac.bytes().as_ref());
                frame.put(uuid.as_ref());
                frame.put_u16(wlan_channel_list);
                frame.put_u64(bytes_received);
                frame.put(
                    language
                        .iter()
                        .map(|c| *c as u8)
                        .collect::<Vec<u8>>()
                        .as_ref(),
                );
                frame.put(capabilities.as_bytes());
            }

            ClientMessage::Bye(val) => {
                msg.put("BYE!".as_bytes());
                frame.put_u8(val);
            }
            ClientMessage::Stat {
                event_code,
                stat_data,
            } => {
                msg.put("STAT".as_bytes());
                frame.put(event_code.as_bytes());
                frame.put_u8(stat_data.crlf);
                frame.put_u16(0);
                frame.put_u32(stat_data.buffer_size);
                frame.put_u32(stat_data.fullness);
                frame.put_u64(stat_data.bytes_received);
                frame.put_u16(stat_data.sig_strength);
                frame.put_u32(stat_data.jiffies.as_millis().try_into().unwrap_or_default());
                frame.put_u32(stat_data.output_buffer_size);
                frame.put_u32(stat_data.output_buffer_fullness);
                frame.put_u32(stat_data.elapsed_seconds);
                frame.put_u16(stat_data.voltage);
                frame.put_u32(stat_data.elapsed_milliseconds);
                frame.put_u32(
                    stat_data
                        .timestamp
                        .as_millis()
                        .try_into()
                        .unwrap_or_default(),
                );
                frame.put_u16(stat_data.error_code);
            }

            ClientMessage::Name(name) => {
                msg.put("SETD".as_bytes());
                frame.put_u8(0);
                frame.put(name.as_bytes());
            }
        }

        frame_size.put_u32(frame.len() as u32);
        msg.append(&mut frame_size);
        msg.append(&mut frame);

        msg.into()
    }
}

#[derive(Debug, PartialEq)]
pub enum AutoStart {
    None,
    Auto,
    Direct,
    AutoDirect,
}

#[derive(Debug, PartialEq)]
pub enum Format {
    Pcm,
    Mp3,
    Flac,
    Wma,
    Ogg,
    Aac,
    Alac,
}
#[derive(Debug, PartialEq)]
pub enum PcmSampleSize {
    Eight,
    Sixteen,
    Twenty,
    ThirtyTwo,
    SelfDescribing,
}

#[derive(Debug, PartialEq)]
pub enum PcmSampleRate {
    Rate(u32),
    SelfDescribing,
}

#[derive(Debug, PartialEq)]
pub enum PcmChannels {
    Mono,
    Stereo,
    SelfDescribing,
}

#[derive(Debug, PartialEq)]
pub enum PcmEndian {
    Big,
    Little,
    SelfDescribing,
}

#[derive(Debug, PartialEq)]
pub enum SpdifEnable {
    Auto,
    On,
    Off,
}

#[derive(Debug, PartialEq)]
pub enum TransType {
    None,
    Crossfade,
    FadeIn,
    FadeOut,
    FadeInOut,
}

bitflags! {
    #[derive(Debug, Default, PartialEq)]
    pub struct StreamFlags: u8 {
        const INF_LOOP = 0b1000_0000;
        const NO_RESTART_DECODER = 0b0100_0000;
        const INVERT_POLARITY_LEFT = 0b0000_0001;
        const INVERT_POLARITY_RIGHT = 0b0000_0010;
    }
}

/// A type that describes all messages that are sent from the server to
/// the client.
#[derive(Debug)]
pub enum ServerMessage {
    Serv {
        ip_address: Ipv4Addr,
        sync_group_id: Option<String>,
    },
    Status(Duration),
    Stream {
        autostart: AutoStart,
        format: Format,
        pcmsamplesize: PcmSampleSize,
        pcmsamplerate: PcmSampleRate,
        pcmchannels: PcmChannels,
        pcmendian: PcmEndian,
        threshold: u32,
        spdif_enable: SpdifEnable,
        trans_period: Duration,
        trans_type: TransType,
        flags: StreamFlags,
        output_threshold: Duration,
        replay_gain: f64,
        server_port: u16,
        server_ip: Ipv4Addr,
        http_headers: Option<String>,
    },
    Gain(f64, f64),
    Enable(bool, bool),
    Flush,
    Stop,
    Pause(Duration),
    Unpause(Duration),
    Queryname,
    Setname(String),
    DisableDac,
    Skip(Duration),
    Unrecognised(String),
    Error,
}

pub type ServerMessages = Vec<ServerMessage>;

impl From<BytesMut> for ServerMessage {
    fn from(mut src: BytesMut) -> ServerMessage {
        const GAIN_FACTOR: f64 = 65536.0;

        let msg = String::from_utf8(src.split_to(4).to_vec()).unwrap_or_default();
        let mut buf = src; //.split();

        match msg.as_str() {
            "serv" => {
                if buf.len() < 4 {
                    return ServerMessage::Error;
                }

                let ip_addr = Ipv4Addr::from(buf.split_to(4).get_u32());
                let sync_group = if !buf.is_empty() {
                    Some(buf.into_iter().map(|c| c as char).collect::<String>())
                } else {
                    None
                };
                ServerMessage::Serv {
                    ip_address: ip_addr,
                    sync_group_id: sync_group,
                }
            }

            "strm" => {
                if buf.len() < 24 {
                    return ServerMessage::Error;
                }

                match buf.split_to(1)[0] as char {
                    't' => {
                        buf.advance(14);
                        let timestamp = Duration::from_millis(buf.get_u32() as u64);
                        ServerMessage::Status(timestamp)
                    }

                    's' => {
                        let autostart = match buf.split_to(1)[0] as char {
                            '0' => AutoStart::None,
                            '1' => AutoStart::Auto,
                            '2' => AutoStart::Direct,
                            '3' => AutoStart::AutoDirect,
                            _ => return ServerMessage::Error,
                        };
                        let format = match buf.split_to(1)[0] as char {
                            'p' => Format::Pcm,
                            'm' => Format::Mp3,
                            'f' => Format::Flac,
                            'w' => Format::Wma,
                            'o' => Format::Ogg,
                            'a' => Format::Aac,
                            'l' => Format::Alac,
                            _ => return ServerMessage::Error,
                        };

                        let pcmsamplesize = match buf.split_to(1)[0] as char {
                            '0' => PcmSampleSize::Eight,
                            '1' => PcmSampleSize::Sixteen,
                            '2' => PcmSampleSize::Twenty,
                            '3' => PcmSampleSize::ThirtyTwo,
                            '?' => PcmSampleSize::SelfDescribing,
                            _ => return ServerMessage::Error,
                        };

                        let pcmsamplerate = match buf.split_to(1)[0] as char {
                            '0' => PcmSampleRate::Rate(11_000),
                            '1' => PcmSampleRate::Rate(22_000),
                            '2' => PcmSampleRate::Rate(32_000),
                            '3' => PcmSampleRate::Rate(44_100),
                            '4' => PcmSampleRate::Rate(48_000),
                            '5' => PcmSampleRate::Rate(8_000),
                            '6' => PcmSampleRate::Rate(12_000),
                            '7' => PcmSampleRate::Rate(16_000),
                            '8' => PcmSampleRate::Rate(24_000),
                            '9' => PcmSampleRate::Rate(96_000),
                            '?' => PcmSampleRate::SelfDescribing,
                            _ => return ServerMessage::Error,
                        };

                        let pcmchannels = match buf.split_to(1)[0] as char {
                            '1' => PcmChannels::Mono,
                            '2' => PcmChannels::Stereo,
                            '?' => PcmChannels::SelfDescribing,
                            _ => return ServerMessage::Error,
                        };

                        let pcmendian = match buf.split_to(1)[0] as char {
                            '0' => PcmEndian::Big,
                            '1' => PcmEndian::Little,
                            '?' => PcmEndian::SelfDescribing,
                            _ => return ServerMessage::Error,
                        };

                        let threshold = buf.split_to(1)[0] as u32 * 1024u32;

                        let spdif_enable = match buf.split_to(1)[0] {
                            0 => SpdifEnable::Auto,
                            1 => SpdifEnable::On,
                            2 => SpdifEnable::Off,
                            _ => return ServerMessage::Error,
                        };

                        let trans_period = Duration::from_secs(buf.split_to(1)[0] as u64);

                        let trans_type = match buf.split_to(1)[0] as char {
                            '0' => TransType::None,
                            '1' => TransType::Crossfade,
                            '2' => TransType::FadeIn,
                            '3' => TransType::FadeOut,
                            '4' => TransType::FadeInOut,
                            _ => return ServerMessage::Error,
                        };

                        let flags = StreamFlags::from_bits(buf.split_to(1)[0]).unwrap_or_default();

                        let output_threshold =
                            Duration::from_millis(buf.split_to(1)[0] as u64 * 10);

                        buf.advance(1);

                        let replay_gain = buf.split_to(4).get_u32() as f64 / GAIN_FACTOR;

                        let server_port = buf.split_to(2).get_u16();

                        let server_ip = Ipv4Addr::from(buf.split_to(4).get_u32());

                        let http_headers = if !buf.is_empty() {
                            Some(String::from_utf8_lossy(&buf).to_string())
                        } else {
                            None
                        };

                        ServerMessage::Stream {
                            autostart,
                            format,
                            pcmsamplesize,
                            pcmsamplerate,
                            pcmchannels,
                            pcmendian,
                            threshold,
                            spdif_enable,
                            trans_period,
                            trans_type,
                            flags,
                            output_threshold,
                            replay_gain,
                            server_port,
                            server_ip,
                            http_headers,
                        }
                    }

                    'q' => ServerMessage::Stop,

                    'f' => ServerMessage::Flush,

                    'p' => {
                        buf.advance(13);
                        let timestamp = buf.get_u32();
                        ServerMessage::Pause(Duration::from_millis(timestamp as u64))
                    }

                    'u' => {
                        buf.advance(13);
                        let timestamp = buf.get_u32();
                        ServerMessage::Unpause(Duration::from_millis(timestamp as u64))
                    }

                    'a' => {
                        buf.advance(13);
                        let timestamp = buf.get_u32();
                        ServerMessage::Skip(Duration::from_millis(timestamp as u64))
                    }

                    cmd => {
                        let mut msg = msg.to_owned();
                        msg.push('_');
                        msg.push(cmd);
                        ServerMessage::Unrecognised(msg)
                    }
                }
            }

            "aude" => {
                if buf.len() < 2 {
                    return ServerMessage::Error;
                }

                let (spdif, dac) = (buf[0] != 0, buf[1] != 0);
                ServerMessage::Enable(spdif, dac)
            }

            "audg" => {
                if buf.len() < 18 {
                    return ServerMessage::Error;
                }

                buf.advance(10);
                let left = buf.split_to(4).get_u32() as f64 / GAIN_FACTOR;
                let right = buf.split_to(4).get_u32() as f64 / GAIN_FACTOR;
                ServerMessage::Gain(left, right)
            }

            "setd" => {
                if buf.is_empty() {
                    return ServerMessage::Error;
                }

                match buf.split_to(1)[0] {
                    0 => {
                        if buf.is_empty() {
                            ServerMessage::Queryname
                        } else {
                            let name = String::from_utf8(buf[..buf.len() - 1].to_vec())
                                .unwrap_or_default();
                            ServerMessage::Setname(name)
                        }
                    }

                    4 => ServerMessage::DisableDac,

                    v => ServerMessage::Unrecognised(format!("This SETD is unused: {}", v)),
                }
            }

            cmd => ServerMessage::Unrecognised(cmd.to_owned()),
        }
    }
}
