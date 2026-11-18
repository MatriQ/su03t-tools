//! Authenticated runtime logger configuration. Never claims total UART silence.
use crate::{firmware_audio, firmware_profile};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Runtime,
    Engine,
    Bootloader,
}
#[derive(Clone, Debug, Serialize)]
pub struct Setting {
    pub kind: Kind,
    /// Requested runtime configuration, not a hardware observation.
    pub enabled: Option<bool>,
    pub editable: bool,
    pub raw_value: Option<u8>,
    pub source_offset: Option<usize>,
    pub config_path: &'static str,
    pub evidence: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub settings: Vec<Setting>,
    pub arpt_enabled: Option<bool>,
    pub all_output_silence_verified: bool,
}
impl Report {
    pub fn unknown() -> Self {
        Self {
            settings: [Kind::Runtime, Kind::Engine, Kind::Bootloader]
                .into_iter()
                .map(|kind| Setting {
                    kind,
                    enabled: None,
                    editable: false,
                    raw_value: None,
                    source_offset: None,
                    config_path: match kind {
                        Kind::Runtime => "runtime.log.enable",
                        Kind::Engine => "engine.log",
                        Kind::Bootloader => "unknown",
                    },
                    evidence: "unverified_image_or_layout",
                })
                .collect(),
            arpt_enabled: None,
            all_output_silence_verified: false,
        }
    }
    pub fn get(&self, kind: Kind) -> &Setting {
        self.settings
            .iter()
            .find(|setting| setting.kind == kind)
            .expect("all logger kinds present")
    }
}
pub fn inspect(bytes: &[u8]) -> Report {
    inspect_validated(
        bytes,
        firmware_audio::inspect(bytes).is_ok_and(|layout| layout.is_some()),
    )
}
pub(crate) fn inspect_validated(bytes: &[u8], audio_valid: bool) -> Report {
    let mut report = Report::unknown();
    if !audio_valid {
        return report;
    }
    let Some(profile) = firmware_profile::authenticate(bytes) else {
        return report;
    };
    let (offset, _) = profile.runtime_log;
    report.settings[0] = Setting {
        kind: Kind::Runtime,
        enabled: Some(bytes[offset] == b'1'),
        editable: true,
        raw_value: Some(bytes[offset] - b'0'),
        source_offset: Some(offset),
        config_path: "runtime.log.enable",
        evidence: "authenticated_profile_and_runtime_initialization_gate",
    };
    // Actual OsalLogSetLevel is a stub in all admitted samples. Its declared
    // enum 6=NONE is not proof that the final firmware applies that level.
    report.settings[1] = Setting {
        kind: Kind::Engine,
        enabled: None,
        editable: false,
        raw_value: Some(bytes[profile.engine_log] - b'0'),
        source_offset: Some(profile.engine_log),
        config_path: "engine.log",
        evidence: "level_setter_stub_no_independent_control",
    };
    // Authenticated runtime JSON has arpt_enable=0. Leave it immutable.
    report.arpt_enabled = Some(false);
    report
}
pub fn replace(bytes: &[u8], kind: Kind, enabled: bool) -> Result<Vec<u8>> {
    let report = inspect(bytes);
    let setting = report.get(kind);
    ensure!(
        setting.editable,
        "This logger is unknown or has no verified control for this image"
    );
    let mut next = bytes.to_vec();
    next[setting.source_offset.unwrap()] = if enabled { b'1' } else { b'0' };
    ensure!(
        firmware_profile::authenticate(&next).is_some(),
        "Log edit failed image authentication"
    );
    Ok(next)
}
#[derive(Debug, Serialize)]
pub struct Change {
    pub kind: Kind,
    pub config_path: &'static str,
    pub old_enabled: bool,
    pub new_enabled: bool,
    pub old_value: u8,
    pub new_value: u8,
    pub source_offset: usize,
    pub update_offset: usize,
    pub length: usize,
}
pub(crate) fn changes(original: &[u8], current: &[u8], skip: usize) -> Vec<Change> {
    let old = inspect(original);
    let new = inspect(current);
    old.settings
        .iter()
        .filter_map(|before| {
            let after = new.get(before.kind);
            if !before.editable || !after.editable || before.enabled == after.enabled {
                return None;
            }
            let offset = after.source_offset?;
            Some(Change {
                kind: after.kind,
                config_path: after.config_path,
                old_enabled: before.enabled?,
                new_enabled: after.enabled?,
                old_value: before.raw_value?,
                new_value: after.raw_value?,
                source_offset: offset,
                update_offset: offset.checked_sub(skip)?,
                length: 1,
            })
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_looking_unknown_image_never_grants_logging_writes() {
        let bytes = br#"{"log":{"enable":0,"set_level":2},"audio_stream":{},"kws_lp":{}}"#;
        let report = inspect(bytes);
        assert!(
            report
                .settings
                .iter()
                .all(|s| !s.editable && s.enabled.is_none())
        );
        for kind in [Kind::Runtime, Kind::Engine, Kind::Bootloader] {
            assert!(replace(bytes, kind, true).is_err());
        }
        assert!(!report.all_output_silence_verified);
    }
}
