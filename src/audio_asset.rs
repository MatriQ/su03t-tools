//! Lossless source recordings and an explicit external FFmpeg conversion step.
//! No audio device is opened here and no user file is overwritten.
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_SECONDS: u32 = 60;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Clip {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}
impl Clip {
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
    pub fn wav(&self, start: f64, end: f64, gain: f32) -> Result<Vec<u8>> {
        ensure!(
            (8000..=192000).contains(&self.sample_rate),
            "Unsupported source sample rate"
        );
        ensure!(
            start.is_finite()
                && end.is_finite()
                && start >= 0.0
                && end > start
                && end <= self.duration() + 0.001,
            "Invalid recording trim range"
        );
        ensure!(
            gain.is_finite() && (0.0..=4.0).contains(&gain),
            "Invalid recording gain"
        );
        let from = (start * self.sample_rate as f64) as usize;
        let to = ((end * self.sample_rate as f64) as usize).min(self.samples.len());
        ensure!(
            to > from && to - from <= self.sample_rate as usize * MAX_SECONDS as usize,
            "Recording is empty or too long"
        );
        let mut output = Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: self.sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut output, spec)?;
            for &sample in &self.samples[from..to] {
                ensure!(sample.is_finite(), "Non-finite audio sample");
                writer.write_sample(
                    (sample.mul_add(gain, 0.0).clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16,
                )?;
            }
            writer.finalize()?;
        }
        Ok(output.into_inner())
    }
    pub fn from_wav(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_FILE_BYTES, "WAV exceeds 32 MiB");
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).context("Read WAV recording")?;
        let spec = reader.spec();
        ensure!(
            (1..=8).contains(&spec.channels) && (8000..=192000).contains(&spec.sample_rate),
            "Unsupported WAV channel count or sample rate"
        );
        let duration = reader.duration() as usize;
        ensure!(
            duration > 0 && duration <= spec.sample_rate as usize * MAX_SECONDS as usize,
            "WAV must contain at most 60 seconds of audio"
        );
        let values: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<std::result::Result<_, _>>()?,
            hound::SampleFormat::Int => {
                ensure!(
                    (1..=32).contains(&spec.bits_per_sample),
                    "Unsupported WAV bit depth"
                );
                let scale = 2f32.powi(spec.bits_per_sample as i32 - 1);
                reader
                    .samples::<i32>()
                    .map(|sample| sample.map(|value| value as f32 / scale))
                    .collect::<std::result::Result<_, _>>()?
            }
        };
        ensure!(
            values.len().is_multiple_of(spec.channels as usize),
            "Incomplete WAV frame"
        );
        ensure!(
            values.iter().all(|s| s.is_finite()),
            "Non-finite audio sample"
        );
        Ok(Self {
            sample_rate: spec.sample_rate,
            samples: values
                .chunks_exact(spec.channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / spec.channels as f32)
                .collect(),
        })
    }
}

pub fn load_wav(path: &Path) -> Result<Clip> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    Clip::from_wav(&bytes)
}

