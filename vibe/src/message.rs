/// LMS server-message and internal player-message handling.
///
use std::{
    net::Ipv4Addr,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use crossbeam::channel::Sender;
use lms_proto::{ClientMessage, ServerMessage, StatusCode};
use log::{error, info, warn};

#[cfg(feature = "notify")]
use crate::notify::notify;
use crate::{
    audio_out::{self, AudioOutput},
    decode,
    state::{SKIP, STATUS, VOLUME},
};

// ---------------------------------------------------------------------------
// PlayerMsg
// ---------------------------------------------------------------------------

/// Messages sent between the decoder / stream thread and the main event loop.
#[allow(unused)]
pub enum PlayerMsg {
    EndOfDecode,
    Drained,
    Pause,
    Unpause,
    Connected,
    BufferThreshold,
    NotSupported,
    StreamEstablished,
    TrackStarted,
    Decoder((decode::VibeDecoder, decode::StreamParams)),
}

// ---------------------------------------------------------------------------
// PlayerContext
// ---------------------------------------------------------------------------

/// All the state needed to react to server and player messages.
pub struct PlayerContext {
    /// Current audio output sink, if one has been enabled by the LMS.
    pub output: Option<Box<dyn AudioOutput>>,
    /// The IP of the LMS that first connected; used as a fallback for stream connections.
    pub server_default_ip: Ipv4Addr,
    /// Human-readable player name (may be changed at runtime by the server).
    pub name: Arc<RwLock<String>>,
    /// Channel for sending SlimProto client messages back to the server.
    pub slim_tx: Sender<ClientMessage>,
    /// Channel for posting `PlayerMsg` values back into the event loop.
    pub stream_tx: Sender<PlayerMsg>,
    /// The instant the current connection was established; used to honour timed unpauses.
    pub start_time: Instant,
    /// Which audio backend to use ("pulse", "pipewire", or "rodio").
    pub output_system: String,
    /// Optional preferred output device name (currently only used by the rodio backend).
    pub device: Option<String>,
    #[cfg(feature = "notify")]
    pub quiet: bool,
}

impl PlayerContext {
    // -----------------------------------------------------------------------
    // Server-message handler
    // -----------------------------------------------------------------------

    /// Dispatch a message received from the LMS.
    pub fn handle_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Serv { ip_address, .. } => self.on_serv(ip_address),
            ServerMessage::Queryname => self.on_query_name(),
            ServerMessage::Setname(new_name) => self.on_set_name(new_name),
            ServerMessage::Gain(left, right) => self.on_gain(left, right),
            ServerMessage::Status(ts) => self.on_status(ts), // ts: Duration
            ServerMessage::Stop => self.on_stop(),
            ServerMessage::Flush => self.on_flush(),
            ServerMessage::Pause(interval) => self.on_pause(interval),
            ServerMessage::Unpause(interval) => self.on_unpause(interval),
            ServerMessage::Skip(interval) => self.on_skip(interval),
            ServerMessage::Stream {
                http_headers,
                server_ip,
                server_port,
                threshold,
                format,
                pcmsamplesize,
                pcmsamplerate,
                pcmchannels,
                autostart,
                output_threshold,
                ..
            } => self.on_stream(
                http_headers,
                server_ip,
                server_port,
                threshold,
                format,
                pcmsamplesize,
                pcmsamplerate,
                pcmchannels,
                autostart,
                output_threshold,
            ),
            ServerMessage::Enable(spdif, dac) => self.on_enable(spdif, dac),
            ServerMessage::DisableDac => self.on_disable_dac(),
            cmd => {
                warn!("Unimplemented command: {:?}", cmd);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Player-message handler
    // -----------------------------------------------------------------------

    /// Dispatch an internal player message (decoder → event loop).
    pub fn handle_player_message(&mut self, msg: PlayerMsg) {
        match msg {
            PlayerMsg::EndOfDecode => self.on_end_of_decode(),
            PlayerMsg::Drained => self.on_drained(),
            PlayerMsg::Pause => self.on_player_pause(),
            PlayerMsg::Unpause => self.on_player_unpause(),
            PlayerMsg::Connected => self.on_connected(),
            PlayerMsg::BufferThreshold => self.on_buffer_threshold(),
            PlayerMsg::NotSupported => self.on_not_supported(),
            PlayerMsg::StreamEstablished => self.on_stream_established(),
            PlayerMsg::TrackStarted => self.on_track_started(),
            PlayerMsg::Decoder(decoder_params) => self.on_decoder(decoder_params),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn send_status(&self, code: StatusCode) {
        if let Ok(mut status) = STATUS.lock() {
            let msg = status.make_status_message(code);
            _ = self.slim_tx.send(msg);
        }
    }

    fn update_elapsed_and_send(&self, play_time: Duration, code: StatusCode) {
        if let Ok(mut status) = STATUS.lock() {
            status.set_elapsed_milli_seconds(play_time.as_millis() as u32);
            status.set_elapsed_seconds(play_time.as_secs() as u32);
            let msg = status.make_status_message(code);
            _ = self.slim_tx.send(msg);
        }
    }

    fn current_play_time(&self) -> Duration {
        self.output
            .as_ref()
            .map(|o| o.get_dur())
            .unwrap_or(Duration::ZERO)
    }

    // -----------------------------------------------------------------------
    // Individual server-message handlers
    // -----------------------------------------------------------------------

    fn on_serv(&mut self, ip_address: Ipv4Addr) {
        info!("Switching to server at {ip_address}");
        self.server_default_ip = ip_address;
    }

    fn on_query_name(&self) {
        info!("Name query from server");
        if let Ok(name) = self.name.read() {
            info!("Sending name: {name}");
            _ = self.slim_tx.send(ClientMessage::Name(name.to_owned()));
        }
    }

    fn on_set_name(&self, new_name: String) {
        if let Ok(mut name) = self.name.write() {
            info!("Set name to {new_name}");
            *name = new_name;
        }
    }

    fn on_gain(&self, left: f64, right: f64) {
        info!("Setting volume to ({left}, {right})");
        if let Ok(mut vol) = VOLUME.lock() {
            let left = (left.min(1.0) as f32).sqrt();
            let right = (right.min(1.0) as f32).sqrt();
            vol[0] = left;
            vol[1] = right;
        }
    }

    fn on_status(&self, ts: Duration) {
        let play_time = self.current_play_time();
        if let Ok(mut status) = STATUS.lock() {
            status.set_elapsed_milli_seconds(play_time.as_millis() as u32);
            status.set_elapsed_seconds(play_time.as_secs() as u32);
            status.set_timestamp(ts);
            let msg = status.make_status_message(StatusCode::Timer);
            _ = self.slim_tx.send(msg);
        }
    }

    fn on_stop(&mut self) {
        info!("Stop playback received");
        if let Some(output) = &mut self.output {
            output.stop();
        }
        if let Ok(mut status) = STATUS.lock() {
            status.set_elapsed_milli_seconds(0);
            status.set_elapsed_seconds(0);
            status.set_output_buffer_size(0);
            status.set_output_buffer_fullness(0);
            info!("Player flushed");
            let msg = status.make_status_message(StatusCode::Flushed);
            _ = self.slim_tx.send(msg);
        }
    }

    fn on_flush(&mut self) {
        info!("Flushing");
        if let Some(output) = &mut self.output {
            output.flush();
        }
        if let Ok(mut status) = STATUS.lock() {
            status.set_elapsed_milli_seconds(0);
            status.set_elapsed_seconds(0);
            status.set_output_buffer_size(0);
            status.set_output_buffer_fullness(0);
            info!("Player flushed");
            let msg = status.make_status_message(StatusCode::Flushed);
            _ = self.slim_tx.send(msg);
        }
    }

    fn on_pause(&mut self, interval: Duration) {
        let play_time = self.current_play_time();
        info!("Pause requested with interval {:?}", interval);

        if let Some(output) = &mut self.output {
            if interval.is_zero() {
                if output.pause() {
                    self.update_elapsed_and_send(play_time, StatusCode::Pause);
                }
            } else if output.pause() {
                let stream_tx = self.stream_tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(interval);
                    _ = stream_tx.send(PlayerMsg::Unpause);
                });
            }
        }
    }

    fn on_unpause(&mut self, interval: Duration) {
        let play_time = self.current_play_time();
        info!("Resume requested with interval {:?}", interval);

        if interval.is_zero() {
            if let Some(output) = &mut self.output
                && output.unpause()
            {
                info!("Sending resumed to server");
                self.update_elapsed_and_send(play_time, StatusCode::Resume);
            }
        } else {
            let dur = interval.saturating_sub(Instant::now() - self.start_time);
            info!("Resuming in {:?}", dur);
            let stream_tx = self.stream_tx.clone();
            let slim_tx = self.slim_tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(dur);
                _ = stream_tx.send(PlayerMsg::Unpause);
                if let Ok(mut status) = STATUS.lock() {
                    info!("Sending resumed to server");
                    status.set_elapsed_milli_seconds(play_time.as_millis() as u32);
                    status.set_elapsed_seconds(play_time.as_secs() as u32);
                    let msg = status.make_status_message(StatusCode::Resume);
                    _ = slim_tx.send(msg);
                }
            });
        }
    }

    fn on_skip(&self, interval: Duration) {
        info!("Skip ahead: {:?}", interval);
        SKIP.store(interval);
    }

    #[allow(clippy::too_many_arguments)]
    fn on_stream(
        &self,
        http_headers: Option<String>,
        server_ip: Ipv4Addr,
        server_port: u16,
        threshold: u32,
        format: lms_proto::Format,
        pcmsamplesize: lms_proto::PcmSampleSize,
        pcmsamplerate: lms_proto::PcmSampleRate,
        pcmchannels: lms_proto::PcmChannels,
        autostart: lms_proto::AutoStart,
        output_threshold: Duration,
    ) {
        info!("Start stream command from server");
        info!("\tFormat: {:?}", format);
        info!("\tThreshold: {} bytes", threshold);
        info!("\tOutput threshold: {:?}", output_threshold);

        if let Some(http_headers) = http_headers {
            let num_crlf = http_headers.matches("\r\n").count();
            if num_crlf == 0 {
                return;
            }

            if let Ok(mut status) = STATUS.lock() {
                status.add_crlf(num_crlf as u8);
            }

            let stream_tx = self.stream_tx.clone();
            let default_ip = self.server_default_ip;
            std::thread::spawn(move || {
                match decode::make_decoder(
                    server_ip,
                    default_ip,
                    server_port,
                    http_headers,
                    stream_tx.clone(),
                    threshold,
                    format,
                    pcmsamplesize,
                    pcmsamplerate,
                    pcmchannels,
                    autostart,
                    output_threshold,
                ) {
                    Ok(decoder_params) => {
                        _ = stream_tx.send(PlayerMsg::Decoder(decoder_params));
                    }
                    Err(e) => {
                        warn!("{}", e);
                        _ = stream_tx.send(PlayerMsg::NotSupported);
                    }
                }
            });
        }
    }

    fn on_enable(&mut self, spdif: bool, dac: bool) {
        if spdif && dac {
            info!("Connecting output");
            self.output = audio_out::make_audio_output(
                &self.output_system,
                #[cfg(feature = "rodio")]
                &self.device,
            )
            .ok();
        } else {
            info!("Disconnecting output");
            self.output = None;
        }
    }

    fn on_disable_dac(&mut self) {
        info!("Disconnecting output");
        self.output = None;
    }

    // -----------------------------------------------------------------------
    // Individual player-message handlers
    // -----------------------------------------------------------------------

    fn on_end_of_decode(&self) {
        info!("Decoder ready for new stream");
        self.send_status(StatusCode::DecoderReady);
    }

    fn on_drained(&mut self) {
        info!("End of track");
        if let Some(output) = &mut self.output {
            output.shift();
            output.unpause();
            if let Ok(mut status) = STATUS.lock() {
                status.set_elapsed_milli_seconds(0);
                status.set_elapsed_seconds(0);
            }
        }
    }

    fn on_player_pause(&mut self) {
        info!("Pausing track");
        if let Some(output) = &mut self.output {
            output.pause();
        }
    }

    fn on_player_unpause(&mut self) {
        if let Some(output) = &mut self.output
            && output.unpause()
        {
            info!("Sending track unpaused by player");
            self.send_status(StatusCode::TrackStarted);
        }
    }

    fn on_connected(&self) {
        info!("Sending stream connected");
        self.send_status(StatusCode::Connect);
    }

    fn on_buffer_threshold(&self) {
        info!("Sending buffer threshold reached");
        self.send_status(StatusCode::BufferThreshold);
    }

    fn on_not_supported(&self) {
        warn!("Unsupported format");
        self.send_status(StatusCode::NotSupported);
    }

    fn on_stream_established(&self) {
        info!("Sending stream established");
        self.send_status(StatusCode::StreamEstablished);
    }

    fn on_track_started(&self) {
        info!("Sending track started");
        if let Ok(mut status) = STATUS.lock() {
            status.set_elapsed_milli_seconds(0);
            status.set_elapsed_seconds(0);
            let msg = status.make_status_message(StatusCode::TrackStarted);
            _ = self.slim_tx.send(msg);
        }
    }

    #[allow(unused_mut)]
    fn on_decoder(
        &mut self,
        (mut decoder, stream_params): (decode::VibeDecoder, decode::StreamParams),
    ) {
        #[cfg(feature = "notify")]
        if let Some(metadata) = decoder.metadata() {
            if !self.quiet {
                notify(metadata);
            }
        }

        if let Some(output) = &mut self.output
            && let Err(e) = output.enqueue_new_stream(
                decoder,
                self.stream_tx.clone(),
                stream_params,
                &self.device,
            )
        {
            error!("{}", e);
        }
    }
}
