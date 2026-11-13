//! Exact RX capture and readable event logs. Each stream keeps a bounded set of files.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn environment() -> String {
    format!(
        "SU-03T Tools {} | {}/{} | unix_ms={}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    )
}

#[derive(Clone)]
pub struct Options {
    pub prefix: PathBuf,
    pub max_bytes: u64,
    pub backups: usize,
}
impl Options {
    pub fn new(prefix: PathBuf) -> Self {
        Self {
            prefix,
            max_bytes: 16 * 1024 * 1024,
            backups: 3,
        }
    }
}

struct RollingFile {
    path: PathBuf,
    file: File,
    size: u64,
    max: u64,
    backups: usize,
}
impl RollingFile {
    fn open(path: PathBuf, max: u64, backups: usize) -> Result<Self> {
        ensure!(
            max > 0 && (1..=32).contains(&backups),
            "记录文件大小需大于0，备份数量需为1～32"
        );
        // New session never truncates a file from an earlier session.
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("创建记录 {} 失败；请使用新的文件名前缀", path.display()))?;
        Ok(Self {
            path,
            file,
            size: 0,
            max,
            backups,
        })
    }
    fn backup(&self, index: usize) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(format!(".{index}"));
        name.into()
    }
    fn rotate(&mut self) -> Result<()> {
        self.file.flush()?;
        for index in (1..=self.backups).rev() {
            let source = if index == 1 {
                self.path.clone()
            } else {
                self.backup(index - 1)
            };
            if source.exists() {
                let destination = self.backup(index);
                if destination.exists() {
                    std::fs::remove_file(&destination)?;
                }
                std::fs::rename(source, destination)?;
            }
        }
        self.file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)?;
        self.size = 0;
        Ok(())
    }
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            if self.size == self.max {
                self.rotate()?;
            }
            let n = bytes.len().min((self.max - self.size) as usize);
            self.file.write_all(&bytes[..n])?;
            self.size += n as u64;
            bytes = &bytes[n..];
        }
        Ok(())
    }
    fn write_record(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() as u64 <= self.max
            && self.size > 0
            && self.size + bytes.len() as u64 > self.max
        {
            self.rotate()?;
        }
        self.write_all(bytes)
    }
}
pub struct Recorder {
    raw: RollingFile,
    text: RollingFile,
}
impl Recorder {
    pub fn open(options: &Options, description: &str) -> Result<Self> {
        ensure!(
            options.max_bytes > 0 && (1..=32).contains(&options.backups),
            "记录文件大小需大于0，备份数量需为1～32"
        );
        let path = |suffix: &str| {
            let mut name = options.prefix.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        };
        // Reserve text first; do not open an existing RX file destructively.
        let text_path = path(".log");
        for suffix in [".log", ".rx.bin"] {
            for index in 0..=options.backups {
                let candidate = if index == 0 {
                    path(suffix)
                } else {
                    path(&format!("{suffix}.{index}"))
                };
                ensure!(
                    !candidate.exists(),
                    "记录文件已存在：{}；请选择新前缀",
                    candidate.display()
                );
            }
        }
        let text = RollingFile::open(text_path.clone(), options.max_bytes, options.backups)?;
        let raw = match RollingFile::open(path(".rx.bin"), options.max_bytes, options.backups) {
            Ok(raw) => raw,
            Err(error) => {
                drop(text);
                let _ = std::fs::remove_file(text_path);
                return Err(error);
            }
        };
        let mut recorder = Self { raw, text };
        recorder.system(
            Duration::ZERO,
            &format!(
                "{} | {} | RX原始流；TX仅表示主机写入；每文件{}字节/{}备份",
                environment(),
                description,
                options.max_bytes,
                options.backups
            ),
        )?;
        Ok(recorder)
    }
    pub fn event(&mut self, at: Duration, direction: &str, bytes: &[u8]) -> Result<()> {
        if direction == "RX" {
            self.raw.write_all(bytes).context("写入原始接收文件")?;
        }
        self.text
            .write_record(
                format!(
                    "[{:.3}s] {direction} HEX={}\n",
                    at.as_secs_f64(),
                    crate::serial_debug::hex(bytes)
                )
                .as_bytes(),
            )
            .context("写入文本记录")
    }
    pub fn system(&mut self, at: Duration, message: &str) -> Result<()> {
        self.text.write_record(
            format!(
                "[{:.3}s] SYS {}\n",
                at.as_secs_f64(),
                message.replace(['\r', '\n'], " ")
            )
            .as_bytes(),
        )
    }
    pub fn flush(&mut self) -> Result<()> {
        self.raw.file.flush()?;
        self.text.file.flush()?;
        Ok(())
    }
}

/// A separate ring for raw export: clearing visible rows never discards these bytes.
#[derive(Default)]
pub struct RawBuffer {
    bytes: std::collections::VecDeque<u8>,
    pub removed: u64,
}
impl RawBuffer {
    pub const LIMIT: usize = 1024 * 1024;
    pub fn push(&mut self, bytes: &[u8]) {
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(Self::LIMIT);
        self.removed = self.removed.saturating_add(overflow as u64);
        if bytes.len() >= Self::LIMIT {
            self.bytes.clear();
            self.bytes.extend(&bytes[bytes.len() - Self::LIMIT..]);
        } else {
            self.bytes.drain(..overflow);
            self.bytes.extend(bytes);
        }
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut file = File::create(path)?;
        for slice in [self.bytes.as_slices().0, self.bytes.as_slices().1] {
            file.write_all(slice)?;
        }
        file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_rx_is_exact_bounded_and_does_not_overwrite_existing_session() {
        let dir = tempfile::tempdir().unwrap();
        let options = Options {
            prefix: dir.path().join("test"),
            max_bytes: 4,
            backups: 2,
        };
        let mut recorder = Recorder::open(&options, "test").unwrap();
        recorder
            .event(Duration::ZERO, "RX", &[0, 1, 0x80, 0xff, 2, 3, 4, 5, 6])
            .unwrap();
        recorder.event(Duration::ZERO, "TX", &[0xee]).unwrap();
        recorder.flush().unwrap();
        drop(recorder);
        let mut bytes = vec![];
        for suffix in [".rx.bin.2", ".rx.bin.1", ".rx.bin"] {
            bytes.extend(std::fs::read(format!("{}{}", options.prefix.display(), suffix)).unwrap());
        }
        assert_eq!(bytes, [0, 1, 0x80, 0xff, 2, 3, 4, 5, 6]);
        assert!(Recorder::open(&options, "again").is_err());
    }
    #[test]
    fn raw_tail_keeps_binary_bytes_and_counts_eviction() {
        let mut buffer = RawBuffer::default();
        buffer.push(&vec![0x80; RawBuffer::LIMIT + 2]);
        buffer.push(&[0, 1, 0xff]);
        assert_eq!(buffer.len(), RawBuffer::LIMIT);
        assert_eq!(buffer.removed, 5);
        assert_eq!(buffer.bytes.back(), Some(&0xff));
    }
}
