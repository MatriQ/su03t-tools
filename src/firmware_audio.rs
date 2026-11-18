//! Observed fixed-slot MPEG2 Layer III resource adapter. Profiles authenticate
//! every byte outside audio slots/lengths and approved runtime log digits. No code is patched.
use crate::firmware_profile::{self, Profile};
use anyhow::{Result, bail, ensure};
use serde::Serialize;
use std::{collections::BTreeSet, io::Write, path::Path};

#[derive(Clone, Debug, Serialize)]
pub struct Reply {
    pub id: u32,
    pub offset: usize,
    pub length: usize,
    pub capacity: usize,
    pub mp3_bytes: usize,
    pub duration_ms: usize,
    pub directory_length_offset: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct Layout {
    pub profile: String,
    pub full_image: bool,
    pub blob_offset: usize,
    pub directory_offset: usize,
    pub replies: Vec<Reply>,
    pub codec: String,
    pub experimental_edit: bool,
}

fn word(bytes: &[u8], offset: usize) -> Result<usize> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow::anyhow!("Truncated audio directory"))?;
    Ok(u32::from_le_bytes(value.try_into()?) as usize)
}

/// Return the MP3 frame-byte count, allowing less than 64 bytes of zero padding.
/// Header checks are format validation, not an audio-content quality guarantee.
pub fn mp3_frame_bytes(bytes: &[u8]) -> Result<usize> {
    let mut offset = 0;
    while offset + 72 <= bytes.len() {
        let h = &bytes[offset..offset + 4];
        if h[0] != 0xff || h[1] != 0xf3 || h[2] != 0x28 || h[3] & 0xc0 != 0xc0 {
            break;
        }
        offset += 72;
    }
    ensure!(
        offset > 0,
        "Expected raw 16 kHz mono 16 kbps MPEG2 Layer III frames (no ID3/Xing)"
    );
    ensure!(
        bytes.len() - offset < 64 && bytes[offset..].iter().all(|b| *b == 0),
        "Invalid MP3 frame sequence or padding"
    );
    Ok(offset)
}

fn parse_profile(bytes: &[u8], profile: &Profile) -> Result<Layout> {
    let mut ids = BTreeSet::new();
    let mut replies = Vec::new();
    for index in 0..profile.count {
        let entry = profile.directory + index * 12;
        let id = word(bytes, entry)? as u32;
        let relative = word(bytes, entry + 4)?;
        let length = word(bytes, entry + 8)?;
        let next = if index + 1 == profile.count {
            profile.directory - profile.base
        } else {
            word(bytes, entry + 16)?
        };
        ensure!(ids.insert(id), "Duplicate reply ID");
        ensure!(index != 0 || relative == 0, "Invalid first audio offset");
        ensure!(
            relative < next && next <= profile.directory - profile.base,
            "Overlapping or out-of-bounds audio slots"
        );
        let capacity = next - relative;
        ensure!(
            length > 0 && length <= capacity && length.is_multiple_of(64),
            "Invalid reply length"
        );
        let offset = profile.base + relative;
        let mp3_bytes = mp3_frame_bytes(&bytes[offset..offset + length])?;
        ensure!(
            bytes[offset + length..offset + capacity]
                .iter()
                .all(|b| *b == 0),
            "Unexpected data in unused audio slot"
        );
        replies.push(Reply {
            id,
            offset,
            length,
            capacity,
            mp3_bytes,
            duration_ms: mp3_bytes / 72 * 36,
            directory_length_offset: entry + 8,
        });
    }
    Ok(Layout {
        profile: profile.name.into(),
        full_image: profile.full,
        blob_offset: profile.base,
        directory_offset: profile.directory,
        replies,
        codec: "MPEG2 Layer III / 16000 Hz / mono / 16000 bps".into(),
        experimental_edit: true,
    })
}

pub fn inspect(bytes: &[u8]) -> Result<Option<Layout>> {
    if let Some(profile) = firmware_profile::authenticate(bytes) {
        return parse_profile(bytes, profile).map(Some);
    }
    Ok(None)
}

