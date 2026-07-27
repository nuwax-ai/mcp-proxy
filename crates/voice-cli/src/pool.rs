//! 通用 round-robin 实例池。
//!
//! STT `engine_pool`（WhisperEngine）+ TTS `engine_pool`（OfflineTts）两处同构：
//! `Vec<Arc<Mutex<T>>>` + `AtomicUsize` round-robin + `pick()`。抽出单一实现，
//! round-robin 逻辑只维护一份（fastembed `ModelPool` 结构不同，不在此次统一范围）。
//!
//! 存 `Arc<Mutex<T>>`，`pick` 返回 Arc clone——锁留给调用方（`MutexGuard` 不能跨函数返回）。

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// round-robin 实例池。`T: Send` 保证 `Arc<Mutex<T>>: Send + Sync`，可入静态缓存。
pub struct Pool<T: Send> {
    instances: Vec<Arc<Mutex<T>>>,
    next: AtomicUsize,
}

impl<T: Send> Pool<T> {
    /// 构造：`instances` 不能为空（调用方应已 clamp pool_size >= 1，构造时再 assert 兜底）。
    pub fn new(instances: Vec<Arc<Mutex<T>>>) -> Self {
        assert!(
            !instances.is_empty(),
            "Pool::new 要求至少 1 个实例（pool_size 应被 clamp 到 >= 1）"
        );
        Self {
            instances,
            next: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// 池是否为空（实际不会发生：`new` 会 assert）。
    /// 仅为满足 clippy `len_without_is_empty` 约定。
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// round-robin 取实例。并发请求分散到不同实例 → 最多 N 路并行；
    /// 命中同一实例则在该实例上排队（竞争其 Mutex）。
    pub fn pick(&self) -> Arc<Mutex<T>> {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.instances.len();
        self.instances[idx].clone()
    }
}
