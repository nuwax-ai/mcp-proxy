//! 并发优化器
//! 
//! 提供任务队列管理、工作线程池和负载均衡功能

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::collections::VecDeque;
use dashmap::DashMap;
use tokio::sync::{Semaphore, RwLock, Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use futures::future::BoxFuture;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::error::AppError;
use super::{PerformanceOptimizable, ConcurrencyConfig};

/// 并发优化器
pub struct ConcurrencyOptimizer {
    config: ConcurrencyConfig,
    task_queue: Arc<TaskQueue>,
    worker_pool: Arc<WorkerPool>,
    load_balancer: Arc<LoadBalancer>,
    semaphore: Arc<Semaphore>,
    stats: Arc<ConcurrencyStats>,
}

impl ConcurrencyOptimizer {
    /// 创建新的并发优化器
    pub async fn new(config: &AppConfig) -> Result<Self, AppError> {
        let concurrency_config = ConcurrencyConfig::default(); // 从配置中获取
        
        let task_queue = Arc::new(TaskQueue::new(concurrency_config.task_queue_size));
        let worker_pool = Arc::new(WorkerPool::new(concurrency_config.worker_threads).await?);
        let load_balancer = Arc::new(LoadBalancer::new());
        let semaphore = Arc::new(Semaphore::new(concurrency_config.max_concurrent_tasks));
        let stats = Arc::new(ConcurrencyStats::new());
        
        Ok(Self {
            config: concurrency_config,
            task_queue,
            worker_pool,
            load_balancer,
            semaphore,
            stats,
        })
    }
    
    /// 提交任务
    pub async fn submit_task<F, T>(&self, task: F) -> Result<TaskHandle<T>, AppError>
    where
        F: FnOnce() -> BoxFuture<'static, Result<T, AppError>> + Send + 'static,
        T: Send + 'static,
    {
        // 获取信号量许可
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| AppError::Config("Concurrency limit exceeded".to_string()))?;
        
        // 创建任务
        let task_id = Uuid::new_v4().to_string();
        let (result_tx, result_rx) = oneshot::channel();
        
        let concurrent_task = ConcurrentTask {
            id: task_id.clone(),
            task: Box::new(move || {
                Box::pin(async move {
                    let result = task().await;
                    let _ = result_tx.send(result);
                })
            }),
            priority: TaskPriority::Normal,
            submitted_at: Instant::now(),
            timeout: self.config.task_timeout,
        };
        
        // 提交到队列
        self.task_queue.enqueue(concurrent_task).await?;
        
        // 更新统计
        self.stats.record_task_submitted().await;
        
        // 通知工作池有新任务
        self.worker_pool.notify_new_task().await;
        
        Ok(TaskHandle {
            id: task_id,
            result_rx,
            _permit: permit,
        })
    }
    
    /// 提交高优先级任务
    pub async fn submit_priority_task<F, T>(&self, task: F) -> Result<TaskHandle<T>, AppError>
    where
        F: FnOnce() -> BoxFuture<'static, Result<T, AppError>> + Send + 'static,
        T: Send + 'static,
    {
        let permit = self.semaphore.clone().acquire_owned().await
            .map_err(|_| AppError::Config("Concurrency limit exceeded".to_string()))?;
        
        let task_id = Uuid::new_v4().to_string();
        let (result_tx, result_rx) = oneshot::channel();
        
        let concurrent_task = ConcurrentTask {
            id: task_id.clone(),
            task: Box::new(move || {
                Box::pin(async move {
                    let result = task().await;
                    let _ = result_tx.send(result);
                })
            }),
            priority: TaskPriority::High,
            submitted_at: Instant::now(),
            timeout: self.config.task_timeout,
        };
        
        self.task_queue.enqueue_priority(concurrent_task).await?;
        self.stats.record_priority_task_submitted().await;
        self.worker_pool.notify_new_task().await;
        
        Ok(TaskHandle {
            id: task_id,
            result_rx,
            _permit: permit,
        })
    }
    
    /// 获取队列状态
    pub async fn get_queue_status(&self) -> QueueStatus {
        QueueStatus {
            pending_tasks: self.task_queue.pending_count().await,
            active_tasks: self.worker_pool.active_count().await,
            available_workers: self.worker_pool.available_count().await,
            queue_capacity: self.config.task_queue_size,
        }
    }
    
    /// 获取并发统计
    pub async fn get_concurrency_stats(&self) -> Result<ConcurrencyStats, AppError> {
        Ok(self.stats.clone_stats().await)
    }
    
    /// 调整并发参数
    pub async fn adjust_concurrency(&self, new_max_concurrent: usize) -> Result<(), AppError> {
        // 动态调整信号量
        let current_permits = self.semaphore.available_permits();
        
        if new_max_concurrent > current_permits {
            self.semaphore.add_permits(new_max_concurrent - current_permits);
        }
        
        self.stats.record_concurrency_adjustment(new_max_concurrent).await;
        
        Ok(())
    }
}