pub fn default_ffmpeg() -> PathBuf {
    for path in [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ] {
        if Path::new(path).is_file() {
            return path.into();
        }
    }
    "ffmpeg".into()
}

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..20 {
            let path = std::env::temp_dir().join(format!(
                "su03t-audio-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("Could not create private audio workspace")
    }
    fn run(
        &self,
        executable: &Path,
        arguments: &[&str],
        input: &[u8],
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>> {
        std::fs::write(self.0.join("input"), input)?;
        let errors = std::fs::File::create(self.0.join("errors"))?;
        let mut command = Command::new(executable);
        command
            .current_dir(&self.0)
            .args(["-v", "error", "-nostdin", "-y", "-i", "input"])
            .args(arguments)
            .arg("output")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(errors);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .context("Start FFmpeg; install it or select its executable in the audio panel")?;
        let started = Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("Audio conversion cancelled or timed out");
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let mut error = String::new();
                        std::fs::File::open(self.0.join("errors"))?
                            .take(8192)
                            .read_to_string(&mut error)?;
                        bail!("FFmpeg failed: {}", error.trim());
                    }
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
        }
        let mut bytes = Vec::new();
        std::fs::File::open(self.0.join("output"))?
            .take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_FILE_BYTES,
            "Invalid or oversized audio output"
        );
        Ok(bytes)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn encode_reply(executable: &Path, wav: &[u8], cancel: &AtomicBool) -> Result<Vec<u8>> {
    let bytes = Workspace::new()?.run(
        executable,
        &[
            "-map_metadata",
            "-1",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "16k",
            "-write_xing",
            "0",
            "-id3v2_version",
            "0",
            "-write_id3v1",
            "0",
            "-f",
            "mp3",
        ],
        wav,
        cancel,
    )?;
    let length = crate::firmware_audio::mp3_frame_bytes(&bytes)?;
    Ok(bytes[..length].to_vec())
}
pub fn decode_reply(executable: &Path, mp3: &[u8], cancel: &AtomicBool) -> Result<Clip> {
    let frames = crate::firmware_audio::mp3_frame_bytes(mp3)? / 72;
    ensure!(
        frames * 576 <= 16000 * MAX_SECONDS as usize,
        "Reply exceeds the 60 second preview limit; extract the MP3 instead"
    );
    let bytes = Workspace::new()?.run(
        executable,
        &[
            "-ac",
            "1",
            "-ar",
            "16000",
            "-t",
            "60",
            "-codec:a",
            "pcm_s16le",
            "-f",
            "s16le",
        ],
        mp3,
        cancel,
    )?;
    ensure!(
        bytes.len().is_multiple_of(2),
        "Incomplete decoded audio sample"
    );
    Ok(Clip {
        sample_rate: 16000,
        samples: bytes
            .chunks_exact(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
            .collect(),
    })
}
pub fn cancellation() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wav_trim_gain_and_roundtrip_keep_original() {
        let clip = Clip {
            sample_rate: 16000,
            samples: vec![0.25; 16000],
        };
        let wav = clip.wav(0.25, 0.75, 2.0).unwrap();
        let decoded = Clip::from_wav(&wav).unwrap();
        assert_eq!(decoded.samples.len(), 8000);
        assert!((decoded.samples[0] - 0.5).abs() < 0.0001);
        assert_eq!(clip.samples.len(), 16000);
        assert!(clip.wav(0.5, 0.25, 1.0).is_err());
        assert!(clip.wav(0.0, 2.0, 1.0).is_err());
        assert!(Clip::from_wav(b"not audio").is_err());
        let invalid = Clip {
            sample_rate: 16000,
            samples: vec![f32::NAN; 160],
        };
        assert!(invalid.wav(0.0, 0.01, 1.0).is_err());
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;
    #[test]
    #[ignore = "requires local FFmpeg with libmp3lame; uses synthetic audio only"]
    fn synthetic_encode_decode_and_cancellation() {
        let clip = Clip {
            sample_rate: 48000,
            samples: (0..24000)
                .map(|i| (i as f32 * 0.0576).sin() * 0.25)
                .collect(),
        };
        let wav = clip.wav(0.05, 0.4, 0.7).unwrap();
        let cancel = AtomicBool::new(false);
        let ffmpeg = default_ffmpeg();
        let mp3 = encode_reply(&ffmpeg, &wav, &cancel).unwrap();
        assert_eq!(
            crate::firmware_audio::mp3_frame_bytes(&mp3).unwrap(),
            mp3.len()
        );
        let decoded = decode_reply(&ffmpeg, &mp3, &cancel).unwrap();
        assert_eq!(decoded.sample_rate, 16000);
        assert!(decoded.duration() >= 0.35 && decoded.duration() < 0.6);
        assert!(decoded.samples.iter().any(|s| s.abs() > 0.05));
        cancel.store(true, Ordering::Relaxed);
        assert!(encode_reply(&ffmpeg, &wav, &cancel).is_err());
        assert_eq!(clip.samples.len(), 24000);
    }
}
