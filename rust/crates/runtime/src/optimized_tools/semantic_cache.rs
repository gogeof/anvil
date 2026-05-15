//! 语义缓存
//!
//! 功能：缓存文件摘要和搜索结果，避免重复读取

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Instant, Duration};

/// 缓存条目
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub data: T,
    pub created_at: Instant,
    pub ttl: Duration,
    pub file_hash: Option<String>,
}

/// 语义缓存
pub struct SemanticCache<T> {
    entries: HashMap<String, CacheEntry<T>>,
    max_size: usize,
}

impl<T: Clone> SemanticCache<T> {
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_size,
        }
    }
    
    /// 获取缓存
    pub fn get(&self, key: &str) -> Option<&T> {
        self.entries.get(key).and_then(|entry| {
            // 检查是否过期
            if entry.created_at.elapsed() > entry.ttl {
                None
            } else {
                Some(&entry.data)
            }
        })
    }
    
    /// 设置缓存
    pub fn set(&mut self, key: String, data: T, ttl: Duration) {
        // 检查容量
        if self.entries.len() >= self.max_size {
            // 移除最旧的条目
            let oldest = self.entries.iter()
                .min_by_key(|(_, e)| e.created_at)
                .map(|(k, _)| k.clone());
            
            if let Some(k) = oldest {
                self.entries.remove(&k);
            }
        }
        
        self.entries.insert(key, CacheEntry {
            data,
            created_at: Instant::now(),
            ttl,
            file_hash: None,
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

impl<T: Clone> Default for SemanticCache<T> {
    fn default() -> Self {
        Self::new(100)
    }
}

/// 文件摘要缓存键
pub fn file_summary_cache_key(path: &PathBuf) -> String {
    format!("file_summary:{}", path.display())
}

/// 搜索结果缓存键
pub fn search_cache_key(pattern: &str, path: &str) -> String {
    format!("search:{}:{}", pattern, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cache_set_get() {
        let mut cache: SemanticCache<String> = SemanticCache::new(10);
        
        cache.set("key1".to_string(), "value1".to_string(), Duration::from_secs(60));
        
        assert_eq!(cache.get("key1"), Some(&"value1".to_string()));
        assert_eq!(cache.get("key2"), None);
    }
    
    #[test]
    fn test_cache_eviction() {
        let mut cache: SemanticCache<String> = SemanticCache::new(2);
        
        cache.set("key1".to_string(), "value1".to_string(), Duration::from_secs(60));
        cache.set("key2".to_string(), "value2".to_string(), Duration::from_secs(60));
        cache.set("key3".to_string(), "value3".to_string(), Duration::from_secs(60));
        
        // 应该已经移除了最旧的条目
        assert_eq!(cache.size(), 2);
    }
}
