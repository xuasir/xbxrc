use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 缓存与在途查询共享一个槽位，LRU 只回收已经释放的槽位。
pub(super) struct DetailSlots<T> {
    clock: u64,
    capacity: usize,
    entries: HashMap<String, (u64, Arc<Mutex<Option<T>>>)>,
}
impl<T> DetailSlots<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            clock: 0,
            capacity,
            entries: HashMap::new(),
        }
    }
    pub(super) fn slot(&mut self, key: String) -> Option<Arc<Mutex<Option<T>>>> {
        self.clock += 1;
        if let Some((access, slot)) = self.entries.get_mut(&key) {
            *access = self.clock;
            return Some(slot.clone());
        }
        if self.entries.len() >= self.capacity {
            let idle = self
                .entries
                .iter()
                .filter(|(_, (_, slot))| Arc::strong_count(slot) == 1)
                .min_by_key(|(_, (access, _))| *access)
                .map(|(key, _)| key.clone())?;
            self.entries.remove(&idle);
        }
        let slot = Arc::new(Mutex::new(None));
        self.entries.insert(key, (self.clock, slot.clone()));
        Some(slot)
    }
}

#[cfg(test)]
#[path = "cloud_detail_cache.test.rs"]
mod tests;
