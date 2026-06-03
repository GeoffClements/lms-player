/// This module provides the `discover` function which "pings" for a server
/// on the network returning its address if it exists.

use crate::{proto::{Server, ServerTlv, ServerTlvMap, SLIM_PORT}, Capabilities};

use std::{
    collections::HashMap,
    io,
    net::{Ipv4Addr, SocketAddr, UdpSocket, SocketAddrV4},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{sleep, spawn},
    time::Duration,
};

/// Repeatedly send discover "pings" to the server with an optional timeout.
///
/// Returns:
/// - `Ok(None)` on timeout
/// - `Ok(Some(Server))` on server response.
/// - `io::Error` if an error occurs
///
/// Note that the Slim Protocol is IPv4 only.
/// This function will try forever if no timeout is passed in which case `Ok(None)` can never
/// be returned.
pub fn discover(timeout: Option<Duration>) -> io::Result<Option<Server>> {
    const UDPMAXSIZE: usize = 1450; // as defined in LMS code

    let cx = UdpSocket::bind((Ipv4Addr::new(0, 0, 0, 0), 0))?;
    cx.set_broadcast(true)?;
    cx.set_read_timeout(timeout)?;

    let cx_send = cx.try_clone()?;
    let running = Arc::new(AtomicBool::new(true));
    let is_running = running.clone();
    spawn(move || {
        let buf = b"eNAME\0IPAD\0JSON\0VERS"; // Also \0UUID\0JVID
        while is_running.load(Ordering::Relaxed) {
            cx_send
                .send_to(buf, (Ipv4Addr::new(255, 255, 255, 255), SLIM_PORT))
                .ok();
            sleep(Duration::from_secs(5));
        }
    });

    let mut buf = [0u8; UDPMAXSIZE];
    let response = cx.recv_from(&mut buf);
    running.store(false, Ordering::Relaxed);

    response.map_or_else(
        |e| match e.kind() {
            io::ErrorKind::WouldBlock => Ok(None),
            _ => Err(e),
        },
        |(len, sock_addr)| match sock_addr {
            SocketAddr::V4(addr) => Ok(Some(Server {
                socket: SocketAddrV4::new(*addr.ip(), SLIM_PORT),
                // ip_address: *addr.ip(),
                // port: SLIM_PORT,
                tlv_map: {
                    if len > 0 && buf[0] == b'E' {
                        Some(decode_tlv(&buf[1..]))
                    } else {
                        None
                    }
                },
                sync_group_id: None,
                caps: Capabilities(Vec::new()), // No capabilities in discovery
            })),
            _ => Ok(None),
        },
    )
}

fn decode_tlv(buf: &[u8]) -> ServerTlvMap {
    let mut ret = HashMap::new();
    let mut view = &buf[..];

    while view.len() > 4 && view[0].is_ascii() {
        let token = String::from_utf8(view[..4].to_vec()).unwrap_or_default();
        let valen = view[4] as usize;
        view = &view[5..];

        if view.len() < valen {
            break;
        }

        let value = String::from_utf8(view[..valen].to_vec()).unwrap_or_default();

        let value = match token.as_str() {
            "NAME" => ServerTlv::Name(value),
            "VERS" => ServerTlv::Version(value),
            "IPAD" => {
                if let Ok(addr) = value.parse::<Ipv4Addr>() {
                    ServerTlv::Address(addr)
                } else {
                    break;
                }
            }
            "JSON" => {
                if let Ok(port) = value.parse::<u16>() {
                    ServerTlv::Port(port)
                } else {
                    break;
                }
            }
            _ => {
                break;
            }
        };

        ret.insert(token, value);
        view = &view[valen..];
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_discover() {
        let res = discover(Some(Duration::from_secs(1)));
        assert!(res.is_ok());

        if let Ok(Some(server)) = res {
            assert!(!server.socket.ip().is_unspecified());
            assert!(server.tlv_map.is_some());
        }
    }
}
