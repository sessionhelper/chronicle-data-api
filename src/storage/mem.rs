//! In-memory object store used by integration tests.
//!
//! Never wired into the binary — exists so tests can exercise the chunk
//! upload / download / cascade-delete paths without an S3 endpoint.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::AppError;
use crate::storage::ObjectStore;

#[derive(Default)]
pub struct MemStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl ObjectStore for MemStore {
    async fn put(&self, key: &str, body: Vec<u8>, _content_type: &str) -> Result<(), AppError> {
        self.objects.lock().unwrap().insert(key.to_string(), body);
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, AppError> {
        self.objects
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("object {key} not found")))
    }

    async fn delete_many(&self, keys: &[String]) -> Result<u32, AppError> {
        let mut map = self.objects.lock().unwrap();
        let mut n = 0u32;
        for k in keys {
            if map.remove(k).is_some() {
                n += 1;
            }
        }
        Ok(n)
    }

    async fn head_bucket(&self) -> Result<(), AppError> {
        Ok(())
    }
}
