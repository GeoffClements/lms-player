// src/pipewire_out.rs
// Pipewire audio output implementation
//
// Required system dependencies:
// - Pipewire development libraries (libpipewire-0.3-dev on Debian-based systems)
// - SPA development libraries (libspa-0.2-dev on Debian-based systems)
// - clang development libraries (libclang-dev on Debian-based systems)

use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

use crossbeam::{
    atomic::AtomicCell,
    channel::{Sender, bounded},
};
use log::warn;
use pipewire::{
    context::ContextRc,
    core::CoreRc,
    properties::properties,
    registry::{Listener, RegistryRc},
    spa::{
        param::audio::{AudioFormat, AudioInfoRaw},
        pod::{Pod, serialize::PodSerializer},
        utils::Direction,
    },
    stream::{Stream, StreamFlags, StreamListener, StreamRc, StreamState},
    thread_loop::ThreadLoopRc,
    types::ObjectType,
};

use crate::{
    audio_out::AudioOutput,
    decode::StreamParams,
    decode::{DecoderError, VibeDecoder},
    message::PlayerMsg,
    state::SKIP,
};

const MIN_AUDIO_BUFFER_SIZE: usize = 8 * 1024;

#[derive(PartialEq)]
enum ProcessState {
    Running,
    Draining,
}

// Drop order is important here to ensure the mainloop is dropped last,
// as it may be needed for proper cleanup of other resources.
pub struct PipewireAudioOutput {
    nodes: Arc<Mutex<HashMap<String, u32>>>,
    _listener: Listener,
    _registry: RegistryRc,
    duration: Arc<AtomicCell<u64>>, // in milliseconds
    next_up: Option<(StreamRc, StreamListener<()>)>,
    playing: Option<(StreamRc, StreamListener<()>)>,
    core: CoreRc,
    _context: ContextRc,
    mainloop: ThreadLoopRc,
}

impl PipewireAudioOutput {
    pub fn try_new() -> anyhow::Result<Self> {
        let mainloop = unsafe { ThreadLoopRc::new(None, None) }?;
        let context = ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;

        let nodes = Arc::new(Mutex::new(HashMap::new()));
        let nodes_ref = nodes.clone();

        mainloop.start();

        let listener = registry
            .add_listener_local()
            .global(move |global| {
                if global.type_ == ObjectType::Node
                    && let Some(props) = global.props
                    && props.get("media.class") == Some("Audio/Sink")
                    && let Ok(mut node_lock) = nodes_ref.lock()
                {
                    node_lock.insert(
                        props.get("node.name").unwrap_or_default().to_owned(),
                        global.id,
                    );
                }
            })
            .register();

        Ok(Self {
            mainloop,
            _context: context,
            core,
            playing: None,
            next_up: None,
            duration: Arc::new(AtomicCell::new(0)),
            _registry: registry,
            _listener: listener,
            nodes,
        })
    }

    fn enqueue(&mut self, stream: (StreamRc, StreamListener<()>), autostart: lms_proto::AutoStart) {
        if self.playing.is_some() {
            self.next_up = Some(stream);
        } else {
            self.playing = Some(stream);
            if autostart == lms_proto::AutoStart::Auto {
                self.play();
            }
        }
    }

    fn play(&mut self) -> bool {
        if let Some(ref mut stream) = self.playing {
            let _pw_lock = self.mainloop.lock();
            stream.0.set_active(true).is_ok()
        } else {
            false
        }
    }
}

