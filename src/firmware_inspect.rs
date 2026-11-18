//! Bounded, read-only observations from full or update images. This parser never
//! authorizes flashing or assumes that recovered constants are editable assets.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

pub const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_JSON: usize = 16 * 1024;
const MAX_CANDIDATES: usize = 8192;
const BOOT_MARKER: &[u8] = b"unsupport xmodem-128 trans";
const LP_HEADER: &[u8] = b"<type:LP language:mandarin version:v3.1>";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    Full,
    Update,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub offset: usize,
    pub length: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct CommandEntry {
    pub span: Span,
    pub text: String,
    pub command: String,
    pub pcm: String,
    pub reply_ids: Vec<u32>,
    /// File offset of the corroborating pointer, when present.
    pub pointer_offset: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Configuration {
    pub span: Span,
    pub kind: String,
    pub value: Value,
}
#[derive(Clone, Debug, Serialize)]
pub struct GrammarResource {
    pub span: Span,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub bytes: usize,
    pub sha256: String,
    pub image_kind: ImageKind,
    pub boot_marker_present: bool,
    pub application_offset: Option<usize>,
    pub boot_sha256: Option<String>,
    pub pointer_address_bias: Option<usize>,
    pub commands: Vec<CommandEntry>,
    pub configurations: Vec<Configuration>,
    pub grammar_resources: Vec<GrammarResource>,
    pub audio: Option<crate::firmware_audio::Layout>,
    pub logging: crate::firmware_logging::Report,
    pub warnings: Vec<String>,
    pub firmware_edit_supported: bool,
    pub reply_audio_decode_supported: bool,
}

pub fn load(path: &Path) -> Result<(Vec<u8>, Report)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .context("Read firmware for inspection")?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("Read firmware for inspection")?;
    let report = inspect(&bytes)?;
    Ok((bytes, report))
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn u32_at(bytes: &[u8], offset: usize) -> Option<usize> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?) as usize)
}

