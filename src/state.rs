use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct State {
    pub dirs: HashMap<String, String>,
    pub files: HashMap<String, FileEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub page_id: String,
    pub hash: String,
}

pub fn load(path: &Path) -> State {
    if !path.exists() {
        return State::default();
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save(path: &Path, state: &State) {
    let content = serde_json::to_string_pretty(state).expect("state serialization failed");
    std::fs::write(path, content).expect("failed to write state file");
}
