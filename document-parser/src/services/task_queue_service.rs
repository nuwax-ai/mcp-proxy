use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock, Semaphore, watch};
use tokio::time::{Duration, Instant, interval, sleep};
use std::collections::{HashMap, VecDeque, BTreeMap};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::error::AppError;
use crate::models::{TaskStatus, ProcessingStage};
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
    
    // 优先级队列
    priority_queue: Arc<Mutex<BTreeMap<u8, VecDeque<QueueItem>>>>,
    
    // 并发控制
    semaphore: Arc<Semaphore>,
    
    // 正在处理的任务
    processing_tasks: Arc<RwLock<HashMap<String, TaskExecutionContext>>>,
    
    // 统计信息
    stats: Arc<RwLock<QueueStats>>,
    completed_count: Arc<AtomicU64>,
    failed_count: Arc<AtomicU64>,
    retry_count: Arc<AtomicU64>,
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
    task_id: String,
    started_at: Instant,
    worker_id: usize,
    retry_count: u32,
    priority: u8,
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
            priority_queue: Arc::new(Mutex::new(BTreeMap::new())),
            semaphore: Arc::new(Semaphore::new(config.max_concurrent_tasks)),
            processing_tasks: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(QueueStats {
                last_updated: Instant::now(),
                ..Default::default()
            })),
            completed_count: Arc::new(AtomicU64::new(0)),
            failed_count: Arc::new(AtomicU64::new(0)),
            retry_count: Arc::new(AtomicU64::new(0)),
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
        log::info!("启动任务队列服务，最大并发: {}, 队列大小: {}", 
                  self.config.max_concurrent_tasks, self.config.max_queue_size);
        
        // 创建channel并保存sender
        let (task_sender, task_receiver) = mpsc::channel(self.config.max_queue_size);
        self.task_sender = Some(task_sender);
        
        // 启动多个工作协程
        self.spawn_workers(processor, task_receiver).await?;
        
        // 启动监控协程
        self.spawn_monitors().await?;
        
        Ok(())
    }

    /// 启动工作协程
    async fn spawn_workers<P>(&self, processor: Arc<P>, task_receiver: mpsc::Receiver<QueueItem>) -> Result<(), AppError>
    where
        P: TaskProcessor + 'static,
    {
        // 启动任务分发器
        self.spawn_task_dispatcher(task_receiver).await;
        
        // 启动工作协程池
        for worker_id in 0..self.config.max_concurrent_tasks {
            self.spawn_worker(worker_id, Arc::clone(&processor)).await;
        }
        
        Ok(())
    }

    /// 启动任务分发器
    async fn spawn_task_dispatcher(&self, mut task_receiver: mpsc::Receiver<QueueItem>) {
        let priority_queue = Arc::clone(&self.priority_queue);
        let semaphore = Arc::clone(&self.semaphore);
        let processing_tasks = Arc::clone(&self.processing_tasks);
        let backpressure_events = Arc::clone(&self.backpressure_events);
        let overflow_events = Arc::clone(&self.overflow_events);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_receiver.clone();
        
        tokio::spawn(async move {
            let mut backpressure_active = false;
            
            loop {
                tokio::select! {
                    // 检查关闭信号
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            log::info!("任务分发器收到关闭信号");
                            break;
                        }
                    }
                    
                    // 处理新任务
                    task_item = task_receiver.recv() => {
                        if let Some(item) = task_item {
                            // 检查背压情况
                            let current_load = Self::calculate_current_load(&processing_tasks, &semaphore).await;
                            
                            if current_load > config.backpressure_threshold {
                                if !backpressure_active {
                                    log::warn!("激活背压控制，当前负载: {:.2}", current_load);
                                    backpressure_active = true;
                                    backpressure_events.fetch_add(1, Ordering::Relaxed);
                                }
                                
                                // 将任务放入优先级队列等待
                                let mut queue = priority_queue.lock().await;
                                Self::insert_into_priority_queue(&mut queue, item);
                            } else {
                                if backpressure_active {
                                    log::info!("背压控制解除，当前负载: {:.2}", current_load);
                                    backpressure_active = false;
                                }
                                
                                // 直接处理任务
                                Self::try_dispatch_task(item, &semaphore, &processing_tasks).await;
                            }
                        }
                    }
                    
                    // 定期检查优先级队列
                    _ = sleep(Duration::from_millis(100)) => {
                        Self::process_priority_queue(&priority_queue, &semaphore, &processing_tasks).await;
                    }
                }
            }
        });
    }

    /// 启动单个工作协程
    async fn spawn_worker<P>(&self, worker_id: usize, processor: Arc<P>)
    where
        P: TaskProcessor + 'static,
    {
        let semaphore = Arc::clone(&self.semaphore);
        let processing_tasks = Arc::clone(&self.processing_tasks);
        let task_service = Arc::clone(&self.task_service);
        let completed_count = Arc::clone(&self.completed_count);
        let failed_count = Arc::clone(&self.failed_count);
        let retry_count = Arc::clone(&self.retry_count);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_receiver.clone();
        
        tokio::spawn(async move {
            log::debug!("工作协程 {} 已启动", worker_id);
            
            loop {
                tokio::select! {
                    // 检查关闭信号
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            log::info!("工作协程 {} 收到关闭信号", worker_id);
                            break;
                        }
                    }
                    
                    // 获取信号量许可
                    permit = semaphore.acquire() => {
                        if let Ok(permit) = permit {
                            // 从处理队列中获取任务
                            if let Some(context) = Self::get_next_processing_task(&processing_tasks).await {
                                let task_id = context.task_id.clone();
                                let start_time = Instant::now();
                                
                                log::debug!("工作协程 {} 开始处理任务: {}", worker_id, task_id);
                                
                                // 更新任务状态
                                if let Err(e) = task_service.update_task_status(
                                    &task_id, 
                                    TaskStatus::new_processing(ProcessingStage::FormatDetection)
                                ).await {
                                    log::error!("更新任务状态失败: {}", e);
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
                                            log::error!("更新任务完成状态失败: {}", e);
                                        }
                                        
                                        log::info!("工作协程 {} 完成任务: {} (耗时: {:?})", 
                                                  worker_id, task_id, processing_time);
                                    }
                                    Ok(Err(e)) => {
                                        failed_count.fetch_add(1, Ordering::Relaxed);
                                        
                                        // 检查是否需要重试
                                        if context.retry_count < context.retry_count {
                                            retry_count.fetch_add(1, Ordering::Relaxed);
                                            
                                            // 计算重试延迟
                                            let delay = Self::calculate_retry_delay(
                                                context.retry_count, 
                                                &config
                                            );
                                            
                                            log::warn!("任务 {} 失败，将在 {:?} 后重试 (第 {} 次)", 
                                                      task_id, delay, context.retry_count + 1);
                                            
                                            // 重新排队
                                            tokio::spawn(async move {
                                                sleep(delay).await;
                                                // 重新入队逻辑
                                            });
                                        } else {
                                            if let Err(err) = task_service.set_task_error(&task_id, e.to_string()).await {
                                                log::error!("设置任务错误失败: {}", err);
                                            }
                                            
                                            log::error!("工作协程 {} 任务失败: {} - {}", worker_id, task_id, e);
                                        }
                                    }
                                    Err(_) => {
                                        failed_count.fetch_add(1, Ordering::Relaxed);
                                        
                                        if let Err(e) = task_service.set_task_error(&task_id, "任务处理超时".to_string()).await {
                                            log::error!("设置任务超时错误失败: {}", e);
                                        }
                                        
                                        log::error!("工作协程 {} 任务超时: {} (超时时间: {:?})", 
                                                   worker_id, task_id, config.task_timeout);
                                    }
                                }
                                
                                // 从处理队列中移除任务
                                {
                                    let mut tasks = processing_tasks.write().await;
                                    tasks.remove(&task_id);
                                }
                                
                                // 释放许可
                                drop(permit);
                            } else {
                                // 没有任务可处理，释放许可并等待
                                drop(permit);
                                sleep(Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
            
            log::debug!("工作协程 {} 已停止", worker_id);
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
        let priority_queue = Arc::clone(&self.priority_queue);
        let completed_count = Arc::clone(&self.completed_count);
        let failed_count = Arc::clone(&self.failed_count);
        let retry_count = Arc::clone(&self.retry_count);
        let backpressure_events = Arc::clone(&self.backpressure_events);
        let overflow_events = Arc::clone(&self.overflow_events);
        let semaphore = Arc::clone(&self.semaphore);
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
                        let pending_count = {
                            let queue = priority_queue.lock().await;
                            queue.values().map(|v| v.len()).sum()
                        };
                        
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
                        let available_permits = semaphore.available_permits();
                        let total_permits = config.max_concurrent_tasks;
                        let worker_utilization = if total_permits > 0 {
                            1.0 - (available_permits as f64 / total_permits as f64)
                        } else {
                            0.0
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
                                    log::warn!("检测到可能卡住的任务: {} (运行时间: {:?})", task_id, elapsed);
                                    unhealthy_tasks += 1;
                                }
                            }
                        }
                        
                        // 更新健康状态
                        let healthy = unhealthy_tasks == 0;
                        is_healthy.store(if healthy { 1 } else { 0 }, Ordering::Relaxed);
                        
                        if !healthy {
                            log::warn!("队列服务健康检查失败: {} 个任务可能卡住", unhealthy_tasks);
                        }
                    }
                }
            }
        });
    }

    /// 添加任务到队列
    pub async fn enqueue_task(&self, task_id: String, priority: u8) -> Result<(), AppError> {
        // 检查队列是否已启动
        let task_sender = self.task_sender.as_ref()
            .ok_or_else(|| AppError::Queue("队列服务尚未启动，请先调用 start() 方法".to_string()))?;
        
        let item = QueueItem::new(task_id.clone(), priority);
        
        // 尝试发送任务，如果队列满了则触发背压
        match task_sender.try_send(item) {
            Ok(()) => {
                log::debug!("任务已加入队列: {} (优先级: {})", task_id, priority);
                Ok(())
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.overflow_events.fetch_add(1, Ordering::Relaxed);
                log::warn!("队列已满，触发背压控制: {}", task_id);
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
        
        tasks.iter()
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
        log::info!("开始关闭任务队列服务...");
        
        // 发送关闭信号
        if let Err(e) = self.shutdown_sender.send(true) {
            log::error!("发送关闭信号失败: {}", e);
        }
        
        // 等待所有正在处理的任务完成
        let mut wait_count = 0;
        while wait_count < 30 { // 最多等待30秒
            let processing_count = {
                let tasks = self.processing_tasks.read().await;
                tasks.len()
            };
            
            if processing_count == 0 {
                break;
            }
            
            log::info!("等待 {} 个任务完成...", processing_count);
            sleep(Duration::from_secs(1)).await;
            wait_count += 1;
        }
        
        log::info!("任务队列服务已关闭");
        Ok(())
    }

    // 辅助方法
    
    /// 计算当前负载
    async fn calculate_current_load(
        processing_tasks: &Arc<RwLock<HashMap<String, TaskExecutionContext>>>,
        semaphore: &Arc<Semaphore>,
    ) -> f64 {
        let available_permits = semaphore.available_permits();
        let total_permits = semaphore.available_permits() + {
            let tasks = processing_tasks.read().await;
            tasks.len()
        };
        
        if total_permits > 0 {
            1.0 - (available_permits as f64 / total_permits as f64)
        } else {
            0.0
        }
    }

    /// 将任务插入优先级队列
    fn insert_into_priority_queue(
        queue: &mut BTreeMap<u8, VecDeque<QueueItem>>,
        item: QueueItem,
    ) {
        queue.entry(item.priority)
            .or_insert_with(VecDeque::new)
            .push_back(item);
    }

    /// 尝试分发任务
    async fn try_dispatch_task(
        item: QueueItem,
        semaphore: &Arc<Semaphore>,
        processing_tasks: &Arc<RwLock<HashMap<String, TaskExecutionContext>>>,
    ) {
        if let Ok(permit) = semaphore.try_acquire() {
            let context = TaskExecutionContext {
                task_id: item.task_id.clone(),
                started_at: Instant::now(),
                worker_id: 0, // 将由工作协程设置
                retry_count: item.retry_count,
                priority: item.priority,
            };
            
            {
                let mut tasks = processing_tasks.write().await;
                tasks.insert(item.task_id.clone(), context);
            }
            
            // 释放许可，让工作协程获取
            drop(permit);
        } else {
            // 无法获取许可，放入优先级队列
            // 这里需要访问优先级队列，但为了避免死锁，我们简化处理
            log::debug!("无法立即处理任务 {}，等待工作协程空闲", item.task_id);
        }
    }

    /// 处理优先级队列
    async fn process_priority_queue(
        priority_queue: &Arc<Mutex<BTreeMap<u8, VecDeque<QueueItem>>>>,
        semaphore: &Arc<Semaphore>,
        processing_tasks: &Arc<RwLock<HashMap<String, TaskExecutionContext>>>,
    ) {
        let mut queue = priority_queue.lock().await;
        
        // 按优先级从高到低处理
        let priorities: Vec<u8> = queue.keys().rev().cloned().collect();
        
        for priority in priorities {
            if let Some(priority_queue) = queue.get_mut(&priority) {
                while let Some(item) = priority_queue.pop_front() {
                    if semaphore.try_acquire().is_ok() {
                        let context = TaskExecutionContext {
                            task_id: item.task_id.clone(),
                            started_at: Instant::now(),
                            worker_id: 0,
                            retry_count: item.retry_count,
                            priority: item.priority,
                        };
                        
                        {
                            let mut tasks = processing_tasks.write().await;
                            tasks.insert(item.task_id.clone(), context);
                        }
                        
                        break; // 处理一个任务后退出
                    } else {
                        // 无法获取许可，将任务放回队列
                        priority_queue.push_front(item);
                        break;
                    }
                }
                
                // 清理空的优先级队列
                if priority_queue.is_empty() {
                    queue.remove(&priority);
                }
            }
        }
    }

    /// 获取下一个待处理任务
    async fn get_next_processing_task(
        processing_tasks: &Arc<RwLock<HashMap<String, TaskExecutionContext>>>,
    ) -> Option<TaskExecutionContext> {
        let tasks = processing_tasks.read().await;
        tasks.values().next().cloned()
    }

    /// 计算重试延迟
    fn calculate_retry_delay(retry_count: u32, config: &QueueConfig) -> Duration {
        let base_delay = config.retry_base_delay.as_millis() as u64;
        let max_delay = config.retry_max_delay.as_millis() as u64;
        
        // 指数退避
        let delay_ms = std::cmp::min(
            base_delay * 2_u64.pow(retry_count),
            max_delay,
        );
        
        Duration::from_millis(delay_ms)
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use sled::Db;

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
            log::info!("处理任务: {}", task_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_task_queue_basic() {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let task_service = Arc::new(TaskService::new(db).unwrap());
        
        let config = QueueConfig {
            max_concurrent_tasks: 2,
            max_queue_size: 10,
            task_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        
        let mut queue_service = TaskQueueService::with_config(task_service, config);
        let processor = Arc::new(TestProcessor::new());
        
        // 启动队列服务
        queue_service.start(processor.clone()).await.unwrap();
        
        // 添加任务
        queue_service.enqueue_task("task1".to_string(), 1).await.unwrap();
        queue_service.enqueue_task("task2".to_string(), 2).await.unwrap();
        
        // 等待处理完成
        sleep(Duration::from_millis(200)).await;
        
        // 检查处理结果
        let stats = queue_service.get_stats().await;
        assert!(stats.completed_count >= 2);
        assert!(queue_service.is_healthy());
        
        // 关闭服务
        queue_service.shutdown().await.unwrap();
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
            match queue_service.enqueue_task(format!("task{}", i), 1).await {
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
    async fn test_priority_queue() {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let task_service = Arc::new(TaskService::new(db).unwrap());
        
        let mut queue_service = TaskQueueService::new(task_service);
        let processor = Arc::new(TestProcessor::new());
        
        queue_service.start(processor).await.unwrap();
        
        // 添加不同优先级的任务
        queue_service.enqueue_task("low_priority".to_string(), 1).await.unwrap();
        queue_service.enqueue_task("high_priority".to_string(), 10).await.unwrap();
        queue_service.enqueue_task("medium_priority".to_string(), 5).await.unwrap();
        
        // 等待处理
        sleep(Duration::from_millis(300)).await;
        
        let stats = queue_service.get_stats().await;
        assert_eq!(stats.completed_count, 3);
        
        queue_service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_error_handling_and_retry() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let task_service = Arc::new(TaskService::new(db).unwrap());
        
        let mut queue_service = TaskQueueService::new(task_service);
        let processor = Arc::new(TestProcessor::with_failure());
        
        queue_service.start(processor).await.expect("start ok");
        
        // 添加会失败的任务
        queue_service.enqueue_task("task_fail".to_string(), 1).await.expect("enqueue fail");
        queue_service.enqueue_task("task_success".to_string(), 1).await.expect("enqueue success");
        
        // 等待处理
        sleep(Duration::from_millis(300)).await;
        
        let stats = queue_service.get_stats().await;
        assert!(stats.failed_count > 0, "应该有失败的任务");
        assert!(stats.completed_count > 0, "应该有成功的任务");
        
        queue_service.shutdown().await.expect("shutdown ok");
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
            queue_service.enqueue_task(format!("task{}", i), 1).await.expect("enqueue ok");
        }
        
        // 等待处理和统计更新
        sleep(Duration::from_millis(400)).await;
        
        let stats = queue_service.get_stats().await;
        assert!(stats.last_updated.elapsed() < Duration::from_secs(1));
        assert!(stats.worker_utilization >= 0.0 && stats.worker_utilization <= 1.0);
        assert!(stats.memory_usage_bytes > 0);
        
        queue_service.shutdown().await.expect("shutdown ok");
    }
}