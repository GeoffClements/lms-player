// src/pulse_out.rs
// PulseAudio audio output implementation
//
// Required system dependencies:
// - PulseAudio development libraries (libpulse-dev on Debian-based systems)

use std::{cell::RefCell, ops::Deref, rc::Rc, sync::Arc, time::Duration};

use anyhow::anyhow;
use crossbeam::{
    atomic::AtomicCell,
    channel::{bounded, Sender},
};
use log::warn;
use pulse::{
    callbacks::ListResult,
    context::{Context, FlagSet as CxFlagSet, State},
    mainloop::threaded::Mainloop,
    sample::Spec,
    stream::{FlagSet as SmFlagSet, SeekMode, Stream},
};

use crate::{
    audio_out::AudioOutput,
    decode::{DecoderError, VibeDecoder},
    message::PlayerMsg,
    decode::StreamParams,
    state::SKIP,
};

const MIN_AUDIO_BUFFER_SIZE: usize = 8 * 1024;

#[derive(PartialEq)]
enum WriteState {
    Starting,
    Running,
    Draining,
    Drained,
}

pub struct PulseAudioOutput {
    mainloop: Rc<RefCell<Mainloop>>,
    context: Rc<RefCell<Context>>,
    playing: Option<Rc<RefCell<Stream>>>,
    next_up: Option<Rc<RefCell<Stream>>>,
}

impl PulseAudioOutput {
    pub fn try_new() -> anyhow::Result<Self> {
        let mainloop = Rc::new(RefCell::new(
            Mainloop::new().ok_or(pulse::error::Code::ConnectionRefused)?,
        ));

        let context = Rc::new(RefCell::new(
            Context::new(mainloop.borrow_mut().deref(), "Vibe")
                .ok_or(pulse::error::Code::ConnectionRefused)?,
        ));

        // Context state change callback
        let mainloop_ref = mainloop.clone();
        let context_ref = context.clone();
        context
            .borrow_mut()
            .set_state_callback(Some(Box::new(move || {
                let state = unsafe { (*context_ref.as_ptr()).get_state() };
                match state {
                    State::Ready | State::Terminated | State::Failed => unsafe {
                        (*mainloop_ref.as_ptr()).signal(false);
                    },
                    _ => {}
                }
            })));

        context
            .borrow_mut()
            .connect(None, CxFlagSet::NOFLAGS, None)?;
        mainloop.borrow_mut().lock();
        mainloop.borrow_mut().start()?;

        // Wait for context to be ready
        loop {
            match context.borrow().get_state() {
                State::Ready => {
                    break;
                }
                State::Failed | State::Terminated => {
                    mainloop.borrow_mut().unlock();
                    mainloop.borrow_mut().stop();
                    return Err(anyhow!("Unable to connect with pulseaudio"));
                }
                _ => mainloop.borrow_mut().wait(),
            }
        }

        context.borrow_mut().set_state_callback(None);
        mainloop.borrow_mut().unlock();

        Ok(PulseAudioOutput {
            mainloop,
            context,
            playing: None,
            next_up: None,
        })
    }

    fn connect_stream(
        &mut self,
        stream: &mut Rc<RefCell<Stream>>,
        device: &Option<String>,
    ) -> anyhow::Result<()> {
        self.mainloop.borrow_mut().lock();

        // Stream state change callback
        let mainloop_ref = self.mainloop.clone();
        let stream_ref = self.context.clone();
        stream
            .borrow_mut()
            .set_state_callback(Some(Box::new(move || {
                let state = unsafe { (*stream_ref.as_ptr()).get_state() };
                match state {
                    State::Ready | State::Failed | State::Terminated => unsafe {
                        (*mainloop_ref.as_ptr()).signal(false);
                    },
                    _ => {}
                }
            })));

        let flags =
            SmFlagSet::START_CORKED | SmFlagSet::AUTO_TIMING_UPDATE | SmFlagSet::INTERPOLATE_TIMING;

        stream
            .borrow_mut()
            .connect_playback(device.as_deref(), None, flags, None, None)?;

        // Wait for stream to be ready
        loop {
            match stream.borrow().get_state() {
                pulse::stream::State::Ready => {
                    break;
                }
                pulse::stream::State::Failed | pulse::stream::State::Terminated => {
                    self.mainloop.borrow_mut().unlock();
                    self.mainloop.borrow_mut().stop();
                    return Err(anyhow!(pulse::error::PAErr(
                        pulse::error::Code::ConnectionTerminated as i32,
                    )));
                }
                _ => {
                    self.mainloop.borrow_mut().wait();
                }
            }
        }

        stream.borrow_mut().set_state_callback(None);
        self.mainloop.borrow_mut().unlock();

        Ok(())
    }

