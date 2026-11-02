//! User-owned Scribe memory shared by desktop applications. Document snapshots stay outside it.
use serde::{Deserialize, Serialize};

pub const VERSION_MEMORY: u32 = 3;
pub const MAX_MEMORY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub revision: u64,
    pub preferences: String,
    pub summary: String,
    pub turns: Vec<MemoryTurn>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryTurn {
    pub app: String,
    pub user: String,
    pub assistant: String,
}

impl MemoryTurn {
    pub fn valid(&self) -> bool {
        !self.app.is_empty()
            && self.app.len() <= 64
            && self.app.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && !self.user.trim().is_empty()
            && self.user.len() <= 32 * 1024
            && !self.assistant.trim().is_empty()
            && self.assistant.len() <= 128 * 1024
    }
}

impl Memory {
    pub fn valid(&self) -> bool {
        self.revision < i64::MAX as u64
            && self.preferences.len() <= 8192
            && self.summary.len() <= 16 * 1024
            && self.turns.len() <= 32
            && self.turns.iter().all(MemoryTurn::valid)
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= MAX_MEMORY_BYTES)
    }
}
