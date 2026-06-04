use crossbeam::channel::{Receiver, Sender};
use lms_proto::{discover, Capability, ClientMessage, ServerMessage};
use log::{error, info};
use mac_address::{get_mac_address, MacAddress};

use crate::state::STATUS;

use std::{
    net::SocketAddrV4,
    thread::{sleep, spawn},
    time::Duration,
};

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
        // The outer loop allows us to reconnect to a different server when a Serv message is received, or if the connection is lost.
        'outer: loop {
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
                .map(|status| status.bytes_received)
                .unwrap_or(0);

            let hello = lms_proto::Hello::new()
                .device_id(12)
                .mac(mac)
                .bytes_received(bytes_received)
                .capabilities(caps); //todo more params to set

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
                                error!("No server found on the network");
                                continue;
                            }
                        }
                    }
                },
            };

            
            // Now attempt to connect to the server
            info!("Attempting to connect to server at {}", lms_sock.ip());
            let (mut rx, mut tx) = match hello.connect(lms_sock) {
                Ok((rx, tx)) => (rx, tx),
                Err(e) => {
                    error!("Failed to connect to server at {}: {}", lms_sock.ip(), e);
                    sleep(Duration::from_secs(5));
                    new_server_sock = Some(lms_sock);
                    continue;
                }
            };

            // Start write thread. The thread will exit when the connection is lost or a Bye message with n=1 is sent.
            let slim_tx_out_ref = slim_tx_out.clone();
            spawn(move || {
                while let Ok(msg) = slim_tx_out_ref.recv() {
                    // println!("{:?}", msg);
                    let end = if let ClientMessage::Bye(1) = msg {
                        true
                    } else {
                        false
                    };

                    if tx.send(msg).is_err() {
                        break;
                    }

                    if end {
                        break;
                    }
                }
                info!("Write thread exiting");
            });

            // The inner loop reads messages from the server until the connection is lost or a Serv message is received,
            // in which case it breaks to the outer loop to reconnect.
            'inner: loop {
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
                                    break 'inner;
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
                        break 'outer;
                    }
                }
            }
        }
    });
}
