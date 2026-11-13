//! Read once, hash those exact bytes, and transmit without rewriting the image.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024 - 65536;
pub struct Image {
    pub path: PathBuf,
    pub sha256: String,
    bytes: Vec<u8>,
}
impl Image {
    pub fn load(path: &Path, allow_other_filename: bool) -> Result<Self> {
        ensure!(
            allow_other_filename
                || path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_ascii_lowercase().ends_with("_update.bin")),
            "请选择 *_update.bin 串口升级文件；完整 release.bin 不能用于此路径"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .context("读取固件")?
            .take((MAX_IMAGE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .context("读取固件")?;
        Self::from_bytes(path.to_path_buf(), bytes)
    }
    fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
            "固件必须非空且不超过2MiB减64KiB启动区"
        );
        let boot = &bytes[..bytes.len().min(65536)];
        ensure!(
            !boot
                .windows(b"unsupport xmodem-128 trans".len())
                .any(|w| w == b"unsupport xmodem-128 trans"),
            "检测到官方启动区字符串：疑似完整固件，请使用对应_update.bin"
        );
        Ok(Self {
            path,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn verify_expected(&self, expected: Option<&str>) -> Result<()> {
        if let Some(expected) = expected {
            ensure!(
                expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
                "SHA-256 必须为64位十六进制"
            );
            ensure!(
                self.sha256.eq_ignore_ascii_case(expected),
                "固件 SHA-256 已变化，尚未打开串口。请重新选择并确认文件"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hashes_exact_immutable_bytes_and_rejects_changed_image() {
        let image = Image::from_bytes("test_update.bin".into(), b"abc".to_vec()).unwrap();
        assert_eq!(
            image.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        image.verify_expected(Some(&image.sha256)).unwrap();
        assert!(image.verify_expected(Some(&"0".repeat(64))).is_err());
        assert!(image.verify_expected(Some("invalid")).is_err());
        assert_eq!(image.bytes(), b"abc");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changed_update.bin");
        std::fs::write(&path, b"abc").unwrap();
        let frozen = Image::load(&path, false).unwrap();
        std::fs::write(&path, b"changed on disk").unwrap();
        assert_eq!(frozen.bytes(), b"abc");
        assert!(
            Image::load(&path, false)
                .unwrap()
                .verify_expected(Some(&frozen.sha256))
                .is_err()
        );
    }
    #[test]
    fn rejects_empty_oversized_and_known_boot_image() {
        for bytes in [
            vec![],
            vec![0; MAX_IMAGE_BYTES + 1],
            b"unsupport xmodem-128 trans".to_vec(),
        ] {
            assert!(Image::from_bytes("test_update.bin".into(), bytes).is_err());
        }
    }
}
