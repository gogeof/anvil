//! 文件读取缓存
//!
//! 功能：缓存文件内容，避免重复读取

use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::sync::{Arc, Mutex};

/// 缓存条目
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub content: String,
    pub created_at: Instant,
    pub ttl: Duration,
}

/// 文件缓存
#[derive(Debug)]
pub struct FileCache {
    entries: HashMap<String, CacheEntry>,
    max_size: usize,
}

impl FileCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
        }
    }
    
    /// 获取缓存
    pub fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).and_then(|entry| {
            if entry.created_at.elapsed() > entry.ttl {
                None
            } else {
                Some(entry.content.clone())
            }
        })
    }
    
    /// 设置缓存
    pub fn set(&mut self, key: String, content: String, ttl: Duration) {
        // LRU 淘汰
        if self.entries.len() >= self.max_size {
            if let Some(oldest) = self.entries.iter()
                .min_by_key(|(_, e)| e.created_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        
        self.entries.insert(key, CacheEntry {
            content,
            created_at: Instant::now(),
            ttl,
        });
    }
    
    /// 清除缓存
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    
    /// 获取缓存大小
    pub fn size(&self) -> usize {
        self.entries.len()
    }
}

impl Default for FileCache {
    fn default() -> Self {
        Self::new(100)
    }
}

/// 缓存键
pub fn cache_key(path: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    match (offset, limit) {
        (None, None) => format!("file:{}", path),
        (Some(o), None) => format!("file:{}:{}", path, o),
        (Some(o), Some(l)) => format!("file:{}:{}:{}", path, o, l),
        (None, Some(l)) => format!("file:{}::{}", path, l),
    }
}

/// 创建线程安全的缓存
pub fn create_thread_safe_cache(max_size: usize) -> Arc<Mutex<FileCache>> {
    Arc::new(Mutex::new(FileCache::new(max_size)))
}
