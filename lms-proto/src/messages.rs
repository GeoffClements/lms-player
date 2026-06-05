//! Message types and (de)serialization for the LMS Protocol.
//!
//! This module defines the `ClientMessage` and `ServerMessage` enums and
//! implements conversions to/from byte buffers used on the wire.
use std::{net::Ipv4Addr, time::Duration};

use bitflags::bitflags;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use mac_address::MacAddress;

use crate::status::StatusData;

/// Messages sent from the client to the server.
///
/// `ClientMessage` variants are converted into the length-prefixed binary
/// frames expected by LMS; the `From<ClientMessage> for Bytes` implementation
/// performs the serialization.
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
                msg.put_u8(device_id);
                msg.put_u8(revision);
                msg.put(mac.bytes().as_ref());
                msg.put(uuid.as_ref());
                msg.put_u16(wlan_channel_list);
                msg.put_u64(bytes_received);
                msg.put(
                    language
                        .iter()
                        .map(|c| *c as u8)
                        .collect::<Vec<u8>>()
                        .as_ref(),
                );
                msg.put(capabilities.as_bytes());
            }

            ClientMessage::Bye(val) => {
                msg.put("BYE!".as_bytes());
                msg.put_u8(val);
            }
            ClientMessage::Stat {
                event_code,
                stat_data,
            } => {
                msg.put("STAT".as_bytes());
                msg.put(event_code.as_bytes());
                msg.put_u8(stat_data.crlf);
                msg.put_u16(0);
                msg.put_u32(stat_data.buffer_size);
                msg.put_u32(stat_data.fullness);
                msg.put_u64(stat_data.bytes_received);
                msg.put_u16(stat_data.sig_strength);
                msg.put_u32(stat_data.jiffies.as_millis().try_into().unwrap_or_default());
                msg.put_u32(stat_data.output_buffer_size);
                msg.put_u32(stat_data.output_buffer_fullness);
                msg.put_u32(stat_data.elapsed_seconds);
                msg.put_u16(stat_data.voltage);
                msg.put_u32(stat_data.elapsed_milliseconds);
                msg.put_u32(
                    stat_data
                        .timestamp
                        .as_millis()
                        .try_into()
                        .unwrap_or_default(),
                );
                msg.put_u16(stat_data.error_code);
            }

            ClientMessage::Name(name) => {
                msg.put("SETD".as_bytes());
                msg.put_u8(0);
                msg.put(name.as_bytes());
            }
        }

        let mut msg_len = Vec::with_capacity(4);
        msg_len.put_u32((msg.len().saturating_sub(4)) as u32);
        msg.splice(4..4, msg_len);

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

