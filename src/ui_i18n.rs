//! UI-only localization. Formatting values (paths, checksums, bytes) stay verbatim.
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, cell::Cell, path::PathBuf};
include!("ui_catalog.rs");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Language {
    #[default]
    System,
    Zh,
    En,
}
thread_local! { static ENGLISH: Cell<bool> = const { Cell::new(false) }; }
pub(super) fn english() -> bool {
    ENGLISH.get()
}
pub(super) fn set(language: Language, system_english: bool) {
    ENGLISH.set(match language {
        Language::System => system_english,
        Language::Zh => false,
        Language::En => true,
    });
}
pub(super) fn system_english() -> bool {
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        && out.status.success()
    {
        let value = String::from_utf8_lossy(&out.stdout);
        if let Some(first) = value
            .split(['\n', '(', ')', '"', ' ', ','])
            .find(|v| v.starts_with("zh") || v.starts_with("en"))
        {
            return !first.starts_with("zh");
        }
    }
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "en".into());
    !locale.to_lowercase().starts_with("zh")
}
fn preference_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        Some(
            PathBuf::from(std::env::var_os("HOME")?)
                .join("Library/Application Support/SU03T Tools/language.json"),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("su03t-tools/language.json"))
    }
}
pub(super) fn load() -> Language {
    preference_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub(super) fn save(language: Language) -> std::io::Result<()> {
    if let Some(path) = preference_path() {
        std::fs::create_dir_all(path.parent().expect("preferences parent"))?;
        let tmp = path.with_extension(format!("{}.json.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec(&language)?)?;
        std::fs::rename(tmp, path)?;
    }
    Ok(())
}
pub(super) fn choose<'a>(zh: &'a str, en: &'a str) -> &'a str {
    if english() { en } else { zh }
}
pub(super) fn t(zh: &'static str) -> &'static str {
    if !english() {
        return zh;
    }
    CATALOG
        .iter()
        .find(|(a, _)| *a == zh)
        .map(|(_, b)| *b)
        .unwrap_or(zh)
}

// Compatibility for legacy UI system records: match a complete catalog template,
// preserving the formatted values. This is never applied to device RX/TX text.
fn parts(template: &str) -> (Vec<&str>, Vec<&str>) {
    let mut literals = Vec::new();
    let mut keys = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open + 1..].find('}') else {
            break;
        };
        literals.push(&rest[..open]);
        keys.push(
            rest[open + 1..open + 1 + close]
                .split(':')
                .next()
                .unwrap_or(""),
        );
        rest = &rest[open + close + 2..];
    }
    literals.push(rest);
    (literals, keys)
}
fn substitute(source: &str, destination: &str, value: &str, depth: usize) -> Option<String> {
    let (literals, keys) = parts(source);
    if keys.is_empty() {
        return None;
    }
    let mut rest = value.strip_prefix(literals[0])?;
    let mut values = Vec::new();
    for literal in &literals[1..] {
        let n = if literal.is_empty() {
            rest.len()
        } else {
            rest.find(literal)?
        };
        values.push(&rest[..n]);
        rest = &rest[n..];
        rest = rest.strip_prefix(literal)?;
    }
    if !rest.is_empty() {
        return None;
    }
    let (out_literals, out_keys) = parts(destination);
    let mut out = out_literals[0].to_owned();
    let mut positional = 0;
    for (i, key) in out_keys.iter().enumerate() {
        let index = if key.is_empty() {
            let n = positional;
            positional += 1;
            n
        } else {
            keys.iter().position(|k| k == key)?
        };
        let value = *values.get(index)?;
        out.push_str(&translate(value, depth + 1));
        out.push_str(out_literals[i + 1]);
    }
    Some(out)
}
fn translate(value: &str, depth: usize) -> Cow<'_, str> {
    if value.starts_with(['/', '\\']) {
        return Cow::Borrowed(value);
    }
    if depth > 3 {
        return Cow::Borrowed(value);
    }
    for &(zh, en) in CATALOG {
        let (a, b) = if english() { (zh, en) } else { (en, zh) };
        if value == a {
            return Cow::Borrowed(b);
        }
    }
    for &(zh, en) in CATALOG {
        let (a, b) = if english() { (zh, en) } else { (en, zh) };
        if let Some(result) = substitute(a, b, value, depth) {
            return Cow::Owned(result);
        }
    }
    if value.contains(": ") {
        return Cow::Owned(
            value
                .split(": ")
                .map(|part| translate(part, depth + 1))
                .collect::<Vec<_>>()
                .join(": "),
        );
    }
    if value.contains(" | ") {
        return Cow::Owned(
            value
                .split(" | ")
                .map(|part| translate(part, depth + 1))
                .collect::<Vec<_>>()
                .join(" | "),
        );
    }
    if value.contains(" / ") && !value.starts_with('/') {
        return Cow::Owned(
            value
                .split(" / ")
                .map(|part| translate(part, depth + 1))
                .collect::<Vec<_>>()
                .join(" / "),
        );
    }
    Cow::Borrowed(value)
}
pub(super) fn display(value: &str) -> Cow<'_, str> {
    translate(value, 0)
}

macro_rules! ui_format {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::gui::i18n::english() { format!($en $(, $arg)*) } else { format!($zh $(, $arg)*) }
    };
}
pub(super) use ui_format;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translated_system_records_preserve_paths_and_can_switch_back() {
        set(Language::En, false);
        assert_eq!(
            display("日志已保存：/tmp/中文.log"),
            "Log saved: /tmp/中文.log"
        );
        assert_eq!(
            display("原始接收缓冲已保存：/tmp/a.bin（256字节）"),
            "Raw RX buffer saved: /tmp/a.bin (256 bytes)"
        );
        set(Language::Zh, false);
        assert_eq!(
            display("Log saved: /tmp/中文.log"),
            "日志已保存：/tmp/中文.log"
        );
    }
}