#[async_trait::async_trait]
impl PerformanceOptimizable for ConcurrencyOptimizer {
    async fn optimize(&self) -> Result<(), AppError> {
        // 优化任务队列
        self.task_queue.optimize().await?;
        
        // 优化工作池
        self.worker_pool.optimize().await?;
        
        // 执行负载均衡
        self.load_balancer.balance(&self.worker_pool).await?;
        
        Ok(())
    }
    
    async fn get_stats(&self) -> Result<serde_json::Value, AppError> {
        let stats = self.get_concurrency_stats().await?;
        let queue_status = self.get_queue_status().await;
        
        Ok(serde_json::json!({
            "stats": stats,
            "queue_status": queue_status
        }))
    }
    
    async fn reset_stats(&self) -> Result<(), AppError> {
        self.stats.reset().await;
        Ok(())
    }
}

/// 任务队列
pub struct TaskQueue {
    normal_queue: Arc<Mutex<VecDeque<ConcurrentTask>>>,
    priority_queue: Arc<Mutex<VecDeque<ConcurrentTask>>>,
    max_size: usize,
    stats: Arc<QueueStats>,
}

impl TaskQueue {
    pub fn new(max_size: usize) -> Self {
        Self {
            normal_queue: Arc::new(Mutex::new(VecDeque::new())),
            priority_queue: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
            stats: Arc::new(QueueStats::new()),
        }
    }
    
    pub async fn enqueue(&self, task: ConcurrentTask) -> Result<(), AppError> {
        let mut queue = self.normal_queue.lock().await;
        
        if queue.len() >= self.max_size {
            return Err(AppError::Config("Queue is full".to_string()));
        }
        
        queue.push_back(task);
        self.stats.record_enqueue().await;
        
        Ok(())
    }
    
    pub async fn enqueue_priority(&self, task: ConcurrentTask) -> Result<(), AppError> {
        let mut queue = self.priority_queue.lock().await;
        
        if queue.len() >= self.max_size / 2 { // 优先级队列占用一半容量
            return Err(AppError::Config("Priority queue is full".to_string()));
        }
        
        queue.push_back(task);
        self.stats.record_priority_enqueue().await;
        
        Ok(())
    }
    
    pub async fn dequeue(&self) -> Option<ConcurrentTask> {
        // 优先处理高优先级任务
        {
            let mut priority_queue = self.priority_queue.lock().await;
            if let Some(task) = priority_queue.pop_front() {
                self.stats.record_priority_dequeue().await;
                return Some(task);
            }
        }
        
        // 处理普通任务
        let mut normal_queue = self.normal_queue.lock().await;
        if let Some(task) = normal_queue.pop_front() {
            self.stats.record_dequeue().await;
            return Some(task);
        }
        
        None
    }
    
    pub async fn pending_count(&self) -> usize {
        let normal_count = self.normal_queue.lock().await.len();
        let priority_count = self.priority_queue.lock().await.len();
        normal_count + priority_count
    }
    
    pub async fn optimize(&self) -> Result<(), AppError> {
        // 清理超时任务
        let now = Instant::now();
        
        {
            let mut normal_queue = self.normal_queue.lock().await;
            normal_queue.retain(|task| now.duration_since(task.submitted_at) < task.timeout);
        }
        
        {
            let mut priority_queue = self.priority_queue.lock().await;
            priority_queue.retain(|task| now.duration_since(task.submitted_at) < task.timeout);
        }
        
        self.stats.record_cleanup().await;
        
        Ok(())
    }
}

/// 工作线程池
pub struct WorkerPool {
    workers: Vec<Worker>,
    task_sender: mpsc::UnboundedSender<WorkerMessage>,
    stats: Arc<WorkerStats>,
}

impl WorkerPool {
    pub async fn new(worker_count: usize) -> Result<Self, AppError> {
        let (task_sender, task_receiver) = mpsc::unbounded_channel();
        let task_receiver = Arc::new(Mutex::new(task_receiver));
        let stats = Arc::new(WorkerStats::new());
        
        let mut workers = Vec::new();
        
        for i in 0..worker_count {
            let worker = Worker::new(
                i,
                task_receiver.clone(),
                stats.clone(),
            ).await?;
            workers.push(worker);
        }
        
        Ok(Self {
            workers,
            task_sender,
            stats,
        })
    }
    