    fn play(&mut self) -> bool {
        self.unpause()
    }

    fn enqueue(&mut self, stream: Rc<RefCell<Stream>>, autostart: slimproto::proto::AutoStart) {
        if self.playing.is_some() {
            self.next_up = Some(stream);
        } else {
            self.playing = Some(stream);
            if autostart == slimproto::proto::AutoStart::Auto {
                self.play();
            }
        }
    }

    fn set_cork_state(&self, state: bool) -> bool {
        let is_success = Arc::new(AtomicCell::new(false));

        if let Some(ref stream) = self.playing {
            let is_success_ref = is_success.clone();
            self.mainloop.borrow_mut().lock();
            let op = {
                let mainloop = self.mainloop.clone();
                stream.borrow_mut().set_corked_state(
                    state,
                    Some(Box::new(move |success| {
                        is_success_ref.store(success);
                        unsafe {
                            (*mainloop.as_ptr()).signal(false);
                        }
                    })),
                )
            };

            while op.get_state() != pulse::operation::State::Done {
                self.mainloop.borrow_mut().wait();
            }

            self.mainloop.borrow_mut().unlock();
        };

        is_success.load()
    }
}

impl AudioOutput for PulseAudioOutput {
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
        loop {
            match decoder.fill_raw_buffer(&mut audio_buf, None) {
                Ok(_) => {}

                // Err(DecoderError::EndOfDecode) => {
                //     stream_in.send(PlayerMsg::EndOfDecode).ok();
                // }
                Err(DecoderError::StreamError(e)) => {
                    warn!("Error reading data stream: {}", e);
                    _ = stream_in.send(PlayerMsg::NotSupported);
                    return Ok(());
                }

                Err(DecoderError::Retry) => {
                    continue;
                }
            };
            break;
        }

        let spec = Spec {
            format: pulse::sample::Format::F32le,
            rate: decoder.sample_rate(),
            channels: decoder.channels() as _,
        };

        self.mainloop.borrow_mut().lock();
        let stream = match Stream::new(&mut self.context.borrow_mut(), "Music", &spec, None) {
            Some(stream) => stream,
            None => {
                _ = stream_in.send(PlayerMsg::NotSupported);
                return Ok(());
            }
        };
        self.mainloop.borrow_mut().unlock();