/// Make a new buffer. Keep the original file, image size, code, recognition
/// resources, other replies and offsets unchanged. Only one slot and its length
/// field may differ; hardware playback/boot acceptance remains unverified.
pub fn replace(bytes: &[u8], reply_id: u32, mp3: &[u8]) -> Result<Vec<u8>> {
    let layout = inspect(bytes)?.ok_or_else(|| {
        anyhow::anyhow!("Audio editing is unavailable for this image fingerprint")
    })?;
    let reply = layout
        .replies
        .iter()
        .find(|r| r.id == reply_id)
        .ok_or_else(|| anyhow::anyhow!("Reply ID not found"))?;
    replace_in_slot(bytes, reply, mp3)
}
fn replace_in_slot(bytes: &[u8], reply: &Reply, mp3: &[u8]) -> Result<Vec<u8>> {
    let frame_bytes = mp3_frame_bytes(mp3)?;
    let length = frame_bytes.next_multiple_of(64);
    ensure!(
        length <= reply.capacity,
        "Recording exceeds this reply slot: {length} bytes required, {} bytes available. Trim or re-record it.",
        reply.capacity
    );
    let mut result = bytes.to_vec();
    result[reply.offset..reply.offset + reply.capacity].fill(0);
    result[reply.offset..reply.offset + frame_bytes].copy_from_slice(&mp3[..frame_bytes]);
    result[reply.directory_length_offset..reply.directory_length_offset + 4]
        .copy_from_slice(&(length as u32).to_le_bytes());
    Ok(result)
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        bail!(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Vec<u8> {
        let mut data = vec![0x11; 72];
        data[..4].copy_from_slice(&[0xff, 0xf3, 0x28, 0xc4]);
        data
    }
    #[test]
    fn shorter_replacement_preserves_other_bytes_and_can_be_reparsed() {
        let mut bytes = vec![0xa5; 512];
        bytes[64..320].fill(0);
        bytes[64..136].copy_from_slice(&frame());
        bytes[320..324].copy_from_slice(&7u32.to_le_bytes());
        bytes[324..328].copy_from_slice(&0u32.to_le_bytes());
        bytes[328..332].copy_from_slice(&256u32.to_le_bytes());
        let profile = Profile {
            name: "synthetic",
            size: 512,
            base: 64,
            directory: 320,
            count: 1,
            full: false,
            digest: "",
            runtime_log: (0, b'0'),
            engine_log: 0,
        };
        // The original 256-byte span has more than 63 tail zeros and is invalid.
        assert!(parse_profile(&bytes, &profile).is_err());
        bytes[328..332].copy_from_slice(&128u32.to_le_bytes());
        let layout = parse_profile(&bytes, &profile).unwrap();
        let patched = replace_in_slot(&bytes, &layout.replies[0], &frame()).unwrap();
        assert_eq!(patched, bytes);
        let mut two = frame();
        two.extend(frame());
        let patched = replace_in_slot(&bytes, &layout.replies[0], &two).unwrap();
        assert_eq!(
            parse_profile(&patched, &profile).unwrap().replies[0].length,
            192
        );
        assert_eq!(patched[..64], bytes[..64]);
        assert_eq!(patched[332..], bytes[332..]);
        let mut oversized = Vec::new();
        for _ in 0..4 {
            oversized.extend(frame());
        }
        assert!(replace_in_slot(&bytes, &layout.replies[0], &oversized).is_err());
        assert!(replace(&bytes, 7, &two).is_err());
    }
    #[test]
    fn rejects_metadata_wrong_codec_partial_frames_and_large_zero_tails() {
        for value in [b"ID3".to_vec(), vec![0; 4096], frame()[..71].to_vec()] {
            assert!(mp3_frame_bytes(&value).is_err());
        }
        let mut data = frame();
        data.extend([0; 56]);
        assert_eq!(mp3_frame_bytes(&data).unwrap(), 72);
        data.push(0);
        data[2] = 0x38;
        assert!(mp3_frame_bytes(&data).is_err());
    }
    #[test]
    fn exporting_never_overwrites_existing_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.bin");
        write_new(&path, b"original").unwrap();
        assert!(write_new(&path, b"replacement").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
}
