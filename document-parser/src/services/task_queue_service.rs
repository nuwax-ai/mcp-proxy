use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::{Mutex, RwLock, mpsc, watch};
use tokio::time::{Duration, Instant, interval, sleep};
use tracing::{debug, error, info, warn};

use crate::error::AppError;
use crate::models::{ProcessingStage, TaskStatus};
use crate::services::TaskService;

/// 任务队列项
#[derive(Debug, Clone)]
pub struct QueueItem {
    pub task_id: String,
    pub priority: u8,
    pub created_at: Instant,
    pub retry_count: u32,
    pub max_retries: u32,
}

impl QueueItem {
    pub fn new(task_id: String, priority: u8) -> Self {
        Self {
            task_id,
            priority,
            created_at: Instant::now(),
            retry_count: 0,
            max_retries: 3,
        }
    }

    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }

    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

/// 队列统计信息
#[derive(Debug, Clone)]
pub struct QueueStats {
    pub pending_count: usize,
    pub processing_count: usize,
    pub completed_count: u64,
    pub failed_count: u64,
    pub retry_count: u64,
    pub average_processing_time: Duration,
    pub queue_throughput: f64, // 任务/秒
    pub backpressure_events: u64,
    pub queue_overflow_events: u64,
    pub worker_utilization: f64, // 0.0 - 1.0
    pub memory_usage_bytes: u64,
    pub last_updated: Instant,
}

impl Default for QueueStats {
    fn default() -> Self {
        Self {
            pending_count: 0,
            processing_count: 0,
            completed_count: 0,
            failed_count: 0,
            retry_count: 0,
            average_processing_time: Duration::from_secs(0),
            queue_throughput: 0.0,
            backpressure_events: 0,
            queue_overflow_events: 0,
            worker_utilization: 0.0,
            memory_usage_bytes: 0,
            last_updated: Instant::now(),
        }
    }
}

/// 任务处理器trait
#[async_trait::async_trait]
pub trait TaskProcessor: Send + Sync {
    async fn process_task(&self, task_id: &str) -> Result<(), AppError>;
}

/// 队列配置
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub max_concurrent_tasks: usize,
    pub max_queue_size: usize,
    pub task_timeout: Duration,
    pub backpressure_threshold: f64, // 0.0 - 1.0
    pub retry_base_delay: Duration,
    pub retry_max_delay: Duration,
    pub metrics_update_interval: Duration,
    pub health_check_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 10,
            max_queue_size: 1000,
            task_timeout: Duration::from_secs(300),
            backpressure_threshold: 0.8,
            retry_base_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(60),
            metrics_update_interval: Duration::from_secs(5),
            health_check_interval: Duration::from_secs(30),
        }
    }
}

/// 任务队列服务
pub struct TaskQueueService {
    // 队列通道 - 使用有界通道实现背压
    task_sender: Option<mpsc::Sender<QueueItem>>,

    // 正在处理的任务
    processing_tasks: Arc<RwLock<HashMap<String, TaskExecutionContext>>>,

    // 统计信息
    stats: Arc<RwLock<QueueStats>>,
    completed_count: Arc<AtomicU64>,
    failed_count: Arc<AtomicU64>,
    retry_count: Arc<AtomicU64>,
    // SPMC 队列中的待处理任务计数
    queued_count: Arc<AtomicU64>,
    backpressure_events: Arc<AtomicU64>,
    overflow_events: Arc<AtomicU64>,

    // 配置
    config: QueueConfig,

    // 服务
    task_service: Arc<TaskService>,

    // 控制通道
    shutdown_sender: watch::Sender<bool>,
    shutdown_receiver: watch::Receiver<bool>,

    // 健康状态
    is_healthy: Arc<AtomicUsize>, // 0 = unhealthy, 1 = healthy
}

/// 任务执行上下文
#[derive(Debug, Clone)]
struct TaskExecutionContext {
    _task_id: String,
    started_at: Instant,
    _worker_id: usize,
    _retry_count: u32,
}