    pub async fn notify_new_task(&self) {
        let _ = self.task_sender.send(WorkerMessage::NewTask);
    }
    
    pub async fn active_count(&self) -> usize {
        self.stats.active_workers().await
    }
    
    pub async fn available_count(&self) -> usize {
        self.workers.len() - self.active_count().await
    }
    
    pub async fn optimize(&self) -> Result<(), AppError> {
        // 检查工作线程健康状态
        for worker in &self.workers {
            if !worker.is_healthy().await {
                worker.restart().await?;
            }
        }
        
        Ok(())
    }
}

/// 工作线程
pub struct Worker {
    id: usize,
    handle: JoinHandle<()>,
    is_active: Arc<AtomicUsize>,
    last_activity: Arc<RwLock<Instant>>,
}

impl Worker {
    pub async fn new(
        id: usize,
        task_receiver: Arc<Mutex<mpsc::UnboundedReceiver<WorkerMessage>>>,
        stats: Arc<WorkerStats>,
    ) -> Result<Self, AppError> {
        let is_active = Arc::new(AtomicUsize::new(0));
        let last_activity = Arc::new(RwLock::new(Instant::now()));
        
        let worker_is_active = is_active.clone();
        let worker_last_activity = last_activity.clone();
        let worker_stats = stats.clone();
        
        let handle = tokio::spawn(async move {
            loop {
                // 等待任务消息
                let message = {
                    let mut receiver = task_receiver.lock().await;
                    receiver.recv().await
                };
                
                match message {
                    Some(WorkerMessage::NewTask) => {
                        worker_is_active.store(1, Ordering::Relaxed);
                        *worker_last_activity.write().await = Instant::now();
                        
                        // 处理任务的逻辑在这里
                        // 实际实现中会从队列中获取任务并执行
                        
                        worker_stats.record_task_completed().await;
                        worker_is_active.store(0, Ordering::Relaxed);
                    }
                    Some(WorkerMessage::Shutdown) => break,
                    None => break, // 通道关闭
                }
            }
        });
        
        Ok(Self {
            id,
            handle,
            is_active,
            last_activity,
        })
    }
    
    pub async fn is_healthy(&self) -> bool {
        let last_activity = *self.last_activity.read().await;
        let inactive_duration = last_activity.elapsed();
        
        // 如果工作线程超过5分钟没有活动，认为不健康
        inactive_duration < Duration::from_secs(300)
    }
    
    pub async fn restart(&self) -> Result<(), AppError> {
        // 重启工作线程的逻辑
        // 在实际实现中，这里会重新创建工作线程
        Ok(())
    }
}

/// 负载均衡器
pub struct LoadBalancer {
    strategy: LoadBalancingStrategy,
    stats: Arc<LoadBalancerStats>,
}

impl LoadBalancer {
    pub fn new() -> Self {
        Self {
            strategy: LoadBalancingStrategy::RoundRobin,
            stats: Arc::new(LoadBalancerStats::new()),
        }
    }
    
    pub async fn balance(&self, worker_pool: &WorkerPool) -> Result<(), AppError> {
        match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                // 轮询负载均衡逻辑
            }
            LoadBalancingStrategy::LeastConnections => {
                // 最少连接负载均衡逻辑
            }
            LoadBalancingStrategy::WeightedRoundRobin => {
                // 加权轮询负载均衡逻辑
            }
        }
        
        self.stats.record_balance_operation().await;
        
        Ok(())
    }
}

/// 任务句柄
pub struct TaskHandle<T> {
    pub id: String,
    result_rx: oneshot::Receiver<Result<T, AppError>>,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl<T> TaskHandle<T> {
    /// 等待任务完成
    pub async fn await_result(self) -> Result<T, AppError> {
        match self.result_rx.await {
            Ok(result) => result,
            Err(_) => Err(AppError::Config("Task was cancelled".to_string())),
        }
    }
    
    /// 等待任务完成（带超时）
    pub async fn await_result_timeout(self, timeout: Duration) -> Result<T, AppError> {
        match tokio::time::timeout(timeout, self.result_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppError::Config("Task was cancelled".to_string())),
            Err(_) => Err(AppError::Config("Task timed out".to_string())),
        }
    }
}

/// 并发任务
struct ConcurrentTask {
    id: String,
    task: Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>,
    priority: TaskPriority,
    submitted_at: Instant,
    timeout: Duration,
}

/// 任务优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// 工作线程消息
enum WorkerMessage {
    NewTask,
    Shutdown,
}