/// Messages sent from the server to the client.
///
/// `ServerMessage` is produced by parsing incoming framed data. Unrecognized
/// or malformed messages are represented by the `Unrecognised` or `Error`
/// variants.
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
        let mut buf = src;

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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn send_helo() {
        let helo = ClientMessage::Helo {
            device_id: 0,
            revision: 1,
            mac: MacAddress::new([1, 2, 3, 4, 5, 6]),
            uuid: [7u8; 16],
            wlan_channel_list: 0x89AB,
            bytes_received: 1234,
            language: ['u', 'k'],
            capabilities: "abcd".to_owned(),
        };

        let buf: Bytes = helo.into();
        assert_eq!(
            &buf[..32],
            &[
                b'H', b'E', b'L', b'O', 0, 0, 0, 40, 0, 1, 1, 2, 3, 4, 5, 6, 7, 7, 7, 7, 7, 7, 7,
                7, 7, 7, 7, 7, 7, 7, 7, 7,
            ]
        );
        assert_eq!(
            &buf[32..],
            &[
                137, 171, 0, 0, 0, 0, 0, 0, 4, 210, b'u', b'k', b'a', b'b', b'c', b'd'
            ]
        );
    }

    #[test]
    fn send_bye() {
        let bye = ClientMessage::Bye(55);

        let buf: Bytes = bye.into();
        assert_eq!(&buf[..], &[b'B', b'Y', b'E', b'!', 0, 0, 0, 1, 55]);
    }

    #[test]
    fn send_stat() {
        let stat_data = StatusData {
            crlf: 0,
            buffer_size: 1234,
            fullness: 5678,
            bytes_received: 9123,
            sig_strength: 45,
            jiffies: Duration::from_millis(6789),
            output_buffer_size: 1234,
            output_buffer_fullness: 5678,
            elapsed_seconds: 9012,
            voltage: 3456,
            elapsed_milliseconds: 7890,
            timestamp: Duration::from_millis(1234),
            error_code: 5678,
            start: Instant::now(),
        };
        let stat = ClientMessage::Stat {
            event_code: "STMt".to_owned(),
            stat_data: stat_data,
        };

        let buf: Bytes = stat.into();
        assert_eq!(
            &buf[..32],
            &[
                b'S', b'T', b'A', b'T', 0, 0, 0, 53, b'S', b'T', b'M', b't', 0, 0, 0, 0, 0, 4, 210,
                0, 0, 22, 46, 0, 0, 0, 0, 0, 0, 35, 163, 0
            ]
        );
        assert_eq!(
            &buf[32..],
            &[
                45, 0, 0, 26, 133, 0, 0, 4, 210, 0, 0, 22, 46, 0, 0, 35, 52, 13, 128, 0, 0, 30,
                210, 0, 0, 4, 210, 22, 46
            ]
        );
    }

    #[test]
    fn send_name() {
        let name = ClientMessage::Name("BadBoy".to_owned());

        let buf: Bytes = name.into();
        assert_eq!(
            &buf[..],
            &[
                b'S', b'E', b'T', b'D', 0, 0, 0, 7, 0, b'B', b'a', b'd', b'B', b'o', b'y'
            ]
        );
    }

    #[test]
    fn recv_serv() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b'e', b'r', b'v', 172, 16, 1, 2, b's', b'y', b'n', b'c',
        ];
        buf.extend_from_slice(&bytes_msg);

        let msg: ServerMessage = buf.into();
        if let ServerMessage::Serv {
            ip_address,
            ref sync_group_id,
        } = msg
        {
            assert_eq!(ip_address, Ipv4Addr::new(172, 16, 1, 2));
            assert_eq!(sync_group_id, &Some("sync".to_owned()));
        } else {
            panic!("SERV message not received");
        }
    }

    #[test]
    fn recv_status() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b't', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);

        let msg: ServerMessage = buf.into();
        match msg {
            ServerMessage::Status(d) => assert_eq!(d, Duration::from_millis(252711186)),
            _ => {
                panic!("STRMt message not received");
            }
        }
    }

    #[test]
    fn recv_stop() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b'q', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if !matches!(msg, ServerMessage::Stop) {
            panic!("STRMq message not received");
        }
    }

    #[test]
    fn recv_pause() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b'p', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if let ServerMessage::Pause(p) = msg {
            assert_eq!(p, Duration::from_millis(235868177));
        } else {
            panic!("STRMp message not received");
        }
    }

    #[test]
    fn recv_unpause() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b'u', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if let ServerMessage::Unpause(p) = msg {
            assert_eq!(p, Duration::from_millis(235868177));
        } else {
            panic!("STRMu message not received");
        }
    }

    #[test]
    fn recv_skip() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b'a', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if let ServerMessage::Skip(p) = msg {
            assert_eq!(p, Duration::from_millis(235868177));
        } else {
            panic!("STRMa message not received");
        }
    }

    #[test]
    fn recv_unrecognised() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b'x', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
            17, 18, 19, 20, 21, 22, 23, 24,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if let ServerMessage::Unrecognised(ref s) = msg {
            assert_eq!(s, "strm_x");
        } else {
            panic!("STRMx message not received");
        }
    }

    #[test]
    fn recv_enable() {
        let mut buf = BytesMut::new();
        let bytes_msg = [b'a', b'u', b'd', b'e', 0, 1];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if let ServerMessage::Enable(a, b) = msg {
            assert_eq!((a, b), (false, true));
        } else {
            panic!("AUDE message not received");
        }
    }

    #[test]
    fn recv_gain() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b'a', b'u', b'd', b'g', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 128, 0,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        match msg {
            ServerMessage::Gain(left, right) => {
                assert_eq!(left, 1.0);
                assert_eq!(right, 0.5);
            }
            _ => panic!("GAIN message incorrect"),
        }
    }

    #[test]
    fn recv_setname() {
        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b'e', b't', b'd', 0, b'n', b'e', b'w', b'n', b'a', b'm', b'e', 0,
        ];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        match msg {
            ServerMessage::Setname(ref name) => {
                assert_eq!(name, "newname");
            }
            _ => panic!("SETNAME message incorrect"),
        }
    }

    #[test]
    fn recv_queryname() {
        let mut buf = BytesMut::new();
        let bytes_msg = [b's', b'e', b't', b'd', 0];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if !matches!(msg, ServerMessage::Queryname) {
            panic!("SETD message not received");
        }
    }

    #[test]
    fn recv_disabledac() {
        let mut buf = BytesMut::new();
        let bytes_msg = [b's', b'e', b't', b'd', 4];
        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        if !matches!(msg, ServerMessage::DisableDac) {
            panic!("SETD message not received");
        }
    }

    #[test]
    fn recv_strm() {
        fn do_panic() {
            panic!("STRMs message not received");
        }

        let mut buf = BytesMut::new();
        let bytes_msg = [
            b's', b't', b'r', b'm', b's', b'1', b'm', b'2', b'3', b'?', b'0', 1, 2, 3, b'4', 1, 2,
            0, 0, 1, 128, 0, 35, 41, 172, 16, 1, 2,
        ];

        buf.extend_from_slice(&bytes_msg);
        let msg: ServerMessage = buf.into();
        match msg {
            ServerMessage::Stream {
                ref autostart,
                ref format,
                ref pcmsamplesize,
                ref pcmsamplerate,
                ref pcmchannels,
                ref pcmendian,
                threshold,
                ref spdif_enable,
                trans_period,
                ref trans_type,
                ref flags,
                output_threshold,
                replay_gain,
                server_port,
                server_ip,
                ref http_headers,
            } => {
                if let AutoStart::Auto = autostart {
                } else {
                    do_panic();
                }
                if let Format::Mp3 = format {
                } else {
                    do_panic();
                }
                if let PcmSampleSize::Twenty = pcmsamplesize {
                } else {
                    do_panic();
                }
                if let PcmSampleRate::Rate(r) = pcmsamplerate {
                    assert_eq!(*r, 44100);
                } else {
                    do_panic();
                }
                if let PcmChannels::SelfDescribing = pcmchannels {
                } else {
                    do_panic();
                }
                if let PcmEndian::Big = pcmendian {
                } else {
                    do_panic();
                }
                assert_eq!(threshold, 1024);
                if let SpdifEnable::Off = spdif_enable {
                } else {
                    do_panic();
                }
                assert_eq!(trans_period, Duration::from_secs(3));
                if let TransType::FadeInOut = trans_type {
                } else {
                    do_panic();
                }
                assert_eq!(flags, &StreamFlags::INVERT_POLARITY_LEFT);
                assert_eq!(output_threshold, Duration::from_millis(20));
                assert_eq!(replay_gain, 1.5);
                assert_eq!(server_port, 9001);
                assert_eq!(server_ip, Ipv4Addr::new(172, 16, 1, 2));
                assert!(http_headers.is_none());
            }
            _ => do_panic(),
        }
    }
}
