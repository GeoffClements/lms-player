/// Entry point for Vibe.
///
/// `main.rs` is purely a wiring layer:
///   - parse CLI arguments  (`cli`)
///   - resolve the server address
///   - run the startup helper if requested  (`startup`)
///   - list audio devices if requested  (`audio_out`)
///   - drive the main reconnect / event-select loop
///
use std::{
    iter::repeat_n,
    net::SocketAddrV4,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use anyhow::Context;
use cfg_if::cfg_if;
use clap::Parser;
use crossbeam::channel::{Select, bounded};
use lms_proto::{ClientMessage, StatusCode};
use log::warn;

mod audio_out;
mod cli;
mod decode;
mod message;
#[cfg(feature = "notify")]
mod notify;
#[cfg(feature = "pipewire")]
mod pipewire_out;
mod proto;
#[cfg(feature = "pulse")]
mod pulse_out;
#[cfg(feature = "rodio")]
mod rodio_out;
mod startup;
mod state;

use cli::Cli;
use message::{PlayerContext, PlayerMsg};
use state::STATUS;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    simple_logger::SimpleLogger::new()
        .with_colors(true)
        .with_level(cli.loglevel)
        .init()?;

    // Determine which audio backend to use at runtime.
    let output_system: String = {
        cfg_if! {
            if #[cfg(any(
                all(feature = "pulse", feature = "rodio"),
                all(feature = "pulse", feature = "pipewire"),
                all(feature = "rodio", feature = "pipewire")
            ))] {
                cli.system.clone()
            } else {
                cli::default_system()
            }
        }
    };

    // --create-service: generate a systemd unit file and exit.
    if cli.create_service {
        if let Some(ref server) = cli.server {
            cli::parse_server_addr(server).context(format!("Server not found: {}", server))?;
        }
        startup::create_systemd_unit(&cli.server, &output_system, &cli.device)?;
        return Ok(());
    }

    // --list: print output device names and exit.
    if cli.list {
        if let Ok(output) = audio_out::make_audio_output(
            &output_system,
            #[cfg(feature = "rodio")]
            &cli.device,
        ) {
            println!("Output devices:");
            let names = output.get_output_device_names()?;
            names
                .iter()
                .enumerate()
                .for_each(|(i, (name, description))| {
                    println!("{}: {}", i, name);
                    if let Some(desc) = description {
                        let indent = repeat_n(
                            " ",
                            if i < 10 {
                                3
                            } else if i < 100 {
                                4
                            } else {
                                5
                            },
                        )
                        .collect::<String>();
                        println!("{}{}", indent, desc);
                    }
                });
            print!("Found {} device", names.len());
            if names.len() != 1 {
                print!("s");
            }
            println!();
        }
        return Ok(());
    }

    // Resolve the optional --server argument once up front.
    let cli_server: Option<SocketAddrV4> = if let Some(ref server) = cli.server {
        Some(cli::parse_server_addr(server).context(format!("Server not found: {}", server))?)
    } else {
        None
    };

    // -----------------------------------------------------------------------
    // Main reconnect loop — restarts whenever the server connection is lost.
    // -----------------------------------------------------------------------
    loop {
        let player_name = {
            let name = match hostname::get().map(|s| s.into_string()) {
                Ok(Ok(hostname)) => format!("{}@{}", cli.name, hostname),
                _ => cli.name.clone(),
            };
            Arc::new(RwLock::new(name))
        };

        let start_time = Instant::now();

        // Channels for the SlimProto protocol thread.
        let (slim_tx, slim_tx_out) = bounded::<ClientMessage>(1);
        let (slim_rx_in, slim_rx) = bounded(1);
        proto::run(cli_server, slim_rx_in, slim_tx_out);

        // Channel for decoder / audio-backend → event loop messages.
        let (stream_tx, stream_rx) = bounded::<PlayerMsg>(10);

        let mut ctx = PlayerContext {
            output: None,
            server_default_ip: *cli_server.unwrap_or(SocketAddrV4::new(0.into(), 0)).ip(),
            name: player_name,
            slim_tx: slim_tx.clone(),
            stream_tx: stream_tx.clone(),
            start_time,
            output_system: output_system.clone(),
            device: cli.device.clone(),
            #[cfg(feature = "notify")]
            quiet: cli.quiet,
        };

        // Multiplex the two incoming channels.
        let mut select = Select::new();
        let slim_idx = select.recv(&slim_rx);
        let stream_idx = select.recv(&stream_rx);

        // Inner event loop — exits on server disconnect.
        loop {
            // Poll more frequently while audio is playing so status ticks are timely.
            let timeout = if ctx.output.is_some() {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(5)
            };

            match select.select_timeout(timeout) {
                // Message from the LMS server.
                Ok(op) if op.index() == slim_idx => match op.recv(&slim_rx)? {
                    Some(msg) => ctx.handle_server_message(msg),
                    None => {
                        warn!("Lost contact with server, resetting");
                        _ = slim_tx.send(ClientMessage::Bye(1));
                        if let Some(ref mut output) = ctx.output {
                            output.stop();
                        }
                        break;
                    }
                },

                // Message from the decoder / audio backend.
                Ok(op) if op.index() == stream_idx => {
                    let msg = op.recv(&stream_rx)?;
                    ctx.handle_player_message(msg);
                }

                // Should not heppen, ignore if it does
                Ok(_) => {}

                // Timeout: send a periodic status update to the server.
                Err(_) => {
                    let play_time = ctx
                        .output
                        .as_ref()
                        .map(|o| o.get_dur())
                        .unwrap_or(Duration::ZERO);

                    if let Ok(mut status) = STATUS.lock() {
                        status.set_elapsed_milli_seconds(play_time.as_millis() as u32);
                        status.set_elapsed_seconds(play_time.as_secs() as u32);
                        let msg = status.make_status_message(StatusCode::Timer);
                        _ = slim_tx.send(msg);
                    }
                }
            }
        }
    }
}
