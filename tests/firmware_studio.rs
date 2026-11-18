//! Optional acceptance against a locally supplied known-profile image. No device I/O.
use std::{path::PathBuf, sync::atomic::AtomicBool};
use su03t_update::{
    audio_asset::{self, Clip},
    firmware_studio::Session,
};

#[test]
#[ignore = "set SU03T_TEST_FIRMWARE to a private known-profile BIN; requires FFmpeg"]
fn edit_undo_export_and_failure_atomicity() {
    let path = PathBuf::from(
        std::env::var_os("SU03T_TEST_FIRMWARE").expect("explicit local fixture path required"),
    );
    let original = std::fs::read(&path).unwrap();
    let mut session = Session::load(&path).unwrap();
    let id = session
        .report
        .audio
        .as_ref()
        .unwrap()
        .replies
        .iter()
        .max_by_key(|r| r.capacity)
        .unwrap()
        .id;
    let baseline = session.export_bytes().unwrap().0;
    let clip = Clip {
        sample_rate: 16000,
        samples: (0..4000).map(|i| (i as f32 * 0.1728).sin() * 0.2).collect(),
    };
    let mp3 = audio_asset::encode_reply(
        &audio_asset::default_ffmpeg(),
        &clip.wav(0.0, 0.25, 1.0).unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    session.replace(id, &mp3).unwrap();
    assert_eq!(session.modified_ids(), vec![id]);
    let changed = session.export_bytes().unwrap().0;
    assert_ne!(baseline, changed);
    assert!(session.replace(id, b"invalid").is_err());
    assert_eq!(session.export_bytes().unwrap().0, changed);
    assert!(session.replace(u32::MAX, &mp3).is_err());
    session.undo(id).unwrap();
    assert!(session.modified_ids().is_empty());
    assert_eq!(session.export_bytes().unwrap().0, baseline);
    session.replace(id, &mp3).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("edited_update.bin");
    // Pending command edits must refuse before creating either output file,
    // while retaining the independently edited reply in memory.
    su03t_update::command_plan::add_recognition_command(
        &mut session.commands,
        &su03t_update::command_plan::RecognitionCommand {
            phrases: "本地合成验证指令".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let plan = session.command_plan();
    assert!(
        session
            .export(&output)
            .unwrap_err()
            .to_string()
            .contains("grammar/SDK")
    );
    assert!(!output.exists());
    assert!(!output.with_extension("bin.manifest.json").exists());
    session.restore_commands();
    assert_eq!(session.export_bytes().unwrap().0, changed);
    session.import_command_plan(plan).unwrap();
    assert!(session.commands_changed());
    session.restore_commands();
    session.export(&output).unwrap();
    assert!(session.export(&output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), changed);
    let mut reopened = Session::load(&output).unwrap();
    assert_eq!(reopened.reply(id, true).unwrap(), mp3);
    reopened.replace(id, &mp3).unwrap();
    assert!(reopened.modified_ids().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
#[ignore = "set SU03T_TEST_FIRMWARE to a private known-profile BIN; requires FFmpeg"]
fn logging_audio_combination_undo_reopen_and_corruption_refusal() {
    use su03t_update::{
        firmware_audio, firmware_inspect,
        firmware_logging::{self, Kind},
    };
    let path =
        PathBuf::from(std::env::var_os("SU03T_TEST_FIRMWARE").expect("explicit fixture required"));
    let original = std::fs::read(&path).unwrap();
    let mut session = Session::load(&path).unwrap();
    let initial = session.report.logging.get(Kind::Runtime).enabled.unwrap();
    let offset = session
        .report
        .logging
        .get(Kind::Runtime)
        .source_offset
        .unwrap();
    let skip = if session.report.audio.as_ref().unwrap().full_image {
        65536
    } else {
        0
    };
    let baseline = session.export_bytes().unwrap().0;
    assert_eq!(original[offset], if initial { b'1' } else { b'0' });
    assert!(!session.report.logging.get(Kind::Engine).editable);
    assert!(session.report.logging.get(Kind::Engine).enabled.is_none());
    assert!(session.set_logging(Kind::Engine, true).is_err());
    assert!(session.set_logging(Kind::Bootloader, false).is_err());
    session.set_logging(Kind::Runtime, !initial).unwrap();
    let (logs_only, manifest) = session.export_bytes().unwrap();
    assert!(session.logging_changed());
    assert!(session.modified_ids().is_empty());
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.logging_changes.len(), 1);
    let change = &manifest.logging_changes[0];
    assert_eq!(change.source_offset, offset);
    assert_eq!(change.update_offset, offset - skip);
    assert_eq!(change.old_enabled, initial);
    assert_eq!(change.new_enabled, !initial);
    let diffs: Vec<_> = baseline
        .iter()
        .zip(&logs_only)
        .enumerate()
        .filter_map(|(i, (a, b))| (a != b).then_some(i))
        .collect();
    assert_eq!(diffs, vec![offset - skip]);
    assert!(firmware_audio::inspect(&logs_only).unwrap().is_some());
    // Replace an audio slot after the log edit, using only synthetic audio.
    let id = session
        .report
        .audio
        .as_ref()
        .unwrap()
        .replies
        .iter()
        .max_by_key(|r| r.capacity)
        .unwrap()
        .id;
    let clip = Clip {
        sample_rate: 16000,
        samples: (0..4000).map(|i| (i as f32 * 0.1728).sin() * 0.2).collect(),
    };
    let mp3 = audio_asset::encode_reply(
        &audio_asset::default_ffmpeg(),
        &clip.wav(0.0, 0.25, 1.0).unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    session.replace(id, &mp3).unwrap();
    assert_eq!(session.modified_ids(), vec![id]);
    assert_eq!(
        session.report.logging.get(Kind::Runtime).enabled,
        Some(!initial)
    );
    let (combined, manifest) = session.export_bytes().unwrap();
    assert_eq!(manifest.logging_changes.len(), 1);
    assert_eq!(manifest.modified_reply_ids, vec![id]);
    // Undo either edit without discarding the other.
    session.undo_logging().unwrap();
    assert!(!session.logging_changed());
    assert_eq!(session.modified_ids(), vec![id]);
    session.set_logging(Kind::Runtime, !initial).unwrap();
    assert_eq!(session.export_bytes().unwrap().0, combined);
    session.undo(id).unwrap();
    assert!(session.modified_ids().is_empty());
    assert_eq!(session.export_bytes().unwrap().0, logs_only);
    session.undo_logging().unwrap();
    assert_eq!(session.export_bytes().unwrap().0, baseline);
    // Reverse order, then export and reopen both types together.
    session.replace(id, &mp3).unwrap();
    session.set_logging(Kind::Runtime, !initial).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("combined_update.bin");
    session.export(&out).unwrap();
    let mut reopened = Session::load(&out).unwrap();
    assert_eq!(
        reopened.report.logging.get(Kind::Runtime).enabled,
        Some(!initial)
    );
    assert_eq!(reopened.reply(id, true).unwrap(), mp3);
    reopened.set_logging(Kind::Runtime, initial).unwrap();
    reopened.replace(id, &mp3).unwrap();
    reopened.undo_logging().unwrap();
    assert_eq!(
        reopened.report.logging.get(Kind::Runtime).enabled,
        Some(!initial)
    );
    assert_eq!(reopened.reply(id, true).unwrap(), mp3);
    assert!(reopened.export_bytes().is_ok());
    // Invalid values, renamed keys, code/model changes and engine changes stay inspect-only.
    for corrupt_offset in [
        offset,
        offset - 4,
        0x10100,
        session.report.audio.as_ref().unwrap().directory_offset
            + 12 * session.report.audio.as_ref().unwrap().replies.len()
            + 100,
        session
            .report
            .logging
            .get(Kind::Engine)
            .source_offset
            .unwrap(),
    ] {
        let mut corrupt = original.clone();
        corrupt[corrupt_offset] ^= 4;
        assert!(firmware_logging::replace(&corrupt, Kind::Runtime, true).is_err());
        assert!(firmware_audio::replace(&corrupt, id, &mp3).is_err());
        assert!(
            !firmware_inspect::inspect(&corrupt)
                .unwrap()
                .logging
                .get(Kind::Runtime)
                .editable
        );
    }
    assert_eq!(std::fs::read(path).unwrap(), original);
}
