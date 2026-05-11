use std::hash::Hash;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Wrapper for cached values with timestamp for TTL validation
#[derive(Serialize, Deserialize)]
struct CachedEntry<V> {
    value: V,
    timestamp: u128,
}

/// Generic content-addressable key-value storage using cacache.
///
/// This storage provides a type-safe wrapper around cacache for arbitrary
/// key-value caching with content verification. Keys are serialized to
/// deterministic strings using hash values, and values are stored as JSON
/// using serde_json for maximum compatibility.
pub struct CacacheStorage {
    cache_dir: PathBuf,
    ttl_seconds: Option<u128>,
}

impl CacacheStorage {
    /// Creates a new key-value storage with the specified cache directory.
    ///
    /// The directory will be created if it doesn't exist. All cache data
    /// will be stored under this directory using cacache's content-addressable
    /// storage format.
    ///
    /// # Arguments
    /// * `cache_dir` - Directory where cache data will be stored
    /// * `ttl_seconds` - Optional TTL in seconds. If provided, entries older
    ///   than this will be considered expired.
    pub fn new(cache_dir: PathBuf, ttl_seconds: Option<u128>) -> Self {
        Self { cache_dir, ttl_seconds }
    }

    /// Converts a key to a deterministic cache key string using its hash value.
    fn key_to_string<K>(&self, key: &K) -> Result<String>
    where
        K: Hash,
    {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        Ok(hasher.finish().to_string())
    }

    /// Gets the current Unix timestamp in seconds
    fn get_current_timestamp() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before UNIX epoch")
            .as_secs() as u128
    }

    /// Checks if a cached entry has expired based on TTL
    fn is_expired(&self, timestamp: u128) -> bool {
        if let Some(ttl) = self.ttl_seconds {
            let current = Self::get_current_timestamp();
            current.saturating_sub(timestamp) > ttl
        } else {
            false
        }
    }
}

#[async_trait::async_trait]
impl forge_app::KVStore for CacacheStorage {
    async fn cache_get<K, V>(&self, key: &K) -> Result<Option<V>>
    where
        K: Hash + Sync,
        V: serde::Serialize + DeserializeOwned + Send,
    {
        let key_str = self.key_to_string(key)?;

        match cacache::read(&self.cache_dir, &key_str).await {
            Ok(data) => {
                // Try to deserialize the cached entry
                match serde_json::from_slice::<CachedEntry<V>>(&data) {
                    Ok(entry) => {
                        // Check if entry has expired
                        if self.is_expired(entry.timestamp) {
                            Ok(None)
                        } else {
                            Ok(Some(entry.value))
                        }
                    }
                    Err(_) => {
                        // Failed to deserialize (likely due to format change)
                        // Clear the invalid cache entry to maintain backward compatibility
                        let _ = cacache::remove(&self.cache_dir, &key_str).await;
                        Ok(None)
                    }
                }
            }
            Err(e) => {
                // Check if error is NotFound by converting to string and checking message
                // cacache errors don't have a kind() method
                let error_str = e.to_string();
                if error_str.contains("not found") || error_str.contains("NotFound") {
                    Ok(None)
                } else {
                    Err(e).context("Failed to read from cache")
                }
            }
        }
    }

    async fn cache_set<K, V>(&self, key: &K, value: &V) -> Result<()>
    where
        K: Hash + Sync,
        V: serde::Serialize + Sync,
    {
        let key_str = self.key_to_string(key)?;

        let entry = CachedEntry { value, timestamp: Self::get_current_timestamp() };

        let data = serde_json::to_vec(&entry).context("Failed to serialize entry for caching")?;

        cacache::write(&self.cache_dir, &key_str, data)
            .await
            .context("Failed to write to cache")?;

        Ok(())
    }

    async fn cache_clear(&self) -> Result<()> {
        if !self.cache_dir.exists() {
            return Ok(());
        }
        cacache::clear(&self.cache_dir)
            .await
            .context("Failed to clear cache")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use forge_app::KVStore;
    use pretty_assertions::assert_eq;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
    struct TestKey {
        id: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TestValue {
        data: String,
        count: i32,
    }

    fn test_cache_dir() -> PathBuf {
        tempfile::tempdir().unwrap().keep()
    }

    #[tokio::test]
    async fn test_get_nonexistent_key() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, None);

        let key = TestKey { id: "test".to_string() };
        let result: Option<TestValue> = cache.cache_get(&key).await.unwrap();

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_set_and_get() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, None);

        let key = TestKey { id: "test".to_string() };
        let value = TestValue { data: "hello".to_string(), count: 42 };

        cache.cache_set(&key, &value).await.unwrap();
        let result: Option<TestValue> = cache.cache_get(&key).await.unwrap();

        assert_eq!(result, Some(value));
    }

    #[tokio::test]
    async fn test_clear() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, None);

        let key1 = TestKey { id: "test1".to_string() };
        let key2 = TestKey { id: "test2".to_string() };
        let value = TestValue { data: "hello".to_string(), count: 42 };

        cache.cache_set(&key1, &value).await.unwrap();
        cache.cache_set(&key2, &value).await.unwrap();

        cache.cache_clear().await.unwrap();

        let result1: Option<TestValue> = cache.cache_get(&key1).await.unwrap();
        let result2: Option<TestValue> = cache.cache_get(&key2).await.unwrap();

        assert_eq!(result1, None);
        assert_eq!(result2, None);
    }

    #[tokio::test]
    async fn test_ttl_not_expired() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, Some(60)); // 60 seconds TTL

        let key = TestKey { id: "test".to_string() };
        let value = TestValue { data: "hello".to_string(), count: 42 };

        cache.cache_set(&key, &value).await.unwrap();

        // Immediately retrieve - should not be expired
        let result: Option<TestValue> = cache.cache_get(&key).await.unwrap();

        assert_eq!(result, Some(value));
    }

    #[tokio::test]
    async fn test_ttl_expired() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, Some(1)); // 1 second TTL

        let key = TestKey { id: "test".to_string() };
        let value = TestValue { data: "hello".to_string(), count: 42 };

        cache.cache_set(&key, &value).await.unwrap();

        // Wait for TTL to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let result: Option<TestValue> = cache.cache_get(&key).await.unwrap();

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_ttl_none_never_expires() {
        let cache_dir = test_cache_dir();
        let cache = CacacheStorage::new(cache_dir, None); // No TTL

        let key = TestKey { id: "test".to_string() };
        let value = TestValue { data: "hello".to_string(), count: 42 };

        cache.cache_set(&key, &value).await.unwrap();

        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Should still be available
        let result: Option<TestValue> = cache.cache_get(&key).await.unwrap();

        assert_eq!(result, Some(value));
    }

    #[tokio::test]
    async fn test_should_not_fail_when_no_cache_dir_present() {
        let cache_dir = PathBuf::from("/tmp/forge_test_nonexistent_cache_dir_that_does_not_exist");
        let cache = CacacheStorage::new(cache_dir, None);
        let actual = cache.cache_clear().await;
        assert!(actual.is_ok());
    }
}
