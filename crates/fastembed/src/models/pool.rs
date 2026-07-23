use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{EmbeddingType, InitializedModel};

/// Model instance pool: N independent instances, round-robin allocation.
/// - pool_size=1: single instance, concurrent requests queue (CPU-optimal)
/// - pool_size>1: N-way concurrent inference (N x memory)
pub struct ModelPool<T> {
    instances: Vec<Arc<Mutex<T>>>,
    next: AtomicUsize,
}

impl<T> ModelPool<T> {
    pub fn new(instances: Vec<T>) -> Self {
        let instances = instances
            .into_iter()
            .map(|m| Arc::new(Mutex::new(m)))
            .collect();
        Self {
            instances,
            next: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Round-robin pick. Concurrent requests spread across N instances.
    /// If two requests hit the same instance, they queue on that instance's Mutex.
    pub fn pick(&self) -> Arc<Mutex<T>> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.instances.len();
        self.instances[idx].clone()
    }
}

/// Global model cache: keyed by (type, model code), each entry is an N-instance pool
pub type CacheKey = (EmbeddingType, String);
pub static MODEL_CACHE: Lazy<DashMap<CacheKey, Arc<ModelPool<InitializedModel>>>> =
    Lazy::new(DashMap::new);

/// Per-model init locks: serialize concurrent first-time loads of the same model
pub static INIT_LOCKS: Lazy<DashMap<CacheKey, Arc<Mutex<()>>>> = Lazy::new(DashMap::new);
