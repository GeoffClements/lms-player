use crossbeam::channel::{Receiver, Sender};
use lms_proto::{Capability, ClientMessage, Hello, ServerMessage, discover};
use log::{info, warn};
use mac_address::{MacAddress, get_mac_address};

use crate::state::STATUS;

use std::{
    net::SocketAddrV4,
    thread::{sleep, spawn},
    time::Duration,
};

const SQUEEZEPLAY_ID: u8 = 12;

pub fn run(
    server_addr: Option<SocketAddrV4>,
    slim_rx_in: Sender<Option<ServerMessage>>,
    slim_tx_out: Receiver<ClientMessage>,
) {
    let mac = match get_mac_address() {
        Ok(Some(mac)) => mac,
        _ => MacAddress::default(),
    };

    // These are used to update the server address when a Serv message is received
    let mut new_server_sock = None;
    let mut sync_group_id: Option<String> = None;

    spawn(move || {
        // The reconnect loop allows us to reconnect to a different server when a Serv message
        // is received.
        'reconnect: loop {
            let mut caps = vec![
                Capability::Model(String::from("squeezelite")),
                Capability::Modelname(String::from("SqueezeLite")),
                Capability::Accurateplaypoints,
                Capability::Hasdigitalout,
                Capability::Haspreamp,
                Capability::Hasdisabledac,
                Capability::Firmware(env!("CARGO_PKG_VERSION").to_owned()),
                Capability::Maxsamplerate(192000),
                Capability::Pcm,
                Capability::Mp3,
                Capability::Aac,
                Capability::Alc,
                Capability::Ogg,
                Capability::Flc,
            ];

            if let Some(sgid) = sync_group_id.take() {
                caps.push(Capability::Syncgroupid(sgid));
            }

            let bytes_received = STATUS
                .lock()
                .map(|status| status.bytes_received())
                .unwrap_or_default();

            let hello = Hello::new()
                .device_id(SQUEEZEPLAY_ID)
                .mac(mac)
                .bytes_received(bytes_received)
                .capabilities(caps);

            // Work out which address to use for the server
            let lms_sock = match new_server_sock.take() {
                Some(sock) => sock,
                None => match server_addr {
                    Some(sock) => sock,
                    None => {
                        info!("No server address provided, attempting discovery...");
                        match discover(Some(Duration::from_secs(30))) {
                            Ok(Some((sock, _))) => sock,
                            _ => {
                                warn!("No server found on the network");
                                continue;
                            }
                        }
                    }
                },
            };

            // Now attempt to connect to the server
            info!("Attempting to connect to server at {}", lms_sock.ip());
            let (mut rx, mut tx) = match hello.connect(lms_sock) {
                Ok(rxtx) => rxtx,
                Err(e) => {
                    warn!("Failed to connect to server at {}: {}", lms_sock.ip(), e);
                    sleep(Duration::from_secs(5));
                    new_server_sock = Some(lms_sock);
                    continue;
                }
            };
            info!("Connected to server at {}", lms_sock.ip());

            // Start write thread. The thread will exit when the connection is lost or a Bye message with n=1 is sent.
            let slim_tx_out_ref = slim_tx_out.clone();
            spawn(move || {
                while let Ok(msg) = slim_tx_out_ref.recv() {
                    // Bye(1) is used to notify this thread to terminate. The LMS takes this
                    // as the client is going down for an upgrade. Send a normal BYE!(0) instead.
                    let (message, terminate) = if matches!(msg, ClientMessage::Bye(1)) {
                        (ClientMessage::Bye(0), true)
                    } else {
                        (msg, false)
                    };

                    if tx.send(message).is_err() || terminate {
                        break;
                    }
                }
                info!("Write thread exiting");
            });

            // The serv loop reads messages from the server until a Serv message is received or the connection is lost.
            // When a Serv message is received it breaks to the reconnect loop to connect to the new server.
            // When the connection is lost it ends the thread by breaking the reconnect loop.
            'serv: loop {
                match rx.recv() {
                    Ok(messages) => {
                        for msg in messages.into_iter() {
                            // println!("{:?}", msg);
                            match msg {
                                // Intercept the request to change to another server
                                ServerMessage::Serv {
                                    ip_address: ip,
                                    sync_group_id: sgid,
                                } => {
                                    // Drop any obviously incorrect addresses
                                    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast()
                                    {
                                        continue;
                                    }
                                    info!("Received SERV message, new server at {}", ip);
                                    _ = slim_rx_in.send(None);
                                    new_server_sock =
                                        Some(SocketAddrV4::new(ip, lms_proto::SLIM_PORT));
                                    sync_group_id = sgid;
                                    break 'serv;
                                }

                                // Business as usual for any other message
                                _ => {
                                    _ = slim_rx_in.send(Some(msg));
                                }
                            }
                        }
                    }

                    Err(_) => {
                        _ = slim_rx_in.send(None);
                        break 'reconnect;
                    }
                }
            }
        }
    });
}
