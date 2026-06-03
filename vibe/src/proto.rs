use crossbeam::channel::{Receiver, Sender};
use log::info;
use slimproto::{
    self, discovery::discover, proto::Server, Capabilities, Capability, ClientMessage,
    FramedReader, FramedWriter, ServerMessage,
};
use std::{net::SocketAddrV4, thread::sleep, time::Duration};

pub fn run(
    server_addr: Option<SocketAddrV4>,
    slim_rx_in: Sender<Option<ServerMessage>>,
    slim_tx_out: Receiver<ClientMessage>,
) {
    std::thread::spawn(move || {
        let mut server = match server_addr {
            Some(sock) => Server::from(sock),
            None => match discover(None) {
                Ok(Some(server)) => server,
                _ => unreachable!(),
            },
        };

        _ = slim_rx_in.send(Some(ServerMessage::Serv {
            ip_address: (*server.socket.ip()),
            sync_group_id: None,
        }));

        let syncgroupid = String::new();
        // Outer loop to reconnect to a different server and
        // update server details when a Serv message is received
        'outer: loop {
            let mut caps = Capabilities::default();

            caps.add(Capability::Firmware(env!("CARGO_PKG_VERSION").to_owned()));
            caps.add(Capability::Maxsamplerate(192000));

            if !syncgroupid.is_empty() {
                info!("Joining sync group: {syncgroupid}");
                caps.add(Capability::Syncgroupid(syncgroupid.to_owned()));
            }

            caps.add(Capability::Pcm);
            caps.add(Capability::Mp3);
            caps.add(Capability::Aac);
            caps.add(Capability::Alc);
            caps.add(Capability::Ogg);
            caps.add(Capability::Flc);

            // Connect to the server
            info!("Attempting to connect to server: {}", server.socket);
            let (mut rx, mut tx) = loop {
                match server.connect() {
                    Ok((rx, tx)) => break (rx, tx),
                    Err(_) => {
                        sleep(Duration::from_secs(5));
                    }
                }
            };

            // Start write thread
            // Continues until connection is dropped
            let slim_tx_out_r = slim_tx_out.clone();
            std::thread::spawn(move || {
                while let Ok(msg) = slim_tx_out_r.recv() {
                    // println!("{:?}", msg);
                    if let ClientMessage::Bye(n) = msg {
                        if n == 1 {
                            break;
                        }
                    }

                    if tx.framed_write(msg).is_err() {
                        break;
                    }
                }
            });

            // Inner read loop
            'inner: loop {
                match rx.framed_read() {
                    Ok(messages) => {
                        for msg in messages.into_iter() {
                            // println!("{:?}", msg);
                            match msg {
                                // Request to change to another server
                                ServerMessage::Serv {
                                    ip_address: ip,
                                    sync_group_id: sgid,
                                } => {
                                    // Drop any obviously incorrect addresses
                                    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast()
                                    {
                                        continue;
                                    }
                                    
                                    // Now inform the main thread,
                                    server = (ip, sgid).into();
                                    _ = slim_rx_in.send(Some(ServerMessage::Serv {
                                        ip_address: ip,
                                        sync_group_id: None,
                                    }));
                                    break 'inner;
                                }

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
        info!("Lost contact with server at {}", server.socket);
    });
}