impl AudioOutput for PipewireAudioOutput {
    fn enqueue_new_stream(
        &mut self,
        mut decoder: VibeDecoder,
        stream_in: Sender<PlayerMsg>,
        stream_params: StreamParams,
        device: &Option<String>,
    ) -> anyhow::Result<()> {
        // Create an audio buffer to hold raw u8 samples
        let buf_size = {
            let num_samples = decoder.dur_to_samples(stream_params.output_threshold) as usize;
            num_samples.max(MIN_AUDIO_BUFFER_SIZE)
        };

        let mut audio_buf = Vec::with_capacity(buf_size);

        // Prefill audio buffer to threshold
        match decoder.fill_raw_buffer(&mut audio_buf, None) {
            Ok(_) => {}

            Err(DecoderError::StreamError(e)) => {
                warn!("Error reading data stream: {}", e);
                _ = stream_in.send(PlayerMsg::NotSupported);
                return Ok(());
            }
        };

        let pw_lock = self.mainloop.lock();
        let stream = match StreamRc::new(
            self.core.clone(),
            "Music",
            properties! {
                *pipewire::keys::MEDIA_TYPE => "Audio",
                *pipewire::keys::MEDIA_ROLE => "Music",
                *pipewire::keys::MEDIA_CATEGORY => "Playback",
                *pipewire::keys::AUDIO_CHANNELS => decoder.channels().to_string(),
            },
        ) {
            Ok(stream) => stream,
            Err(_) => {
                _ = stream_in.send(PlayerMsg::NotSupported);
                return Ok(());
            }
        };

        let duration = self.duration.clone();
        let stream_in_ref = stream_in.clone();
        let channels = decoder.channels();
        let rate = decoder.sample_rate();
        let mut state = ProcessState::Running;
        let on_process = move |stream: &Stream, _data: &mut _| {
            // Pipewire has a nasty habit of calling this callback even when
            // all data had been written and we're in the process of draining.
            // Therefore keep our own state to make sure we don't send multiple
            // end-of-decode messages to the server.

            let mut skip_time = SKIP.take();

            let end_of_decode = if state == ProcessState::Draining {
                true
            } else {
                loop {
                    let eod = match decoder.fill_raw_buffer(&mut audio_buf, None) {
                        Ok(eod) => eod,

                        Err(DecoderError::StreamError(e)) => {
                            warn!("Error reading data stream: {}", e);
                            _ = stream_in_ref.send(PlayerMsg::NotSupported);
                            state = ProcessState::Draining;
                            true
                        }
                    };

                    if skip_time > Duration::ZERO {
                        let mut bytes_to_skip =
                            (decoder.dur_to_samples(skip_time) * size_of::<f32>() as u64) as usize;
                        bytes_to_skip = bytes_to_skip.min(audio_buf.len());
                        audio_buf.drain(..bytes_to_skip);
                        let actual_skip_time =
                            decoder.samples_to_dur((bytes_to_skip / size_of::<f32>()) as _);
                        duration.fetch_add(actual_skip_time.as_millis() as _);
                        skip_time = skip_time.saturating_sub(actual_skip_time);

                        if audio_buf.is_empty() {
                            continue;
                        }
                    };
                    break eod;
                }
            };

            if audio_buf.is_empty() {
                _ = stream.flush(true);
            } else if let Some(mut pw_buf) = stream.dequeue_buffer() {
                let data = &mut pw_buf.datas_mut()[0];
                if let Some(buf_data) = data.data() {
                    let len = buf_data.len().min(audio_buf.len());
                    buf_data[..len].copy_from_slice(&audio_buf.drain(..len).collect::<Vec<u8>>());

                    if buf_data.len() > audio_buf.capacity() {
                        audio_buf.reserve(buf_data.len() - audio_buf.capacity());
                    }

                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = (size_of::<f32>() * channels) as _;
                    *chunk.size_mut() = len as _;

                    duration.fetch_add(
                        (len * 1000 / (size_of::<f32>() * channels * rate as usize)) as _,
                    );
                }
            }

            if end_of_decode && state == ProcessState::Running {
                _ = stream_in_ref.send(PlayerMsg::EndOfDecode);
                state = ProcessState::Draining;
            }
        };

        let stream_in_ref = stream_in.clone();
        let duration = self.duration.clone();
        let on_state_change = move |_stream: &Stream,
                                    _data: &mut _,
                                    old_state: StreamState,
                                    new_state: StreamState| {
            match (old_state, new_state) {
                (StreamState::Connecting, StreamState::Paused)
                | (StreamState::Connecting, StreamState::Streaming) => {
                    duration.store(0);
                    _ = stream_in_ref.send(PlayerMsg::TrackStarted);
                }

                // These messages (pause, unpause) are taken care of in the main thread
                // but may be useful if we move responsibilty for these
                // messages into the decode thread (here)

                // (StreamState::Streaming, StreamState::Paused) => {
                //     _ = stream_in_ref.send(PlayerMsg::Pause);
                // }

                // (StreamState::Paused, StreamState::Streaming) => {
                //     _ = stream_in_ref.send(PlayerMsg::Unpause);
                // }
                (StreamState::Error(_), _) | (_, StreamState::Error(_)) => {
                    _ = stream_in_ref.send(PlayerMsg::NotSupported);
                }

                _ => {}
            }
        };

        let stream_in_ref = stream_in.clone();
        let on_drained = move |_stream: &Stream, _data: &mut _| {
            _ = stream_in_ref.send(PlayerMsg::Drained);
        };

        let listener = stream
            .add_local_listener::<()>()
            .process(on_process)
            .drained(on_drained)
            .state_changed(on_state_change)
            .register()?;

        let pw_flags = StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::INACTIVE;

        let mut audio_info = AudioInfoRaw::new();
        audio_info.set_format(AudioFormat::F32LE);
        audio_info.set_rate(rate);
        audio_info.set_channels(channels as _);
        let mut position = [0; pipewire::spa::param::audio::MAX_CHANNELS];
        position[0] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_FL;
        position[1] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_FR;
        audio_info.set_position(position);

        let values: Vec<u8> = PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &pipewire::spa::pod::Value::Object(pipewire::spa::pod::Object {
                type_: pipewire::spa::sys::SPA_TYPE_OBJECT_Format,
                id: pipewire::spa::sys::SPA_PARAM_EnumFormat,
                properties: audio_info.into(),
            }),
        )
        .unwrap_or_default()
        .0
        .into_inner();

