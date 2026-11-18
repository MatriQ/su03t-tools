//! In-memory edits and explicit, non-overwriting experimental update exports.
use crate::{
    firmware_audio,
    firmware_inspect::{self, Report},
    firmware_logging,
};
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct Session {
    pub source: PathBuf,
    pub report: Report,
    original: Vec<u8>,
    current: Vec<u8>,
    pub commands: Value,
    original_commands: Value,
}
#[derive(Debug, Serialize)]
pub struct ChangedRange {
    pub source_offset: usize,
    pub update_offset: usize,
    pub length: usize,
}
#[derive(Debug, Serialize)]
pub struct ExportManifest {
    pub schema_version: u32,
    pub experimental: bool,
    pub device_verified: bool,
    pub source_sha256: String,
    pub edited_source_sha256: String,
    pub update_sha256: String,
    pub profile: String,
    pub removed_boot_bytes: usize,
    pub modified_reply_ids: Vec<u32>,
    pub changed_ranges: Vec<ChangedRange>,
    pub logging_changes: Vec<firmware_logging::Change>,
    pub logging: firmware_logging::Report,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Session {
    #[cfg(debug_assertions)]
    pub fn preview(report: Report) -> Self {
        Self {
            source: "SIMULATED_update.bin".into(),
            commands: recovered_commands(&report),
            original_commands: recovered_commands(&report),
            report,
            original: vec![0; 1235304],
            current: vec![0; 1235304],
        }
    }
    pub fn load(path: &Path) -> Result<Self> {
        let (bytes, report) = firmware_inspect::load(path)?;
        Ok(Self {
            source: path.into(),
            commands: recovered_commands(&report),
            original_commands: recovered_commands(&report),
            report,
            current: bytes.clone(),
            original: bytes,
        })
    }
    pub fn reply(&self, id: u32, original: bool) -> Result<Vec<u8>> {
        let bytes = if original {
            &self.original
        } else {
            &self.current
        };
        let layout = firmware_audio::inspect(bytes)?
            .ok_or_else(|| anyhow::anyhow!("Unsupported audio profile"))?;
        let reply = layout
            .replies
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow::anyhow!("Reply ID not found"))?;
        Ok(bytes[reply.offset..reply.offset + reply.mp3_bytes].to_vec())
    }
    pub fn replace(&mut self, id: u32, mp3: &[u8]) -> Result<()> {
        let next = firmware_audio::replace(&self.current, id, mp3)?;
        let report = firmware_inspect::inspect(&next)?;
        self.current = next;
        self.report = report;
        Ok(())
    }
    pub fn set_logging(&mut self, kind: firmware_logging::Kind, enabled: bool) -> Result<()> {
        let next = firmware_logging::replace(&self.current, kind, enabled)?;
        let report = firmware_inspect::inspect(&next)?;
        self.current = next;
        self.report = report;
        Ok(())
    }
    pub fn logging_changed(&self) -> bool {
        self.report
            .logging
            .settings
            .iter()
            .filter_map(|setting| setting.source_offset.filter(|_| setting.editable))
            .any(|offset| self.original[offset] != self.current[offset])
    }
    pub fn undo_logging(&mut self) -> Result<()> {
        let mut next = self.current.clone();
        for setting in &self.report.logging.settings {
            if setting.editable
                && let Some(offset) = setting.source_offset
            {
                next[offset] = self.original[offset];
            }
        }
        let report = firmware_inspect::inspect(&next)?;
        self.current = next;
        self.report = report;
        Ok(())
    }
    pub fn undo(&mut self, id: u32) -> Result<()> {
        let original = self.reply(id, true)?;
        self.replace(id, &original)
    }
    pub fn modified_ids(&self) -> Vec<u32> {
        self.report
            .audio
            .as_ref()
            .map(|a| {
                a.replies
                    .iter()
                    .filter(|r| {
                        let slot = r.offset..r.offset + r.capacity;
                        let length = r.directory_length_offset..r.directory_length_offset + 4;
                        self.current[slot.clone()] != self.original[slot]
                            || self.current[length.clone()] != self.original[length]
                    })
                    .map(|r| r.id)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn commands_changed(&self) -> bool {
        self.commands != self.original_commands
    }
    pub fn restore_commands(&mut self) {
        self.commands = self.original_commands.clone();
    }
    /// An edit request, not a complete platform project or generated firmware.
    pub fn command_plan(&self) -> Value {
        json!({"format":"su03t-command-plan","schema_version":1,
            "source_sha256":hash(&self.original),"requires_grammar_rebuild":true,
            "firmware_generated":false,"original_commands":self.original_commands,"commands":self.commands})
    }
    pub fn import_command_plan(&mut self, value: Value) -> Result<()> {
        ensure!(
            value["format"] == "su03t-command-plan" && value["schema_version"] == 1,
            "Unsupported command plan"
        );
        ensure!(
            value["source_sha256"] == hash(&self.original),
            "Command plan belongs to another source BIN"
        );
        ensure!(
            value["original_commands"] == self.original_commands,
            "Recovered command baseline mismatch"
        );
        ensure!(
            value["commands"]["cmdWord"]
                .as_array()
                .is_some_and(|rows| rows.len() <= 1000 && rows.iter().all(Value::is_object)),
            "Invalid command rows"
        );
        self.commands = value["commands"].clone();
        Ok(())
    }
    pub fn export_bytes(&self) -> Result<(Vec<u8>, ExportManifest)> {
        ensure!(
            !self.commands_changed(),
            "Command edits require a compatible grammar/SDK rebuild. Save the command plan or restore commands before audio/log BIN export."
        );
        let layout = firmware_audio::inspect(&self.current)?
            .ok_or_else(|| anyhow::anyhow!("Unknown images are inspect-only"))?;
        let skip = if layout.full_image { 0x10000 } else { 0 };
        let update = self.current[skip..].to_vec();
        let verified = firmware_audio::inspect(&update)?
            .ok_or_else(|| anyhow::anyhow!("Update fingerprint verification failed"))?;
        ensure!(
            !verified.full_image && verified.profile == layout.profile,
            "Update layout mismatch"
        );
        let mut changed_ranges = Vec::new();
        let mut offset = skip;
        while offset < self.current.len() {
            if self.current[offset] == self.original[offset] {
                offset += 1;
                continue;
            }
            let start = offset;
            while offset < self.current.len() && self.current[offset] != self.original[offset] {
                offset += 1;
            }
            changed_ranges.push(ChangedRange {
                source_offset: start,
                update_offset: start - skip,
                length: offset - start,
            });
        }
        let manifest = ExportManifest {
            schema_version: 2,
            experimental: true,
            device_verified: false,
            source_sha256: hash(&self.original),
            edited_source_sha256: hash(&self.current),
            update_sha256: hash(&update),
            profile: layout.profile,
            removed_boot_bytes: skip,
            modified_reply_ids: self.modified_ids(),
            changed_ranges,
            logging_changes: firmware_logging::changes(&self.original, &self.current, skip),
            logging: self.report.logging.clone(),
        };
        Ok((update, manifest))
    }
    /// Writes BIN and adjacent BIN.manifest.json; rolls back the new BIN if the
    /// sidecar cannot be created. Neither path may already exist.
    pub fn export(&self, path: &Path) -> Result<ExportManifest> {
        let (bytes, manifest) = self.export_bytes()?;
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".manifest.json");
        let json = serde_json::to_vec_pretty(&manifest)?;
        firmware_audio::write_new(path, &bytes)?;
        if let Err(error) = firmware_audio::write_new(Path::new(&sidecar), &json) {
            let _ = std::fs::remove_file(path);
            return Err(error.context("Export rolled back: could not create manifest"));
        }
        Ok(manifest)
    }
}

fn recovered_commands(report: &Report) -> Value {
    json!({"cmdWord":report.commands.iter().map(|row|json!({
        "action":row.command,"word":row.text,"reply":row.pcm,
        "source_offset":row.span.offset,"isCMD":true,"trigger":["cmd"]
    })).collect::<Vec<_>>()})
}
#[cfg(test)]
mod command_plan_tests {
    use super::*;
    #[test]
    fn pending_commands_block_bin_export_and_plans_bind_to_source() {
        let report = firmware_inspect::inspect(b"synthetic").unwrap();
        let mut session = Session {
            source: "test.bin".into(),
            report,
            original: vec![1],
            current: vec![1],
            commands: json!({"cmdWord":[]}),
            original_commands: json!({"cmdWord":[]}),
        };
        crate::command_plan::add_recognition_command(
            &mut session.commands,
            &crate::command_plan::RecognitionCommand {
                phrases: "测试指令".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            session
                .export_bytes()
                .unwrap_err()
                .to_string()
                .contains("grammar/SDK")
        );
        let plan = session.command_plan();
        session.restore_commands();
        assert!(!session.commands_changed());
        session.import_command_plan(plan.clone()).unwrap();
        assert!(session.commands_changed());
        let mut wrong = plan;
        wrong["source_sha256"] = json!("wrong");
        assert!(session.import_command_plan(wrong).is_err());
        assert_eq!(session.current, vec![1]);
    }
}
