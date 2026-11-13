//! Language-independent events shared by the CLI worker and native UI.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Block { index: usize, total: usize },
    Eot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    Timeout,
    Nak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum FlashEvent {
    Image {
        path: String,
        bytes: usize,
        sha256: String,
    },
    PortOpened {
        port: String,
        baud: u32,
    },
    HandshakeWaiting {
        timeout_ms: u64,
    },
    StartupRx {
        byte: u8,
        elapsed_ms: u64,
    },
    StartupSummary {
        count: u64,
        sample: Vec<u8>,
    },
    HandshakeReceived,
    Writing {
        target: Target,
        retry: usize,
        max_retries: usize,
    },
    WaitingAck {
        target: Target,
        retry: usize,
        timeout_ms: u64,
    },
    Received {
        target: Target,
        byte: u8,
    },
    Retry {
        target: Target,
        retry: usize,
        max_retries: usize,
        reason: RetryReason,
    },
    Acknowledged {
        target: Target,
    },
    Progress {
        done: usize,
        total: usize,
    },
    Completed {
        probe: bool,
    },
    Failed {
        code: String,
        detail: String,
    },
}

pub const EVENT_PREFIX: &str = "SU03T_EVENT ";