pub fn inspect(bytes: &[u8]) -> Result<Report> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_BYTES,
        "Inspection requires a nonempty image no larger than 2 MiB"
    );
    let boot_marker_present = bytes[..bytes.len().min(65536)]
        .windows(BOOT_MARKER.len())
        .any(|window| window == BOOT_MARKER);
    let mut report = Report {
        schema_version: 1,
        bytes: bytes.len(),
        sha256: sha(bytes),
        image_kind: ImageKind::Unknown,
        boot_marker_present,
        application_offset: None,
        boot_sha256: None,
        pointer_address_bias: None,
        commands: Vec::new(),
        configurations: Vec::new(),
        grammar_resources: Vec::new(),
        audio: None,
        logging: crate::firmware_logging::Report::unknown(),
        warnings: vec![
            "static_observations_only".into(),
            "audio_format_unresolved".into(),
        ],
        firmware_edit_supported: false,
        reply_audio_decode_supported: false,
    };
    let mut candidates = 0;
    for offset in (0..bytes.len()).step_by(4) {
        if bytes[offset] != b'{' {
            continue;
        }
        candidates += 1;
        if candidates > MAX_CANDIDATES {
            report.warnings.push("json_scan_limit".into());
            break;
        }
        let window = &bytes[offset..bytes.len().min(offset + MAX_JSON)];
        let Some(length) = window.iter().position(|b| *b == 0) else {
            continue;
        };
        let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&window[..length]) else {
            continue;
        };
        let span = Span { offset, length };
        if object.len() == 3
            && let (Some(text), Some(command), Some(pcm)) = (
                object.get("asr").and_then(Value::as_str),
                object.get("cmd").and_then(Value::as_str),
                object.get("pcm").and_then(Value::as_str),
            )
        {
            if !text.is_empty() && !command.is_empty() && report.commands.len() < 4096 {
                report.commands.push(CommandEntry {
                    span,
                    text: text.into(),
                    command: command.into(),
                    pcm: pcm.into(),
                    reply_ids: serde_json::from_str(pcm).unwrap_or_default(),
                    pointer_offset: None,
                });
            }
        } else if report.configurations.len() < 64 {
            let kind = if object.contains_key("audio_stream") && object.contains_key("kws_lp") {
                Some("engine")
            } else if object.contains_key("hvad") && object.contains_key("lasr") {
                Some("runtime")
            } else {
                None
            };
            if let Some(kind) = kind {
                report.configurations.push(Configuration {
                    span,
                    kind: kind.into(),
                    value: Value::Object(object),
                });
            }
        }
    }
    // Corroborate constants with adjacent pointers in the observed address
    // convention. A standalone JSON-looking byte sequence is only a candidate.
    let mut mappings = Vec::new();
    for bias in [0, 65536] {
        let offsets: BTreeMap<_, _> = report
            .commands
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.span.offset + bias, index))
            .collect();
        let mut linked = BTreeMap::new();
        let mut run: Vec<(usize, usize)> = Vec::new();
        for offset in (0..bytes.len().saturating_sub(3)).step_by(4) {
            let index = u32_at(bytes, offset)
                .and_then(|address| offsets.get(&address))
                .copied();
            if let Some(index) = index {
                if run.last().is_some_and(|(_, prior)| *prior + 1 != index) {
                    run.clear();
                }
                run.push((offset, index));
                if run.len() >= 2 {
                    for &(pointer, item) in &run[run.len() - 2..] {
                        linked.insert(item, pointer);
                    }
                }
            } else {
                run.clear();
            }
        }
        mappings.push((bias, linked));
    }
    mappings.sort_by_key(|(_, linked)| std::cmp::Reverse(linked.len()));
    if mappings[0].1.len() >= 2 && mappings[0].1.len() > mappings[1].1.len() {
        let (bias, linked) = &mappings[0];
        report.pointer_address_bias = Some(*bias);
        for (&index, &pointer) in linked {
            report.commands[index].pointer_offset = Some(pointer);
        }
        report.image_kind = match (*bias, boot_marker_present, bytes.len() > 65536) {
            (0, true, true) => ImageKind::Full,
            (65536, false, _) if bytes.len() <= MAX_BYTES - 65536 => ImageKind::Update,
            _ => ImageKind::Unknown,
        };
    }
    match report.image_kind {
        ImageKind::Full => {
            report.application_offset = Some(65536);
            report.boot_sha256 = Some(sha(&bytes[..65536]));
        }
        ImageKind::Update => report.application_offset = Some(0),
        ImageKind::Unknown => report.warnings.push("image_layout_unconfirmed".into()),
    }
    if report
        .commands
        .iter()
        .any(|entry| entry.pointer_offset.is_none())
    {
        report
            .warnings
            .push("unreferenced_command_candidates".into());
    }
    if report.commands.is_empty() {
        report.warnings.push("no_command_constants".into());
    }
    // Only accept a pair when both public headers and the local length table
    // agree. Do not guess lengths from a marker or scan beyond the input.
    for first in 24..bytes.len().saturating_sub(LP_HEADER.len()) + 1 {
        if !bytes[first..].starts_with(LP_HEADER) || u32_at(bytes, first - 24) != Some(2) {
            continue;
        }
        let (Some(a), Some(b)) = (u32_at(bytes, first - 20), u32_at(bytes, first - 16)) else {
            continue;
        };
        let second = first.checked_add(a);
        let end = second.and_then(|value| value.checked_add(b));
        if a < LP_HEADER.len()
            || b < LP_HEADER.len()
            || end.is_none_or(|end| end > bytes.len())
            || second.is_none_or(|second| !bytes[second..].starts_with(LP_HEADER))
        {
            report.warnings.push("invalid_grammar_bounds".into());
            continue;
        }
        for (offset, length) in [(first, a), (second.unwrap(), b)] {
            report.grammar_resources.push(GrammarResource {
                span: Span { offset, length },
                sha256: sha(&bytes[offset..offset + length]),
            });
        }
    }
    report.warnings.push("control_actions_not_recovered".into());
    match crate::firmware_audio::inspect(bytes) {
        Ok(Some(audio)) => {
            report.warnings.retain(|w| w != "audio_format_unresolved");
            report
                .warnings
                .push("experimental_audio_edit_no_hardware_acceptance".into());
            report.reply_audio_decode_supported = true;
            report.firmware_edit_supported = true;
            report.audio = Some(audio);
        }
        Ok(None) => {}
        Err(_) => report.warnings.push("invalid_audio_directory".into()),
    }
    report.logging = crate::firmware_logging::inspect_validated(bytes, report.audio.is_some());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(full: bool) -> Vec<u8> {
        let base = if full { 65536 } else { 0 };
        let mut bytes = vec![0xff; base + 1024];
        if full {
            bytes[20..20 + BOOT_MARKER.len()].copy_from_slice(BOOT_MARKER);
        }
        for (i, text) in ["测试甲", "测试乙"].iter().enumerate() {
            let offset = base + 128 + i * 128;
            let data = format!(
                r#"{{"asr":"{text}","cmd":"command_{i}","pcm":"[{}]"}}"#,
                100 + i
            );
            bytes[offset..offset + data.len()].copy_from_slice(data.as_bytes());
            bytes[offset + data.len()] = 0;
            let pointer = ((offset - base + 65536) as u32).to_le_bytes();
            bytes[base + 700 + i * 4..base + 704 + i * 4].copy_from_slice(&pointer);
        }
        bytes
    }

    #[test]
    fn full_update_offsets_and_pointer_evidence_agree() {
        let full = fixture(true);
        let update = fixture(false);
        assert_eq!(full[65536..], update);
        let a = inspect(&full).unwrap();
        let b = inspect(&update).unwrap();
        assert_eq!(a.image_kind, ImageKind::Full);
        assert_eq!(b.image_kind, ImageKind::Update);
        assert_eq!(b.commands[1].reply_ids, [101]);
        assert_eq!(a.commands[0].span.offset, b.commands[0].span.offset + 65536);
        assert!(
            b.commands
                .iter()
                .all(|entry| entry.pointer_offset.is_some())
        );
        assert!(!b.firmware_edit_supported);
        assert!(!b.reply_audio_decode_supported);
    }

    #[test]
    fn embedded_json_and_boot_string_alone_do_not_establish_layout() {
        let mut bytes = fixture(true);
        bytes[65536 + 700..65536 + 708].fill(0);
        let report = inspect(&bytes).unwrap();
        assert_eq!(report.commands.len(), 2);
        assert_eq!(report.image_kind, ImageKind::Unknown);
        assert!(
            report
                .commands
                .iter()
                .all(|entry| entry.pointer_offset.is_none())
        );
    }

    #[test]
    fn damaged_or_unterminated_json_and_forged_lengths_are_bounded() {
        let mut bytes = fixture(false);
        bytes[128 + 2] = 0xff;
        let first: usize = 800;
        bytes[first..first + LP_HEADER.len()].copy_from_slice(LP_HEADER);
        bytes[first - 24..first - 20].copy_from_slice(&2u32.to_le_bytes());
        bytes[first - 20..first - 16].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[first - 16..first - 12].copy_from_slice(&u32::MAX.to_le_bytes());
        let report = inspect(&bytes).unwrap();
        assert_eq!(report.commands.len(), 1);
        assert!(report.grammar_resources.is_empty());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w == "invalid_grammar_bounds")
        );
        assert!(inspect(&[]).is_err());
        assert!(inspect(&vec![0; MAX_BYTES + 1]).is_err());
        for size in [1, 4, 23, 24, 39, 65536] {
            assert!(inspect(&vec![b'{'; size]).is_ok());
        }
    }
}
