//! Local LRU chunk cache for the HyperLink Virtual Filesystem.
//!
//! Provides sub-millisecond cached reads for active file regions and
//! enforces a memory ceiling with Least-Recently-Used eviction.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Default chunk size: 128 KB.
pub const CHUNK_SIZE: usize = 128 * 1024;
/// Default maximum cache entries: 1024 entries * 128 KB = 128 MB memory ceiling.
pub const DEFAULT_MAX_CHUNKS: usize = 1024;

/// Unique key for a cached chunk.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub path: String,
    pub chunk_index: u64,
}

/// In-memory LRU chunk cache.
pub struct FileChunkCache {
    max_chunks: usize,
    chunks: Mutex<HashMap<ChunkKey, Vec<u8>>>,
    lru_order: Mutex<VecDeque<ChunkKey>>,
}

impl FileChunkCache {
    /// Creates a new chunk cache with the specified maximum number of chunks.
    pub fn new(max_chunks: usize) -> Self {
        Self {
            max_chunks: max_chunks.max(1),
            chunks: Mutex::new(HashMap::new()),
            lru_order: Mutex::new(VecDeque::new()),
        }
    }

    /// Retrieves a chunk from the cache, updating its LRU recency.
    pub fn get(&self, path: &str, chunk_index: u64) -> Option<Vec<u8>> {
        let key = ChunkKey {
            path: path.to_string(),
            chunk_index,
        };
        let chunks = self.chunks.lock().unwrap();
        if let Some(data) = chunks.get(&key) {
            let mut lru = self.lru_order.lock().unwrap();
            if let Some(pos) = lru.iter().position(|k| k == &key) {
                lru.remove(pos);
            }
            lru.push_back(key);
            Some(data.clone())
        } else {
            None
        }
    }

    /// Stores a chunk in the cache, evicting the least recently used chunk if at capacity.
    pub fn put(&self, path: &str, chunk_index: u64, data: Vec<u8>) {
        let key = ChunkKey {
            path: path.to_string(),
            chunk_index,
        };

        let mut chunks = self.chunks.lock().unwrap();
        let mut lru = self.lru_order.lock().unwrap();

        if chunks.len() >= self.max_chunks && !chunks.contains_key(&key) {
            if let Some(evicted_key) = lru.pop_front() {
                chunks.remove(&evicted_key);
            }
        }

        if let Some(pos) = lru.iter().position(|k| k == &key) {
            lru.remove(pos);
        }
        lru.push_back(key.clone());
        chunks.insert(key, data);
    }

    /// Invalidates all cached chunks for a specific path (e.g. on write/truncate).
    pub fn invalidate_path(&self, path: &str) {
        let mut chunks = self.chunks.lock().unwrap();
        let mut lru = self.lru_order.lock().unwrap();

        chunks.retain(|k, _| k.path != path);
        lru.retain(|k| k.path != path);
    }

    /// Returns the current number of cached chunks.
    pub fn len(&self) -> usize {
        self.chunks.lock().unwrap().len()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for FileChunkCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CHUNKS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_cache_put_get_lru() {
        let cache = FileChunkCache::new(2);

        cache.put("/photo.jpg", 0, vec![1, 2, 3]);
        cache.put("/photo.jpg", 1, vec![4, 5, 6]);
        assert_eq!(cache.len(), 2);

        // Access chunk 0 so chunk 1 becomes LRU
        assert_eq!(cache.get("/photo.jpg", 0), Some(vec![1, 2, 3]));

        // Put chunk 2, should evict chunk 1
        cache.put("/video.mp4", 0, vec![7, 8, 9]);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get("/photo.jpg", 1), None);
        assert_eq!(cache.get("/photo.jpg", 0), Some(vec![1, 2, 3]));
        assert_eq!(cache.get("/video.mp4", 0), Some(vec![7, 8, 9]));
    }

    #[test]
    fn test_chunk_cache_invalidate() {
        let cache = FileChunkCache::new(10);
        cache.put("/doc.pdf", 0, vec![1]);
        cache.put("/doc.pdf", 1, vec![2]);
        cache.put("/other.txt", 0, vec![3]);

        cache.invalidate_path("/doc.pdf");
        assert_eq!(cache.get("/doc.pdf", 0), None);
        assert_eq!(cache.get("/doc.pdf", 1), None);
        assert_eq!(cache.get("/other.txt", 0), Some(vec![3]));
    }
}
