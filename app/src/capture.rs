//! Nimmt das Standard-Mikrofon auf und liefert 16 kHz / mono / 16 Bit.
//! Jede Aufnahme bekommt einen frischen Stream, damit Gerätewechsel (USB, Headset) nicht stören.

use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tilit_core::audio::{pcm_bytes, Resampler};
use tilit_core::live::LiveInput;
use tilit_core::session::{locked, Recorder};
use tilit_core::PushToTalkAction;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

type Samples = Arc<Mutex<Vec<i16>>>;
type Chunks = Option<UnboundedSender<LiveInput>>;

struct Running {
    stop: mpsc::Sender<oneshot::Sender<()>>,
}

pub struct WasapiRecorder {
    running: Mutex<Option<Running>>,
    samples: Samples,
    /// Ein weggefallenes Gerät beendet das Diktat wie ein Loslassen der Taste.
    actions: UnboundedSender<PushToTalkAction>,
}

impl WasapiRecorder {
    pub fn new(actions: UnboundedSender<PushToTalkAction>) -> Self {
        Self { running: Mutex::new(None), samples: Arc::default(), actions }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    samples: Samples,
    chunks: Chunks,
    actions: UnboundedSender<PushToTalkAction>,
) -> Result<Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let mut resampler = Resampler::new(config.sample_rate.0, usize::from(config.channels));
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let floats: Vec<f32> = data.iter().map(|sample| sample.to_sample::<f32>()).collect();
            let converted = resampler.process(&floats);
            if converted.is_empty() {
                return;
            }
            if let Some(chunks) = &chunks {
                let _ = chunks.send(LiveInput::Audio(pcm_bytes(&converted)));
            }
            locked(&samples).extend_from_slice(&converted);
        },
        move |error| {
            log::error!("audio.stream.failed: {error}");
            let _ = actions.send(PushToTalkAction::Stop);
        },
        None,
    )
}

fn open_stream(samples: Samples, chunks: Chunks, actions: UnboundedSender<PushToTalkAction>) -> Result<Stream, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("Kein Mikrofon verfügbar")?;
    let supported = device.default_input_config().map_err(|_| "Kein Mikrofon verfügbar")?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, samples, chunks, actions),
        SampleFormat::I16 => build::<i16>(&device, &config, samples, chunks, actions),
        SampleFormat::U16 => build::<u16>(&device, &config, samples, chunks, actions),
        SampleFormat::I32 => build::<i32>(&device, &config, samples, chunks, actions),
        _ => return Err("Audioformat nicht unterstützt".into()),
    };
    let stream = stream.map_err(|_| "Mikrofon-Zugriff fehlt (Windows-Datenschutz prüfen)")?;
    stream.play().map_err(|_| "Mikrofon konnte nicht gestartet werden")?;
    Ok(stream)
}

#[async_trait]
impl Recorder for WasapiRecorder {
    async fn start(&self, chunks: Chunks) -> Result<(), String> {
        locked(&self.samples).clear();
        let samples = self.samples.clone();
        let actions = self.actions.clone();
        let (ready, started) = oneshot::channel();
        let (stop, stop_rx) = mpsc::channel::<oneshot::Sender<()>>();
        // Ein Stream darf den Thread nicht wechseln: er lebt ganz auf diesem Thread.
        thread::spawn(move || match open_stream(samples, chunks, actions) {
            Ok(stream) => {
                let _ = ready.send(Ok(()));
                let stopped = stop_rx.recv();
                drop(stream);
                if let Ok(stopped) = stopped {
                    let _ = stopped.send(());
                }
            }
            Err(message) => {
                let _ = ready.send(Err(message));
            }
        });
        started.await.map_err(|_| "Mikrofon konnte nicht gestartet werden".to_owned())??;
        *locked(&self.running) = Some(Running { stop });
        Ok(())
    }

    async fn stop(&self) -> Vec<i16> {
        let running = locked(&self.running).take();
        if let Some(running) = running {
            let (stopped, done) = oneshot::channel();
            if running.stop.send(stopped).is_ok() {
                let _ = done.await;
            }
        }
        std::mem::take(&mut *locked(&self.samples))
    }
}