        let mut params = [Pod::from_bytes(&values).unwrap()];

        let node_id = match self.nodes.lock() {
            Ok(nodes_lock) => device
                .as_ref()
                .and_then(|dev_name| nodes_lock.get(dev_name).copied()),
            Err(_) => None,
        };

        if let Err(e) = stream.connect(Direction::Output, node_id, pw_flags, &mut params) {
            _ = stream_in.send(PlayerMsg::NotSupported);
            return Err(e.into());
        }
        drop(pw_lock);

        _ = stream_in.send(PlayerMsg::StreamEstablished);
        self.enqueue((stream, listener), stream_params.autostart);

        Ok(())
    }

    fn flush(&mut self) {
        self.stop();
    }

    fn get_dur(&self) -> Duration {
        Duration::from_millis(self.duration.load())
    }

    fn pause(&mut self) -> bool {
        if let Some(ref mut stream) = self.playing {
            let _pw_lock = self.mainloop.lock();
            stream.0.set_active(false).is_ok()
        } else {
            false
        }
    }

    fn shift(&mut self) {
        let _pw_lock = self.mainloop.lock();
        if let Some(ref mut stream) = self.playing {
            _ = stream.0.set_active(false);
            _ = stream.0.disconnect();
        }
        self.playing = self.next_up.take();
    }

    fn stop(&mut self) {
        let _pw_lock = self.mainloop.lock();
        if let Some(ref mut stream) = self.playing {
            _ = stream.0.set_active(false);
            _ = stream.0.disconnect();
        }

        if let Some(ref mut stream) = self.next_up {
            _ = stream.0.set_active(false);
            _ = stream.0.disconnect();
        }

        self.playing = None;
        self.next_up = None;
        self.duration.store(0);
    }

    fn unpause(&mut self) -> bool {
        self.play()
    }

    fn get_output_device_names(&self) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let mut ret = Vec::new();
        let (s, r) = bounded(1);

        let lock = self.mainloop.lock();
        let registry = self.core.get_registry()?;
        let _listener = registry
            .add_listener_local()
            .global(move |global| {
                if global.type_ == ObjectType::Node
                    && let Some(props) = global.props
                    && props.get("media.class") == Some("Audio/Sink")
                {
                    let device_name = props.get("node.name").unwrap_or_default().to_owned();
                    let device_desc = props.get("node.description").map(|s| s.to_owned());
                    _ = s.send((device_name, device_desc));
                }
            })
            .register();

        drop(lock);
        while let Ok(item) = r.recv_timeout(Duration::from_millis(250)) {
            ret.push(item);
        }
        self.mainloop.stop();

        Ok(ret)
    }
}

impl Drop for PipewireAudioOutput {
    fn drop(&mut self) {
        self.stop();
        self.mainloop.stop();
    }
}
