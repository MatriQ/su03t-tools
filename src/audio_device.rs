//! Audio hardware is accessed only by explicit UI start/refresh actions.
//! Streams live on their owning worker; dropping a task requests a bounded stop.
use anyhow::{Context, Result, bail, ensure};
use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
use su03t_update::audio_asset::{Clip, MAX_SECONDS};

pub fn inputs() -> Result<Vec<(String, String)>> {
    cpal::default_host()
        .input_devices()?
        .map(|d| Ok((d.id()?.to_string(), d.to_string())))
        .collect()
}
pub enum Outcome {
    Recorded(Clip),
    Played,
}
pub struct Task {
    stop: Arc<AtomicBool>,
    pub result: Receiver<Result<Outcome>>,
    pub peak: Arc<AtomicU32>,
    pub seconds: Arc<AtomicU32>,
}
impl Task {
    #[cfg(test)]
    pub(super) fn simulated() -> (Self, mpsc::Sender<Result<Outcome>>) {
        let (sender, result) = mpsc::channel();
        (
            Self {
                stop: Arc::new(AtomicBool::new(false)),
                result,
                peak: Arc::new(AtomicU32::new(0)),
                seconds: Arc::new(AtomicU32::new(0)),
            },
            sender,
        )
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn record(id: String) -> Self {
        Self::spawn(move |stop, peak, seconds| {
            record(&id, stop, peak, seconds).map(Outcome::Recorded)
        })
    }
    pub fn play(clip: Clip) -> Self {
        Self::spawn(move |stop, _, _| play(clip, stop).map(|()| Outcome::Played))
    }
    fn spawn(
        work: impl FnOnce(Arc<AtomicBool>, Arc<AtomicU32>, Arc<AtomicU32>) -> Result<Outcome>
        + Send
        + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let seconds = Arc::new(AtomicU32::new(0));
        let (tx, result) = mpsc::channel();
        let (s, p, t) = (stop.clone(), peak.clone(), seconds.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work(s, p, t));
        });
        Self {
            stop,
            result,
            peak,
            seconds,
        }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Capture {
    clip: Clip,
    channels: usize,
    peak: Arc<AtomicU32>,
    seconds: Arc<AtomicU32>,
}
impl Capture {
    fn append<T: Sample>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        let limit = self.clip.sample_rate as usize * MAX_SECONDS as usize;
        let mut peak = 0f32;
        for frame in data
            .chunks_exact(self.channels)
            .take(limit.saturating_sub(self.clip.samples.len()))
        {
            let value =
                frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / self.channels as f32;
            let value = if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            peak = peak.max(value.abs());
            self.clip.samples.push(value);
        }
        self.peak.store((peak * 1000.0) as u32, Ordering::Relaxed);
        self.seconds
            .store((self.clip.duration() * 1000.0) as u32, Ordering::Relaxed);
    }
}
fn record(
    id: &str,
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    seconds: Arc<AtomicU32>,
) -> Result<Clip> {
    let host = cpal::default_host();
    let device = host
        .device_by_id(&id.parse()?)
        .context("Selected microphone disconnected; refresh inputs")?;
    let config = device
        .default_input_config()
        .context("Microphone permission/configuration unavailable")?;
    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    ensure!(
        (8000..=192000).contains(&rate) && (1..=8).contains(&channels),
        "Unsupported microphone rate/channels"
    );
    let capture = Arc::new(Mutex::new(Capture {
        clip: Clip {
            sample_rate: rate,
            samples: Vec::with_capacity(rate as usize * MAX_SECONDS as usize),
        },
        channels,
        peak,
        seconds,
    }));
    let (tx, errors) = mpsc::sync_channel(1);
    let err = move |e: cpal::Error| {
        let _ = tx.try_send(e);
    };
    let data = capture.clone();
    macro_rules! stream {
        ($t:ty) => {
            device.build_input_stream(
                config.into(),
                move |values: &[$t], _| {
                    if let Ok(mut c) = data.lock() {
                        c.append(values);
                    }
                },
                err,
                None,
            )?
        };
    }
    let stream = match config.sample_format() {
        SampleFormat::F32 => stream!(f32),
        SampleFormat::I16 => stream!(i16),
        SampleFormat::U16 => stream!(u16),
        SampleFormat::I32 => stream!(i32),
        SampleFormat::F64 => stream!(f64),
        other => bail!("Unsupported microphone sample format: {other}"),
    };
    if stop.load(Ordering::Relaxed) {
        bail!("Recording cancelled");
    }
    stream
        .play()
        .context("Start microphone; check system microphone permission")?;
    let started = Instant::now();
    while !stop.load(Ordering::Relaxed)
        && started.elapsed() < Duration::from_secs(MAX_SECONDS as u64)
    {
        if let Ok(error) = errors.try_recv() {
            bail!("Microphone stream: {error}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    let mut capture = capture
        .lock()
        .map_err(|_| anyhow::anyhow!("Recording buffer unavailable"))?;
    ensure!(
        !capture.clip.samples.is_empty(),
        "No audio captured; check microphone permission/input"
    );
    Ok(Clip {
        sample_rate: rate,
        samples: std::mem::take(&mut capture.clip.samples),
    })
}

// Linear resampling for monitoring only; firmware conversion uses FFmpeg.
fn preview_sample(clip: &Clip, output_frame: usize, output_rate: u32) -> Option<f32> {
    let position = output_frame as f64 * clip.sample_rate as f64 / output_rate as f64;
    let index = position.floor() as usize;
    let a = *clip.samples.get(index)?;
    let b = clip.samples.get(index + 1).copied().unwrap_or(a);
    Some(a + (b - a) * (position - index as f64) as f32)
}
fn output<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    clip: Clip,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    let (tx, errors) = mpsc::sync_channel(1);
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let mut frame = 0;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            for samples in data.chunks_mut(channels) {
                let value = preview_sample(&clip, frame, rate);
                samples.fill(T::from_sample(value.unwrap_or(0.0)));
                frame += 1;
                if value.is_none() {
                    finished.store(true, Ordering::Relaxed);
                }
            }
        },
        move |error| {
            let _ = tx.try_send(error);
        },
        None,
    )?;
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    stream.play()?;
    let started = Instant::now();
    while !stop.load(Ordering::Relaxed) && !done.load(Ordering::Relaxed) {
        if let Ok(error) = errors.try_recv() {
            bail!("Playback stream: {error}");
        }
        ensure!(
            started.elapsed() < Duration::from_secs(MAX_SECONDS as u64 + 5),
            "Playback timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let the final queued output buffer drain, unless explicitly stopped.
    let drain = Instant::now();
    while !stop.load(Ordering::Relaxed) && drain.elapsed() < Duration::from_millis(150) {
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    Ok(())
}
fn play(clip: Clip, stop: Arc<AtomicBool>) -> Result<()> {
    ensure!(
        !clip.samples.is_empty()
            && clip.sample_rate >= 8000
            && clip.duration() <= MAX_SECONDS as f64,
        "Invalid preview clip"
    );
    let device = cpal::default_host()
        .default_output_device()
        .context("No default speaker/output device")?;
    let config = device.default_output_config()?;
    ensure!(
        config.channels() > 0 && config.sample_rate() > 0,
        "Invalid output configuration"
    );
    match config.sample_format() {
        SampleFormat::F32 => output::<f32>(&device, config.into(), clip, stop),
        SampleFormat::I16 => output::<i16>(&device, config.into(), clip, stop),
        SampleFormat::U16 => output::<u16>(&device, config.into(), clip, stop),
        SampleFormat::I32 => output::<i32>(&device, config.into(), clip, stop),
        SampleFormat::F64 => output::<f64>(&device, config.into(), clip, stop),
        other => bail!("Unsupported output sample format: {other}"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_mixes_bounds_and_preserves_source_without_hardware() {
        let peak = Arc::new(AtomicU32::new(0));
        let seconds = Arc::new(AtomicU32::new(0));
        let mut c = Capture {
            clip: Clip {
                sample_rate: 8000,
                samples: vec![],
            },
            channels: 2,
            peak: peak.clone(),
            seconds,
        };
        c.append(&[i16::MAX, i16::MAX, 0, 0]);
        assert!(c.clip.samples[0] > 0.99);
        assert_eq!(c.clip.samples[1], 0.0);
        assert!(peak.load(Ordering::Relaxed) >= 999);
        c.append(&vec![f32::NAN; 8000 * 2 * 61]);
        assert_eq!(c.clip.samples.len(), 8000 * 60);
        assert!(c.clip.samples.iter().all(|s| s.is_finite()));
    }
    #[test]
    fn monitoring_resamples_and_stops_at_end() {
        let clip = Clip {
            sample_rate: 16000,
            samples: vec![0.0, 1.0],
        };
        assert_eq!(preview_sample(&clip, 1, 32000), Some(0.5));
        assert_eq!(preview_sample(&clip, 4, 32000), None);
    }
}