/// 负载均衡策略
#[derive(Debug, Clone, Copy)]
enum LoadBalancingStrategy {
    RoundRobin,
    LeastConnections,
    WeightedRoundRobin,
}

/// 并发统计
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConcurrencyStats {
    pub total_tasks_submitted: u64,
    pub total_tasks_completed: u64,
    pub total_tasks_failed: u64,
    pub priority_tasks_submitted: u64,
    pub average_task_duration: Duration,
    pub peak_concurrent_tasks: usize,
    pub queue_stats: QueueStatsData,
    pub worker_stats: WorkerStatsData,
}

impl ConcurrencyStats {
    pub fn new() -> Self {
        Self {
            total_tasks_submitted: 0,
            total_tasks_completed: 0,
            total_tasks_failed: 0,
            priority_tasks_submitted: 0,
            average_task_duration: Duration::from_secs(0),
            peak_concurrent_tasks: 0,
            queue_stats: QueueStatsData::new(),
            worker_stats: WorkerStatsData::new(),
        }
    }
    
    pub async fn record_task_submitted(&self) {
        // 原子操作记录
    }
    
    pub async fn record_priority_task_submitted(&self) {
        // 原子操作记录
    }
    
    pub async fn record_concurrency_adjustment(&self, _new_max: usize) {
        // 记录并发调整
    }
    
    pub async fn clone_stats(&self) -> ConcurrencyStats {
        self.clone()
    }
    
    pub async fn reset(&self) {
        // 重置统计数据
    }
}

/// 队列状态
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueStatus {
    pub pending_tasks: usize,
    pub active_tasks: usize,
    pub available_workers: usize,
    pub queue_capacity: usize,
}

/// 其他统计结构
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueStatsData {
    pub enqueues: u64,
    pub dequeues: u64,
    pub priority_enqueues: u64,
    pub priority_dequeues: u64,
    pub cleanups: u64,
}

impl QueueStatsData {
    pub fn new() -> Self {
        Self {
            enqueues: 0,
            dequeues: 0,
            priority_enqueues: 0,
            priority_dequeues: 0,
            cleanups: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkerStatsData {
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub total_processing_time: Duration,
    pub restarts: u64,
}

impl WorkerStatsData {
    pub fn new() -> Self {
        Self {
            tasks_completed: 0,
            tasks_failed: 0,
            total_processing_time: Duration::from_secs(0),
            restarts: 0,
        }
    }
}

// 辅助统计结构
struct QueueStats {
    enqueues: AtomicU64,
    dequeues: AtomicU64,
    priority_enqueues: AtomicU64,
    priority_dequeues: AtomicU64,
    cleanups: AtomicU64,
}

impl QueueStats {
    fn new() -> Self {
        Self {
            enqueues: AtomicU64::new(0),
            dequeues: AtomicU64::new(0),
            priority_enqueues: AtomicU64::new(0),
            priority_dequeues: AtomicU64::new(0),
            cleanups: AtomicU64::new(0),
        }
    }
    
    async fn record_enqueue(&self) {
        self.enqueues.fetch_add(1, Ordering::Relaxed);
    }
    
    async fn record_dequeue(&self) {
        self.dequeues.fetch_add(1, Ordering::Relaxed);
    }
    
    async fn record_priority_enqueue(&self) {
        self.priority_enqueues.fetch_add(1, Ordering::Relaxed);
    }
    
    async fn record_priority_dequeue(&self) {
        self.priority_dequeues.fetch_add(1, Ordering::Relaxed);
    }
    
    async fn record_cleanup(&self) {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
    }
}

struct WorkerStats {
    active_workers: AtomicUsize,
    tasks_completed: AtomicU64,
    tasks_failed: AtomicU64,
}

impl WorkerStats {
    fn new() -> Self {
        Self {
            active_workers: AtomicUsize::new(0),
            tasks_completed: AtomicU64::new(0),
            tasks_failed: AtomicU64::new(0),
        }
    }
    
    async fn active_workers(&self) -> usize {
        self.active_workers.load(Ordering::Relaxed)
    }
    
    async fn record_task_completed(&self) {
        self.tasks_completed.fetch_add(1, Ordering::Relaxed);
    }
}

struct LoadBalancerStats {
    balance_operations: AtomicU64,
}

impl LoadBalancerStats {
    fn new() -> Self {
        Self {
            balance_operations: AtomicU64::new(0),
        }
    }
    
    async fn record_balance_operation(&self) {
        self.balance_operations.fetch_add(1, Ordering::Relaxed);
    }
}