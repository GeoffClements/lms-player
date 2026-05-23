// src/rodio_out.rs
// Rodio audio output implementation
//
// Required system dependencies:
// - ALSA development libraries (libasound2-dev on Debian-based systems)

use std::{collections::VecDeque, num::NonZero, time::Duration};

use anyhow::{self, bail, Context};
use crossbeam::channel::Sender;
use log::warn;
use rodio::{
    cpal::traits::HostTrait, nz, Device, DeviceSinkBuilder, DeviceTrait, MixerDeviceSink, Player,
    Source,
};
use slimproto::proto::AutoStart;

use crate::{
    audio_out::AudioOutput,
    decode::{VibeDecoder, DecoderError},
    message::PlayerMsg,
    decode::StreamParams,
    state::SKIP,
};

const MIN_AUDIO_BUFFER_SIZE: usize = 4 * 1024;

pub struct DecoderSource {
    decoder: VibeDecoder,
    frame: VecDeque<f32>,
    stream_in: Sender<PlayerMsg>,
    start_flag: bool,
    eod_flag: bool,
}

impl DecoderSource {
    fn new(decoder: VibeDecoder, capacity: usize, stream_in: Sender<PlayerMsg>) -> Self {
        DecoderSource {
            decoder,
            frame: VecDeque::with_capacity(capacity),
            stream_in,
            start_flag: true,
            eod_flag: false,
        }
    }
}

impl Source for DecoderSource {
    fn current_span_len(&self) -> Option<usize> {
        match self.frame.len() {
            0 => None,
            len => Some(len),
        }
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(self.decoder.channels() as u16).unwrap_or(nz!(2))
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(self.decoder.sample_rate()).unwrap_or(nz!(44100))
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Iterator for DecoderSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.start_flag {
            _ = self.stream_in.send(PlayerMsg::TrackStarted);
            self.start_flag = false;
        }

        if self.frame.len() < MIN_AUDIO_BUFFER_SIZE && !self.eod_flag {
            let mut audio_buf = Vec::<f32>::with_capacity(self.frame.capacity());
            let mut skip = SKIP.take();

            self.eod_flag = loop {
                let eod = match self
                    .decoder
                    .fill_sample_buffer(&mut audio_buf, Some(2 * MIN_AUDIO_BUFFER_SIZE))
                {
                    Ok(eod) => eod,

                    Err(DecoderError::StreamError(e)) => {
                        warn!("Error reading data stream: {}", e);
                        _ = self.stream_in.send(PlayerMsg::NotSupported);
                        true
                    }

                    Err(DecoderError::Retry) => {
                        continue;
                    }
                };

                if skip > Duration::ZERO {
                    let samples_to_skip =
                        self.decoder.dur_to_samples(skip).min(audio_buf.len() as _) as usize;
                    audio_buf.drain(..samples_to_skip);
                    skip = skip.saturating_sub(self.decoder.samples_to_dur(samples_to_skip as _));
                    if audio_buf.is_empty() {
                        continue;
                    }
                }

                if !audio_buf.is_empty() {
                    self.frame.extend(audio_buf);
                }

                break eod;
            };

            if self.eod_flag {
                _ = self.stream_in.send(PlayerMsg::EndOfDecode);
            }
        }

        self.frame.pop_front().or_else(|| {
            _ = self.stream_in.send(PlayerMsg::Drained);
            None
        })
    }
}

struct Stream {
    _output: MixerDeviceSink,
    sink: Player,
}

impl Stream {
    fn try_from_device(device: Device) -> anyhow::Result<Self> {
        let mut output = DeviceSinkBuilder::from_device(device)?.open_stream()?;
        output.log_on_drop(false);
        let sink = Player::connect_new(output.mixer());

        Ok(Self {
            _output: output,
            sink,
        })
    }

    fn play(&mut self, source: DecoderSource) {
        self.sink.append(source);
    }

    fn unpause(&self) {
        self.sink.play();
    }

    fn pause(&self) {
        self.sink.pause();
    }

    fn stop(&self) {
        self.sink.stop();
    }
}

pub struct RodioAudioOutput {
    host: rodio::cpal::Host,
    device: rodio::cpal::Device,
    playing: Option<Stream>,
}

impl RodioAudioOutput {
    pub fn try_new(device_name: &Option<String>) -> anyhow::Result<Self> {
        let host = rodio::cpal::default_host();
        let device = if let Some(dev_name) = device_name {
            match find_device(&host, dev_name) {
                Some(device) => device,
                None => {
                    bail!("Cannot find device: {dev_name}");
                }
            }
        } else {
            host.default_output_device().context("No default device")?
        };

        Ok(Self {
            host,
            device,
            playing: None,
        })
    }
}

impl AudioOutput for RodioAudioOutput {
    fn enqueue_new_stream(
        &mut self,
        decoder: VibeDecoder,
        stream_in: Sender<PlayerMsg>,
        stream_params: StreamParams,
        _device: &Option<String>,
    ) -> anyhow::Result<()> {
        let autostart = stream_params.autostart == AutoStart::Auto;

        let capacity = decoder.dur_to_samples(stream_params.output_threshold) as usize;
        let decoder_source = DecoderSource::new(decoder, capacity, stream_in.clone());

        _ = stream_in.send(PlayerMsg::StreamEstablished);

        if let Some(ref mut playing_stream) = self.playing {
            playing_stream.play(decoder_source);
        } else {
            let mut stream = Stream::try_from_device(self.device.clone())?;
            stream.play(decoder_source);
            if !autostart {
                stream.pause();
            }
            self.playing = Some(stream);
        }

        Ok(())
    }

    fn unpause(&mut self) -> bool {
        if let Some(ref stream) = self.playing {
            stream.unpause();
            return true;
        }
        false
    }

    fn pause(&mut self) -> bool {
        if let Some(ref stream) = self.playing {
            stream.pause();
            return true;
        }
        false
    }

    fn stop(&mut self) {
        if let Some(ref stream) = self.playing {
            stream.stop();
        }
        self.flush();
    }

    fn flush(&mut self) {
        self.playing = None;
    }

    fn shift(&mut self) {
        // Noop - uses rodio's stream append
    }

    fn get_dur(&self) -> Duration {
        match self.playing {
            Some(ref stream) => stream.sink.get_pos(),
            None => Duration::ZERO,
        }
    }

    fn get_output_device_names(&self) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let device_names = self
            .host
            .output_devices()?
            .filter(|d| d.supports_output())
            .filter_map(|d| d.description().ok())
            .map(|d| (d.name().to_owned(), d.driver().map(|s| s.to_owned())))
            .collect();

        Ok(device_names)
    }
}

fn find_device(host: &rodio::cpal::Host, name: &String) -> Option<Device> {
    let mut output_devices = host.output_devices().ok()?;
    output_devices.find(|d| match d.description() {
        Ok(desc) => desc.name() == *name,
        Err(_) => false,
    })
}
