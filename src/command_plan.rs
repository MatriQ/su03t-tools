//! Authoring helpers for BIN command plans. These values never alter grammar bytes.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const MAX_COMMANDS: usize = 1000;

#[derive(Clone, Debug, Default)]
pub struct RecognitionCommand {
    pub phrases: String,
    pub action: String,
    pub reply: String,
}

impl RecognitionCommand {
    fn variants(&self) -> impl Iterator<Item = &str> {
        self.phrases
            .split(['|', '\n', '\r'])
            .map(str::trim)
            .filter(|phrase| !phrase.is_empty())
    }
}

pub struct Issue {
    pub code: &'static str,
    pub warning: bool,
}

fn action_exists(rows: &[Value], action: &str) -> bool {
    rows.iter().any(|row| {
        row["action"]
            .as_str()
            .is_some_and(|existing| existing.eq_ignore_ascii_case(action))
    })
}

fn valid_action(action: &str) -> bool {
    action.len() <= 15
        && action
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && action
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub fn recognition_command_issues(section: &Value, command: &RecognitionCommand) -> Vec<Issue> {
    let error = |code| Issue {
        code,
        warning: false,
    };
    let Some(rows) = section["cmdWord"].as_array() else {
        return vec![error("invalid_commands")];
    };
    let mut issues = Vec::new();
    if rows.len() >= MAX_COMMANDS {
        issues.push(error("command_limit"));
    }
    if command.variants().next().is_none() {
        issues.push(error("empty_phrase"));
    }
    if command
        .phrases
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r'))
    {
        issues.push(error("control_character"));
    }
    let mut phrases: BTreeSet<_> = rows
        .iter()
        .filter(|row| {
            row["isCMD"].as_bool() != Some(false)
                && row["trigger"]
                    .as_array()
                    .is_none_or(|triggers| triggers.iter().any(|t| t == "cmd"))
        })
        .filter_map(|row| row["word"].as_str())
        .flat_map(|words| words.split('|').map(str::trim))
        .collect();
    if command.variants().any(|phrase| !phrases.insert(phrase)) {
        issues.push(error("duplicate_phrase"));
    }
    let action = command.action.trim();
    if !action.is_empty() {
        if !valid_action(action) {
            issues.push(error("invalid_action"));
        }
        if action_exists(rows, action) {
            issues.push(error("duplicate_action"));
        }
    }
    for (code, warning) in [
        (
            "historical_phrase_length",
            command
                .variants()
                .any(|phrase| !(2..=10).contains(&phrase.chars().count())),
        ),
        (
            "historical_phrase_characters",
            command
                .variants()
                .any(|phrase| phrase.chars().any(|c| c.is_ascii_alphanumeric())),
        ),
    ] {
        if warning {
            issues.push(Issue { code, warning });
        }
    }
    issues
}

pub fn add_recognition_command(section: &mut Value, command: &RecognitionCommand) -> Result<usize> {
    if let Some(issue) = recognition_command_issues(section, command)
        .into_iter()
        .find(|issue| !issue.warning)
    {
        anyhow::bail!("Invalid recognition command: {}", issue.code);
    }
    let rows = section["cmdWord"]
        .as_array_mut()
        .context("Invalid command rows")?;
    let explicit_name = command.action.trim();
    let used_numbers: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| {
            let number = &row["num"];
            number.as_u64().or_else(|| number.as_str()?.parse().ok())
        })
        .collect();
    // Each row can reserve at most one number and one generated name.
    let number = (1..=2 * rows.len() as u64 + 1)
        .find(|n| {
            !used_numbers.contains(n)
                && (!explicit_name.is_empty() || !action_exists(rows, &format!("Command{n}")))
        })
        .context("Command ID space exhausted")?;
    let action = if explicit_name.is_empty() {
        format!("Command{number}")
    } else {
        explicit_name.into()
    };
    let index = rows.len();
    rows.push(json!({
        "action": action,
        "num": number.to_string(),
        "word": command.variants().collect::<Vec<_>>().join("|"),
        "reply": command.reply.trim(),
        "trigger": ["cmd"],
        "isCMD": true,
        "remark": ""
    }));
    Ok(index)
}

pub fn remove_command(section: &mut Value, index: usize) -> Result<Value> {
    let rows = section["cmdWord"]
        .as_array_mut()
        .context("Invalid command rows")?;
    ensure!(index < rows.len(), "Command row no longer exists");
    Ok(rows.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_draft_is_rejected_without_altering_recovered_rows() {
        let mut section =
            json!({"cmdWord":[{"word":"开灯|打开灯光","action":"Command2","num":"1","offset":42}]});
        let original = section.clone();
        let mut command = RecognitionCommand {
            phrases: "打开灯光".into(),
            ..Default::default()
        };
        assert!(add_recognition_command(&mut section, &command).is_err());
        assert_eq!(section, original);
        command.phrases = "关灯\n关闭灯光".into();
        let index = add_recognition_command(&mut section, &command).unwrap();
        assert_eq!(section["cmdWord"][index]["word"], "关灯|关闭灯光");
        assert_eq!(section["cmdWord"][index]["action"], "Command3");
        remove_command(&mut section, index).unwrap();
        assert_eq!(section, original);
    }
}
