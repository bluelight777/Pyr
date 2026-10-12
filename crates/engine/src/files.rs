//! Images downloaded from servers, kept in memory (never on disk) with a size limit.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum FileState {
    Missing,
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed(String),
}

/// About 300 MB of images; the oldest go first.
const LIMIT: usize = 300 << 20;

#[derive(Default)]
pub struct FileCache {
    files: HashMap<String, FileState>,
    order: VecDeque<String>,
    bytes: usize,
}

impl FileCache {
    pub fn get(&self, id: &str) -> FileState {
        self.files.get(id).cloned().unwrap_or(FileState::Missing)
    }

    pub fn set_loading(&mut self, id: &str) {
        self.files.insert(id.to_string(), FileState::Loading);
    }

    pub fn set_failed(&mut self, id: &str, why: String) {
        self.files.insert(id.to_string(), FileState::Failed(why));
    }

    pub fn insert(&mut self, id: String, bytes: Arc<Vec<u8>>) {
        self.bytes += bytes.len();
        if let Some(FileState::Ready(old)) = self.files.insert(id.clone(), FileState::Ready(bytes)) {
            self.bytes -= old.len();
        } else {
            self.order.push_back(id);
        }
        while self.bytes > LIMIT {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(FileState::Ready(b)) = self.files.remove(&old) {
                self.bytes -= b.len();
            }
        }
    }

    /// Forget a file (its message was deleted).
    pub fn forget(&mut self, id: &str) {
        if let Some(FileState::Ready(b)) = self.files.remove(id) {
            self.bytes -= b.len();
        }
        self.order.retain(|x| x != id);
    }
}