        let mut stream = Rc::new(RefCell::new(stream));
        let stream_ref = stream.clone();
        let state = Rc::new(RefCell::new(WriteState::Starting));
        let state_ref = state.clone();
        let stream_in_ref = stream_in.clone();
        let on_write = move |len: usize| {
            // Pulseaudio has a nasty habit of calling this callback even when
            // all data had been written and we're in the process of draining.
            // Therefore keep our own state to make sure we don't send multiple
            // end-of-decode messages to the server.

            if *state_ref.borrow() == WriteState::Drained {
                return;
            }

            if *state_ref.borrow() == WriteState::Starting {
                _ = stream_in_ref.send(PlayerMsg::TrackStarted);
                *state_ref.borrow_mut() = WriteState::Running;
            }

            if *state_ref.borrow() != WriteState::Draining {
                let end_of_decode = loop {
                    let eod = match decoder.fill_raw_buffer(&mut audio_buf, Some(len)) {
                        Ok(eod) => eod,
                        Err(DecoderError::StreamError(e)) => {
                            warn!("Error reading data stream: {}", e);
                            _ = stream_in_ref.send(PlayerMsg::NotSupported);
                            *state_ref.borrow_mut() = WriteState::Draining;
                            true
                        }
                        Err(DecoderError::Retry) => continue,
                    };
                    break eod;
                };

                if !audio_buf.is_empty() {
                    let buf_len = audio_buf.len().min(len);
                    let offset =
                        (decoder.dur_to_samples(SKIP.take()) * size_of::<f32>() as u64) as i64;
                    if let Some(stream) = unsafe { stream_ref.as_ptr().as_mut() } {
                        _ = stream.write_copy(
                            &audio_buf.drain(..buf_len).collect::<Vec<u8>>(),
                            offset,
                            SeekMode::Relative,
                        );
                    }
                }

                if end_of_decode {
                    _ = stream_in_ref.send(PlayerMsg::EndOfDecode);
                    *state_ref.borrow_mut() = WriteState::Draining;
                }
            }

            if *state_ref.borrow() == WriteState::Draining && audio_buf.is_empty() {
                *state_ref.borrow_mut() = WriteState::Drained;
            }
        };

        // Add callback to write audio data
        self.mainloop.borrow_mut().lock();
        stream
            .borrow_mut()
            .set_write_callback(Some(Box::new(on_write)));

        // Add callback to detect end of track
        let state_ref = state.clone();
        let stream_in_ref = stream_in.clone();
        stream
            .borrow_mut()
            .set_underflow_callback(Some(Box::new(move || {
                if *state_ref.borrow() == WriteState::Drained {
                    _ = stream_in_ref.send(PlayerMsg::Drained);
                }
            })));
        self.mainloop.borrow_mut().unlock();

        // Connect playback stream
        self.connect_stream(&mut stream, device)?;

        _ = stream_in.send(PlayerMsg::StreamEstablished);
        self.enqueue(stream, stream_params.autostart);

        Ok(())
    }

    fn unpause(&mut self) -> bool {
        self.set_cork_state(false)
    }

    fn pause(&mut self) -> bool {
        self.set_cork_state(true)
    }

    fn stop(&mut self) {
        if let Some(ref stream) = self.playing {
            self.mainloop.borrow_mut().lock();
            stream.borrow_mut().set_write_callback(None);
            _ = stream.borrow_mut().disconnect();
            self.mainloop.borrow_mut().unlock();
        }

        self.next_up = None;
        self.playing = None;
    }

    fn flush(&mut self) {
        self.stop();
    }

    fn shift(&mut self) {
        self.playing = self.next_up.take();
    }

    fn get_dur(&self) -> Duration {
        match self.playing {
            Some(ref stream) => {
                let dur = stream
                    .borrow()
                    .get_time()
                    .unwrap_or_default()
                    .unwrap_or_default();
                Duration::from_micros(dur.0)
            }
            None => Duration::ZERO,
        }
    }

    fn get_output_device_names(&self) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let mut ret = Vec::new();
        let (s, r) = bounded(1);

        self.mainloop.borrow_mut().lock();
        let _op = self
            .context
            .borrow_mut()
            .introspect()
            .get_sink_info_list(move |list_result| match list_result {
                ListResult::Item(item) => {
                    let name = item.name.to_owned().unwrap_or_default().to_string();
                    let description = item.description.to_owned().map(|n| n.to_string());
                    s.send(Some((name, description))).ok();
                }
                ListResult::End | ListResult::Error => {
                    s.send(None).ok();
                }
            });
        self.mainloop.borrow_mut().unlock();

        while let Some(item) = r.recv()? {
            ret.push(item);
        }

        Ok(ret)
    }
}

impl Drop for PulseAudioOutput {
    fn drop(&mut self) {
        self.context.borrow_mut().disconnect();
    }
}
