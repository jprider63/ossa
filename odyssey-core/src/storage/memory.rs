
use crate::storage::Storage;

pub struct MemoryStorage {
}

impl MemoryStorage {
    pub fn new() -> MemoryStorage {
        MemoryStorage {
        }
    }
}

impl Storage for MemoryStorage {
}
