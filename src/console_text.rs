//! Bounded, device-free terminal log presentation. Raw capture stays independent.
use std::{collections::VecDeque, time::Duration};
const MAX_LINE: usize = 8192;
const MAX_ESCAPE: usize = 512;
const MAX_LINES: usize = 2000;
const MAX_COST: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Rx,
    Tx,
    System,
}
impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rx => "RX",
            Self::Tx => "TX",
            Self::System => "SYS",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Line {
    pub at: Duration,
    pub origin: Origin,
    pub text: String,
    /// A forced presentation break for an oversized device line.
    pub continued: bool,
}
#[derive(Default)]
pub struct Update {
    pub completed: usize,
    pub evicted: Vec<Line>,
}
#[derive(Default)]
pub struct TextLog {
    pub lines: VecDeque<Line>,
    pub removed: u64,
    cost: usize,
    pending: Vec<u8>,
    at: Option<Duration>,
    escape: Vec<u8>,
    skip_lf: bool,
}
impl TextLog {
    fn append(&mut self, line: Line, update: &mut Update) {
        self.cost += line.text.len() + 96;
        self.lines.push_back(line);
        update.completed += 1;
        while self.lines.len() > MAX_LINES || self.cost > MAX_COST {
            let old = self.lines.pop_front().unwrap();
            self.cost -= old.text.len() + 96;
            self.removed += 1;
            update.evicted.push(old);
        }
    }
    pub fn notice(&mut self, at: Duration, origin: Origin, text: String) -> Update {
        let mut update = Update::default();
        self.append(
            Line {
                at,
                origin,
                text,
                continued: false,
            },
            &mut update,
        );
        update
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    fn seal(&mut self, continued: bool, update: &mut Update) {
        let cut = if continued {
            complete_prefix(&self.pending)
        } else {
            self.pending.len()
        };
        if cut > 0 {
            let text = visible_utf8(&self.pending[..cut], true);
            self.pending.drain(..cut);
            self.append(
                Line {
                    at: self.at.unwrap_or_default(),
                    origin: Origin::Rx,
                    text,
                    continued,
                },
                update,
            );
        }
        if self.pending.is_empty() {
            self.at = None;
        }
    }
    /// Split CRLF/LF/CR independently of serial read boundaries; strip bounded
    /// CSI/OSC sequences without executing terminal commands or URLs.
    pub fn push(&mut self, at: Duration, bytes: &[u8]) -> Update {
        let mut update = Update::default();
        for &byte in bytes {
            if byte == b'\n' && self.skip_lf {
                self.skip_lf = false;
                continue;
            }
            self.skip_lf = false;
            self.at.get_or_insert(at);
            if matches!(byte, b'\r' | b'\n') {
                self.pending.append(&mut self.escape);
                self.seal(false, &mut update);
                self.skip_lf = byte == b'\r';
            } else if !self.escape.is_empty() {
                self.escape.push(byte);
                match self.escape[1] {
                    b'[' if self.escape.len() > 2 && (0x40..=0x7e).contains(&byte) => {
                        self.escape.clear();
                    }
                    b'[' if self.escape.len() > 2 && !(0x20..=0x3f).contains(&byte) => {
                        self.pending.append(&mut self.escape);
                    }
                    b']' if byte == 7 || self.escape.ends_with(b"\x1b\\") => {
                        self.escape.clear();
                    }
                    b'[' | b']' => {}
                    _ => self.pending.append(&mut self.escape),
                }
                if self.escape.len() >= MAX_ESCAPE {
                    self.pending.append(&mut self.escape);
                }
            } else if byte == 27 {
                self.escape.push(byte);
            } else {
                self.pending.push(byte);
            }
            if self.pending.len() >= MAX_LINE {
                self.seal(true, &mut update);
            }
        }
        update
    }
    pub fn pending_line(&self) -> Option<Line> {
        let text = visible_utf8(&self.pending, false);
        (!text.is_empty()).then(|| Line {
            at: self.at.unwrap_or_default(),
            origin: Origin::Rx,
            text,
            continued: false,
        })
    }
    pub fn finish(&mut self) -> Update {
        let mut update = Update::default();
        self.pending.append(&mut self.escape);
        self.seal(false, &mut update);
        self.skip_lf = false;
        update
    }
}
fn complete_prefix(bytes: &[u8]) -> usize {
    // Only an incomplete trailing character must stay with the continuation.
    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(_) => return bytes.len(),
            Err(e) => {
                offset += e.valid_up_to();
                match e.error_len() {
                    Some(n) => offset += n,
                    None => return offset,
                }
            }
        }
    }
    bytes.len()
}
fn visible_utf8(bytes: &[u8], final_line: bool) -> String {
    let mut out = String::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (text, bad) = match std::str::from_utf8(&bytes[offset..]) {
            Ok(text) => (text, None),
            Err(e) => (
                std::str::from_utf8(&bytes[offset..offset + e.valid_up_to()]).unwrap(),
                Some(e),
            ),
        };
        for ch in text.chars() {
            match ch {
                '\t' => out.push_str("    "),
                ch if ch.is_control() => out.push_str(&format!("\\x{:02X}", ch as u32)),
                ch => out.push(ch),
            }
        }
        offset += text.len();
        if let Some(error) = bad {
            let size = match error.error_len() {
                Some(n) => n,
                None if final_line => bytes.len() - offset,
                None => break,
            };
            for byte in &bytes[offset..offset + size] {
                out.push_str(&format!("\\x{byte:02X}"));
            }
            offset += size;
        } else {
            break;
        }
    }
    out
}
/// Display-only interest heuristic. Never removes bytes or implies a decoded command.
pub fn is_issue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "[e]",
        "[w]",
        "error",
        "failed",
        "panic",
        "fault",
        "exception",
        "错误",
        "失败",
        "异常",
    ]
    .iter()
    .any(|key| lower.contains(key))
}
pub fn is_command_or_issue(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    is_issue(text)
        || [
            "command", "cmd", "asr", "recogn", "result", "nlu", "kwslp", "wakeup", "识别", "命令",
        ]
        .iter()
        .any(|key| lower.contains(key))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_chunks_preserve_chinese_commands_strip_split_ansi_and_coalesce_crlf() {
        let mut log = TextLog::default();
        let input = "\x1b[42;33m[T] command=打开灯光\x1b[0m\r\n[E] ERROR-127\n";
        for (index, byte) in input.as_bytes().iter().enumerate() {
            log.push(Duration::from_millis(index as u64), &[*byte]);
        }
        assert_eq!(log.lines.len(), 2);
        assert_eq!(log.lines[0].text, "[T] command=打开灯光");
        assert_eq!(log.lines[0].at, Duration::ZERO);
        assert_eq!(log.lines[1].text, "[E] ERROR-127");
        assert!(is_command_or_issue(&log.lines[0].text));
        assert!(is_issue(&log.lines[1].text));
        assert!(!is_command_or_issue("[T] send watchdog event[14]"));
    }
    #[test]
    fn live_partial_utf8_invalid_bytes_and_disconnect_tail_remain_readable() {
        let mut log = TextLog::default();
        log.push(Duration::ZERO, b"command=\xe6\x89");
        assert_eq!(log.pending_line().unwrap().text, "command=");
        log.push(Duration::from_secs(1), b"\x93\xff");
        assert_eq!(log.pending_line().unwrap().text, "command=打\\xFF");
        assert!(log.lines.is_empty());
        log.push(Duration::ZERO, b"\xdf");
        log.finish();
        assert_eq!(log.lines[0].text, "command=打\\xFF\\xDF");
    }
    #[test]
    fn osc_and_malformed_escapes_are_bounded_and_do_not_consume_following_lines() {
        let mut log = TextLog::default();
        for part in [
            b"\x1b]8;;https://example.invalid".as_slice(),
            b"\x1b\\cmd",
            b"\x1b]8;;\x07\r",
            b"\nnext\rbroken\x1b[31\nlast\n",
        ] {
            log.push(Duration::ZERO, part);
        }
        assert_eq!(log.lines[0].text, "cmd");
        assert_eq!(log.lines[1].text, "next");
        assert_eq!(log.lines[2].text, "broken\\x1B[31");
        assert_eq!(log.lines[3].text, "last");
        log.push(
            Duration::ZERO,
            &[
                b"\x1b]".as_slice(),
                &vec![b'x'; MAX_ESCAPE * 2],
                b"\ncommand=ok\n",
            ]
            .concat(),
        );
        assert_eq!(log.lines.back().unwrap().text, "command=ok");
        assert!(log.escape.is_empty());
    }
    #[test]
    fn long_lines_and_history_are_bounded_without_utf8_fragment_corruption() {
        let mut log = TextLog::default();
        let text = "命".repeat(4000);
        log.push(Duration::ZERO, text.as_bytes());
        log.finish();
        assert_eq!(log.lines.len(), 2);
        assert!(log.lines[0].continued);
        assert_eq!(
            log.lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<String>(),
            text
        );
        for _ in 0..MAX_LINES + 10 {
            log.push(Duration::ZERO, b"ok\n");
        }
        assert!(log.lines.len() <= MAX_LINES);
        assert!(log.cost <= MAX_COST);
        assert!(log.removed > 0);
        log.push(Duration::ZERO, b"stale");
        log.clear();
        log.push(Duration::ZERO, b"fresh\n");
        assert_eq!(log.lines[0].text, "fresh");
        assert_eq!(log.removed, 0);
    }
}
