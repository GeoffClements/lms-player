use std::time::Duration;

use crossbeam::channel::Sender;

use crate::{decode::{StreamParams, VibeDecoder}, message::PlayerMsg};

#[cfg(feature = "pipewire")]
use crate::pipewire_out::PipewireAudioOutput;
#[cfg(feature = "pulse")]
use crate::pulse_out::PulseAudioOutput;
#[cfg(feature = "rodio")]
use crate::rodio_out::RodioAudioOutput;

pub trait AudioOutput {
    fn enqueue_new_stream(
        &mut self,
        decoder: VibeDecoder,
        stream_in: Sender<PlayerMsg>,
        stream_params: StreamParams,
        device: &Option<String>,
    ) -> anyhow::Result<()>;

    fn unpause(&mut self) -> bool;

    fn pause(&mut self) -> bool;

    fn stop(&mut self);

    fn flush(&mut self);

    fn shift(&mut self);

    fn get_dur(&self) -> Duration;

    fn get_output_device_names(&self) -> anyhow::Result<Vec<(String, Option<String>)>>;
}

pub fn make_audio_output(
    system: &str,
    #[cfg(feature = "rodio")] device: &Option<String>,
) -> anyhow::Result<Box<dyn AudioOutput>> {
    Ok(match system {
        #[cfg(feature = "pulse")]
        "pulse" => Box::new(PulseAudioOutput::try_new()?),
        #[cfg(feature = "pipewire")]
        "pipewire" => Box::new(PipewireAudioOutput::try_new()?),
        #[cfg(feature = "rodio")]
        "rodio" => Box::new(RodioAudioOutput::try_new(device)?),
        _ => unreachable!(),
    })
}