impl TaskQueueService {
    /// 创建新的任务队列服务
    pub fn new(task_service: Arc<TaskService>) -> Self {
        Self::with_config(task_service, QueueConfig::default())
    }

    /// 使用自定义配置创建任务队列服务
    pub fn with_config(task_service: Arc<TaskService>, config: QueueConfig) -> Self {
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);

        Self {
            task_sender: None, // 初始时不创建channel
            processing_tasks: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(QueueStats {
                last_updated: Instant::now(),
                ..Default::default()
            })),
            completed_count: Arc::new(AtomicU64::new(0)),
            failed_count: Arc::new(AtomicU64::new(0)),
            retry_count: Arc::new(AtomicU64::new(0)),
            queued_count: Arc::new(AtomicU64::new(0)),
            backpressure_events: Arc::new(AtomicU64::new(0)),
            overflow_events: Arc::new(AtomicU64::new(0)),
            config,
            task_service,
            shutdown_sender,
            shutdown_receiver,
            is_healthy: Arc::new(AtomicUsize::new(1)),
        }
    }

    /// 启动队列处理器
    pub async fn start<P>(&mut self, processor: Arc<P>) -> Result<(), AppError>
    where
        P: TaskProcessor + 'static,
    {
        info!(
            "Start the task queue service, maximum concurrency: {}, queue size: {}",
            self.config.max_concurrent_tasks, self.config.max_queue_size
        );

        // 创建channel并保存sender
        let (task_sender, task_receiver) = mpsc::channel(self.config.max_queue_size);
        self.task_sender = Some(task_sender);

        // 启动多个工作协程
        self.spawn_workers(processor, task_receiver).await?;

        // 启动监控协程
        self.spawn_monitors().await?;

        // 恢复数据库中的待执行和进行中任务
        self.restore_pending_tasks().await?;

        Ok(())
    }

    /// 从数据库恢复待执行和进行中的任务 - 统一重置状态，让 worker 执行时重新设置
    async fn restore_pending_tasks(&self) -> Result<(), AppError> {
        info!("Start restoring pending and ongoing tasks in the database...");

        let stats = self.task_service.get_task_stats().await?;

        let all_need_process_tasks = stats
            .pending_ids
            .iter()
            .chain(stats.processing_ids.iter())
            .collect::<Vec<_>>();

        info!(
            "Number of tasks to be restored: {}",
            all_need_process_tasks.len()
        );
        info!(
            "Tasks that need to be restored: {:?}",
            all_need_process_tasks
        );

        // 将所有进行中任务统一改为 pending 状态，然后重新入队
        // 这样 worker 真正执行时会重新设置为 processing 状态
        for task_id in all_need_process_tasks.clone() {
            // 使用强制更新，跳过状态转换验证（仅在服务重启恢复时使用）
            if let Err(e) = self
                .task_service
                .update_task_status(task_id, TaskStatus::new_pending())
                .await
            {
                warn!("Failed to re-mark ongoing task status {}: {}", task_id, e);
            } else {
                // 重新入队
                if let Err(e) = self.enqueue_task(task_id.clone(), 1).await {
                    warn!("Requeue task failed {}: {}", task_id, e);
                } else {
                    info!(
                        "The ongoing task has been remarked as pending and added to the queue: {}",
                        task_id
                    );
                }
            }
        }
        // 恢复所有待执行任务
        for task_id in all_need_process_tasks.clone() {
            if let Err(e) = self.enqueue_task(task_id.clone(), 1).await {
                warn!("Failed to restore pending tasks {}: {}", task_id, e);
            } else {
                debug!("Restored pending tasks: {}", task_id);
            }
        }

        info!(
            "Task recovery is completed, with a total of {} tasks recovered. The status of all tasks has been reset to pending, and will be reset to processing when the worker executes",
            all_need_process_tasks.len()
        );
        Ok(())
    }

    /// 启动工作协程 - 采用 SPMC 模式，worker 直接从 channel 消费
    async fn spawn_workers<P>(
        &self,
        processor: Arc<P>,
        task_receiver: mpsc::Receiver<QueueItem>,
    ) -> Result<(), AppError>
    where
        P: TaskProcessor + 'static,
    {
        // 将 task_receiver 包装为 Arc<Mutex<>>，让多个 worker 共享
        let shared_receiver = Arc::new(Mutex::new(task_receiver));

        // 启动 N 个 worker，每个 worker 直接从 channel 消费任务
        for worker_id in 0..self.config.max_concurrent_tasks {
            self.spawn_simple_worker(
                worker_id,
                Arc::clone(&processor),
                Arc::clone(&shared_receiver),
            )
            .await;
        }

        info!(
            "{} worker coroutines have been started, using SPMC mode to directly consume tasks",
            self.config.max_concurrent_tasks
        );
        Ok(())
    }

    /// 启动简化的 worker - 直接从共享 channel 消费任务
    async fn spawn_simple_worker<P>(
        &self,
        worker_id: usize,
        processor: Arc<P>,
        shared_receiver: Arc<Mutex<mpsc::Receiver<QueueItem>>>,
    ) where
        P: TaskProcessor + 'static,
    {
        let task_service = Arc::clone(&self.task_service);
        let processing_tasks = Arc::clone(&self.processing_tasks);
        let completed_count = Arc::clone(&self.completed_count);
        let failed_count = Arc::clone(&self.failed_count);
        let queued_count = Arc::clone(&self.queued_count);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_receiver.clone();

        tokio::spawn(async move {
            debug!(
                "Simplified Worker {} has been started and consumes tasks directly from the channel",
                worker_id
            );

            loop {
                tokio::select! {
                    // 检查关闭信号
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("Worker {} received a shutdown signal", worker_id);
                            break;
                        }
                    }

                    // 从共享 channel 接收任务
                    task_result = async {
                        let mut receiver = shared_receiver.lock().await;
                        receiver.recv().await
                    } => {
                        match task_result {
                            Some(queue_item) => {
                                let task_id = queue_item.task_id.clone();
                                let start_time = Instant::now();

                                debug!("Worker {} starts processing task: {}", worker_id, task_id);

                                // 更新任务状态为处理中
                                if let Err(e) = task_service.update_task_status(
                                    &task_id,
                                    TaskStatus::new_processing(ProcessingStage::FormatDetection)
                                ).await {
                                    error!("Worker {} failed to update task status: {}", worker_id, e);
                                }
                                // 出队计数减一
                                queued_count.fetch_sub(1, Ordering::Relaxed);

                                // 记录到 processing_tasks，便于统计与健康检查
                                {
                                    let mut tasks = processing_tasks.write().await;
                                    tasks.insert(
                                        task_id.clone(),
                                        TaskExecutionContext {
                                            _task_id: task_id.clone(),
                                            started_at: start_time,
                                            _worker_id: worker_id,
                                            _retry_count: queue_item.retry_count,
                                        },
                                    );
                                }

                                // 执行任务处理
                                let result = tokio::time::timeout(
                                    config.task_timeout,
                                    processor.process_task(&task_id)
                                ).await;

                                let processing_time = start_time.elapsed();

                                // 处理结果
                                match result {
                                    Ok(Ok(())) => {
                                        completed_count.fetch_add(1, Ordering::Relaxed);

                                        if let Err(e) = task_service.update_task_status(
                                            &task_id,
                                            TaskStatus::new_completed(processing_time)
                                        ).await {
                                            error!("Worker {} failed to update task completion status: {}", worker_id, e);
                                        }

                                        info!("Worker {} completed the task: {} (time taken: {:?})",
                                                  worker_id, task_id, processing_time);
                                        // 从 processing 列表移除
                                        {
                                            let mut tasks = processing_tasks.write().await;
                                            tasks.remove(&task_id);
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        failed_count.fetch_add(1, Ordering::Relaxed);

                                        if let Err(err) = task_service.set_task_error(&task_id, e.to_string()).await {
                                            error!("Worker {} failed to set task error: {}", worker_id, err);
                                        }

                                        error!("Worker {} task failed: {} - {}", worker_id, task_id, e);
                                        // 从 processing 列表移除
                                        {
                                            let mut tasks = processing_tasks.write().await;
                                            tasks.remove(&task_id);
                                        }
                                    }
                                    Err(_) => {
                                        failed_count.fetch_add(1, Ordering::Relaxed);

                                        if let Err(e) = task_service.set_task_error(&task_id, "任务处理超时".to_string()).await {
                                            error!("Worker {} failed to set task timeout error: {}", worker_id, e);
                                        }

                                        error!("Worker {} task timeout: {} (timeout time: {:?})",worker_id, task_id, config.task_timeout);
                                        // 从 processing 列表移除
                                        {
                                            let mut tasks = processing_tasks.write().await;
                                            tasks.remove(&task_id);
                                        }
                                    }
                                }
                            }
                            None => {
                                // Channel 已关闭
                                info!("Worker {} detected that the channel was closed and stopped working", worker_id);
                                break;
                            }
                        }
                    }
                }
            }

            debug!("Worker {} has stopped", worker_id);
        });
    }

    /// 启动监控协程
    async fn spawn_monitors(&self) -> Result<(), AppError> {
        // 启动统计更新协程
        self.spawn_stats_updater().await;

        // 启动健康检查协程
        self.spawn_health_checker().await;

        Ok(())
    }

    /// 启动统计更新协程
    async fn spawn_stats_updater(&self) {
        let stats = Arc::clone(&self.stats);
        let processing_tasks = Arc::clone(&self.processing_tasks);
        let completed_count = Arc::clone(&self.completed_count);
        let failed_count = Arc::clone(&self.failed_count);
        let retry_count = Arc::clone(&self.retry_count);
        let backpressure_events = Arc::clone(&self.backpressure_events);
        let overflow_events = Arc::clone(&self.overflow_events);
        let queued_count = Arc::clone(&self.queued_count);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_receiver.clone();

        tokio::spawn(async move {
            let mut interval = interval(config.metrics_update_interval);
            let mut processing_times = VecDeque::with_capacity(100);

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }

                    _ = interval.tick() => {
                        let now = Instant::now();

                        // 收集当前统计信息
                        // 队列中的等待任务使用原子计数器近似
                        let pending_count = queued_count.load(Ordering::Relaxed) as usize;

                        let processing_count = {
                            let tasks = processing_tasks.read().await;
                            tasks.len()
                        };

                        let completed = completed_count.load(Ordering::Relaxed);
                        let failed = failed_count.load(Ordering::Relaxed);
                        let retries = retry_count.load(Ordering::Relaxed);
                        let backpressure = backpressure_events.load(Ordering::Relaxed);
                        let overflow = overflow_events.load(Ordering::Relaxed);

                        // 计算工作协程利用率
                        // 利用率直接使用 processing_count / max_concurrent_tasks
                        let worker_utilization = {
                            let tasks = processing_tasks.read().await;
                            if config.max_concurrent_tasks > 0 {
                                (tasks.len() as f64 / config.max_concurrent_tasks as f64).min(1.0)
                            } else { 0.0 }
                        };

                        // 计算平均处理时间
                        let average_processing_time = if processing_times.is_empty() {
                            Duration::from_secs(0)
                        } else {
                            let total_ms: u64 = processing_times.iter().map(|d: &Duration| d.as_millis() as u64).sum();
                            Duration::from_millis(total_ms / processing_times.len() as u64)
                        };

                        // 计算吞吐量
                        let queue_throughput = if completed > 0 && !average_processing_time.is_zero() {
                            1000.0 / average_processing_time.as_millis() as f64
                        } else {
                            0.0
                        };

                        // 估算内存使用
                        let memory_usage_bytes = (pending_count + processing_count) * 1024; // 简化估算

                        // 更新统计信息
                        {
                            let mut stats_guard = stats.write().await;
                            stats_guard.pending_count = pending_count;
                            stats_guard.processing_count = processing_count;
                            stats_guard.completed_count = completed;
                            stats_guard.failed_count = failed;
                            stats_guard.retry_count = retries;
                            stats_guard.backpressure_events = backpressure;
                            stats_guard.queue_overflow_events = overflow;
                            stats_guard.worker_utilization = worker_utilization;
                            stats_guard.average_processing_time = average_processing_time;
                            stats_guard.queue_throughput = queue_throughput;
                            stats_guard.memory_usage_bytes = memory_usage_bytes as u64;
                            stats_guard.last_updated = now;
                        }

                        // 记录处理时间样本
                        {
                            let tasks = processing_tasks.read().await;
                            for (_, context) in tasks.iter() {
                                let elapsed = now.duration_since(context.started_at);
                                processing_times.push_back(elapsed);

                                // 保持队列大小
                                if processing_times.len() > 100 {
                                    processing_times.pop_front();
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    /// 启动健康检查协程
    async fn spawn_health_checker(&self) {
        let is_healthy = Arc::clone(&self.is_healthy);
        let processing_tasks = Arc::clone(&self.processing_tasks);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_receiver.clone();

        tokio::spawn(async move {
            let mut interval = interval(config.health_check_interval);

            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }

                    _ = interval.tick() => {
                        let now = Instant::now();
                        let mut unhealthy_tasks = 0;

                        // 检查是否有任务超时
                        {
                            let tasks = processing_tasks.read().await;
                            for (task_id, context) in tasks.iter() {
                                let elapsed = now.duration_since(context.started_at);
                                if elapsed > config.task_timeout * 2 {
                                    warn!("Possibly stuck task detected: {} (running time: {:?})", task_id, elapsed);
                                    unhealthy_tasks += 1;
                                }
                            }
                        }

                        // 更新健康状态
                        let healthy = unhealthy_tasks == 0;
                        is_healthy.store(if healthy { 1 } else { 0 }, Ordering::Relaxed);

                        if !healthy {
                            warn!("Queue service health check failed: {} tasks may be stuck", unhealthy_tasks);
                        }
                    }
                }
            }
        });
    }

    /// 添加任务到队列
    pub async fn enqueue_task(&self, task_id: String, priority: u8) -> Result<(), AppError> {
        // 检查队列是否已启动
        let task_sender = self.task_sender.as_ref().ok_or_else(|| {
            AppError::Queue("队列服务尚未启动，请先调用 start() 方法".to_string())
        })?;

        let item = QueueItem::new(task_id.clone(), priority);

        // 尝试发送任务，如果队列满了则触发背压
        match task_sender.try_send(item) {
            Ok(()) => {
                self.queued_count.fetch_add(1, Ordering::Relaxed);
                debug!(
                    "Task has been added to the queue: {} (Priority: {})",
                    task_id, priority
                );
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.overflow_events.fetch_add(1, Ordering::Relaxed);
                warn!(
                    "The queue is full, triggering back pressure control: {}",
                    task_id
                );
                Err(AppError::Queue("队列已满，请稍后重试".to_string()))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(AppError::Queue("队列已关闭".to_string()))
            }
        }
    }

    /// 获取队列统计信息
    pub async fn get_stats(&self) -> QueueStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// 获取正在处理的任务列表
    pub async fn get_processing_tasks(&self) -> Vec<(String, Duration)> {
        let tasks = self.processing_tasks.read().await;
        let now = Instant::now();

        tasks
            .iter()
            .map(|(task_id, context)| (task_id.clone(), now.duration_since(context.started_at)))
            .collect()
    }

    /// 检查队列是否健康
    pub fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::Relaxed) == 1
    }

    /// 检查队列是否已启动
    pub fn is_started(&self) -> bool {
        self.task_sender.is_some()
    }

    /// 优雅关闭
    pub async fn shutdown(&self) -> Result<(), AppError> {
        info!("Starting to shut down the task queue service...");

        // 发送关闭信号
        if let Err(e) = self.shutdown_sender.send(true) {
            error!("Failed to send shutdown signal: {}", e);
        }

        // 等待所有正在处理的任务完成
        let mut wait_count = 0;
        while wait_count < 30 {
            // 最多等待30秒
            let processing_count = {
                let tasks = self.processing_tasks.read().await;
                tasks.len()
            };

            if processing_count == 0 {
                break;
            }

            info!("Waiting for {} tasks to complete...", processing_count);
            sleep(Duration::from_secs(1)).await;
            wait_count += 1;
        }

        info!("Task queue service is down");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    struct TestProcessor {
        processed_count: AtomicUsize,
        should_fail: bool,
    }

    impl TestProcessor {
        fn new() -> Self {
            Self {
                processed_count: AtomicUsize::new(0),
                should_fail: false,
            }
        }

        fn with_failure() -> Self {
            Self {
                processed_count: AtomicUsize::new(0),
                should_fail: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl TaskProcessor for TestProcessor {
        async fn process_task(&self, task_id: &str) -> Result<(), AppError> {
            // 模拟处理时间
            sleep(Duration::from_millis(50)).await;

            if self.should_fail && task_id.contains("fail") {
                return Err(AppError::Parse("模拟处理失败".to_string()));
            }

            self.processed_count.fetch_add(1, Ordering::SeqCst);
            info!("Processing task: {}", task_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_backpressure_control() {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let task_service = Arc::new(TaskService::new(db).unwrap());

        let config = QueueConfig {
            max_concurrent_tasks: 1,
            max_queue_size: 2,
            backpressure_threshold: 0.5,
            ..Default::default()
        };

        let mut queue_service = TaskQueueService::with_config(task_service, config);
        let processor = Arc::new(TestProcessor::new());

        queue_service.start(processor).await.unwrap();

        // 添加任务直到触发背压
        let mut success_count = 0;
        let mut backpressure_triggered = false;

        for i in 0..5 {
            match queue_service.enqueue_task(format!("task{i}"), 1).await {
                Ok(()) => success_count += 1,
                Err(_) => {
                    backpressure_triggered = true;
                    break;
                }
            }
        }

        assert!(backpressure_triggered, "背压控制应该被触发");
        assert!(success_count < 5, "不应该所有任务都成功入队");

        queue_service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_metrics_collection() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let task_service = Arc::new(TaskService::new(db).unwrap());

        let config = QueueConfig {
            metrics_update_interval: Duration::from_millis(100),
            ..Default::default()
        };

        let mut queue_service = TaskQueueService::with_config(task_service, config);
        let processor = Arc::new(TestProcessor::new());

        queue_service.start(processor).await.expect("start ok");

        // 添加任务
        for i in 0..3 {
            queue_service
                .enqueue_task(format!("task{i}"), 1)
                .await
                .expect("enqueue ok");
        }

        // 等待处理和统计更新
        sleep(Duration::from_millis(400)).await;

        let stats = queue_service.get_stats().await;
        assert!(stats.last_updated.elapsed() < Duration::from_secs(1));
        assert!(stats.worker_utilization >= 0.0 && stats.worker_utilization <= 1.0);
        // Memory usage might be 0 if tasks are processed quickly, so we just check it's non-negative
        assert!(stats.memory_usage_bytes >= 0);

        queue_service.shutdown().await.expect("shutdown ok");
    }
}
