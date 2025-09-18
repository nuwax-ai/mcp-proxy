//! 性能指标收集器
//!
//! 提供实时性能监控、指标聚合和报告生成功能

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, RwLock};
use tokio::time::interval;
use uuid::Uuid;

use super::{MetricsConfig, PerformanceOptimizable};
use crate::config::AppConfig;
use crate::error::AppError;

/// 性能指标收集器
pub struct MetricsCollector {
    config: MetricsConfig,
    system_metrics: Arc<SystemMetrics>,
    application_metrics: Arc<ApplicationMetrics>,
    custom_metrics: Arc<CustomMetrics>,
    aggregator: Arc<MetricsAggregator>,
    reporter: Arc<MetricsReporter>,
    is_collecting: Arc<AtomicBool>,
}

impl MetricsCollector {
    /// 创建新的指标收集器
    pub async fn new(config: &AppConfig) -> Result<Self, AppError> {
        let metrics_config = MetricsConfig::default(); // 从配置中获取

        let system_metrics = Arc::new(SystemMetrics::new());
        let application_metrics = Arc::new(ApplicationMetrics::new());
        let custom_metrics = Arc::new(CustomMetrics::new());
        let aggregator = Arc::new(MetricsAggregator::new(metrics_config.aggregation_window));
        let reporter = Arc::new(MetricsReporter::new(metrics_config.clone()).await?);

        let collector = Self {
            config: metrics_config,
            system_metrics,
            application_metrics,
            custom_metrics,
            aggregator,
            reporter,
            is_collecting: Arc::new(AtomicBool::new(false)),
        };

        Ok(collector)
    }

    /// 开始收集指标
    pub async fn start_collection(&self) -> Result<(), AppError> {
        if self.is_collecting.swap(true, Ordering::Relaxed) {
            return Ok(()); // 已经在收集中
        }

        // 启动系统指标收集
        self.start_system_metrics_collection().await;

        // 启动应用指标收集
        self.start_application_metrics_collection().await;

        // 启动指标聚合
        self.start_metrics_aggregation().await;

        // 启动报告生成
        self.start_metrics_reporting().await;

        Ok(())
    }

    /// 停止收集指标
    pub async fn stop_collection(&self) {
        self.is_collecting.store(false, Ordering::Relaxed);
    }

    /// 记录请求指标
    pub async fn record_request(&self, duration: Duration, success: bool) {
        self.application_metrics
            .record_request(duration, success)
            .await;
    }

    /// 记录文档处理指标
    pub async fn record_document_processing(
        &self,
        format: &str,
        size: u64,
        duration: Duration,
        success: bool,
    ) {
        self.application_metrics
            .record_document_processing(format, size, duration, success)
            .await;
    }

    /// 记录缓存指标
    pub async fn record_cache_operation(&self, cache_type: &str, operation: &str, hit: bool) {
        self.application_metrics
            .record_cache_operation(cache_type, operation, hit)
            .await;
    }

    /// 记录错误
    pub async fn record_error(&self, error_type: &str, error_code: &str) {
        self.application_metrics
            .record_error(error_type, error_code)
            .await;
    }

    /// 记录自定义指标
    pub async fn record_custom_metric(
        &self,
        name: &str,
        value: f64,
        tags: HashMap<String, String>,
    ) {
        self.custom_metrics.record_metric(name, value, tags).await;
    }

    /// 增加计数器
    pub async fn increment_counter(&self, name: &str, tags: HashMap<String, String>) {
        self.custom_metrics.increment_counter(name, tags).await;
    }

    /// 记录直方图
    pub async fn record_histogram(&self, name: &str, value: f64, tags: HashMap<String, String>) {
        self.custom_metrics
            .record_histogram(name, value, tags)
            .await;
    }

    /// 设置仪表盘值
    pub async fn set_gauge(&self, name: &str, value: f64, tags: HashMap<String, String>) {
        self.custom_metrics.set_gauge(name, value, tags).await;
    }

    /// 获取当前指标快照
    pub async fn get_metrics_snapshot(&self) -> Result<MetricsSnapshot, AppError> {
        let system_metrics = self.system_metrics.get_snapshot().await;
        let application_metrics = self.application_metrics.get_snapshot().await;
        let custom_metrics = self.custom_metrics.get_snapshot().await;

        Ok(MetricsSnapshot {
            timestamp: SystemTime::now(),
            system_metrics,
            application_metrics,
            custom_metrics,
        })
    }

    /// 获取聚合指标
    pub async fn get_aggregated_metrics(
        &self,
        window: Duration,
    ) -> Result<AggregatedMetrics, AppError> {
        self.aggregator.get_aggregated_metrics(window).await
    }

    /// 生成性能报告
    pub async fn generate_performance_report(
        &self,
        period: Duration,
    ) -> Result<PerformanceReport, AppError> {
        self.reporter.generate_report(period).await
    }

    /// 导出指标
    pub async fn export_metrics(&self, format: ExportFormat) -> Result<String, AppError> {
        let snapshot = self.get_metrics_snapshot().await?;

        match format {
            ExportFormat::Json => Ok(serde_json::to_string_pretty(&snapshot)?),
            ExportFormat::Prometheus => self.export_prometheus_format(&snapshot).await,
            ExportFormat::InfluxDB => self.export_influxdb_format(&snapshot).await,
            ExportFormat::Csv => self.export_csv_format(&snapshot).await,
        }
    }

    /// 设置告警阈值
    pub async fn set_alert_threshold(
        &self,
        metric_name: &str,
        threshold: f64,
        condition: AlertCondition,
    ) -> Result<(), AppError> {
        self.reporter
            .set_alert_threshold(metric_name, threshold, condition)
            .await
    }

    /// 检查告警
    pub async fn check_alerts(&self) -> Result<Vec<Alert>, AppError> {
        self.reporter.check_alerts().await
    }

    // 私有方法

    async fn start_system_metrics_collection(&self) {
        let system_metrics = self.system_metrics.clone();
        let is_collecting = self.is_collecting.clone();
        let interval_duration = self.config.collection_interval;

        tokio::spawn(async move {
            let mut interval = interval(interval_duration);

            while is_collecting.load(Ordering::Relaxed) {
                interval.tick().await;
                system_metrics.collect().await;
            }
        });
    }

    async fn start_application_metrics_collection(&self) {
        let application_metrics = self.application_metrics.clone();
        let is_collecting = self.is_collecting.clone();
        let interval_duration = self.config.collection_interval;

        tokio::spawn(async move {
            let mut interval = interval(interval_duration);

            while is_collecting.load(Ordering::Relaxed) {
                interval.tick().await;
                application_metrics.collect().await;
            }
        });
    }

    async fn start_metrics_aggregation(&self) {
        let aggregator = self.aggregator.clone();
        let system_metrics = self.system_metrics.clone();
        let application_metrics = self.application_metrics.clone();
        let custom_metrics = self.custom_metrics.clone();
        let is_collecting = self.is_collecting.clone();
        let aggregation_interval = self.config.aggregation_interval;

        tokio::spawn(async move {
            let mut interval = interval(aggregation_interval);

            while is_collecting.load(Ordering::Relaxed) {
                interval.tick().await;

                let system_snapshot = system_metrics.get_snapshot().await;
                let app_snapshot = application_metrics.get_snapshot().await;
                let custom_snapshot = custom_metrics.get_snapshot().await;

                aggregator
                    .aggregate_metrics(system_snapshot, app_snapshot, custom_snapshot)
                    .await;
            }
        });
    }

    async fn start_metrics_reporting(&self) {
        let reporter = self.reporter.clone();
        let is_collecting = self.is_collecting.clone();
        let reporting_interval = self.config.reporting_interval;

        tokio::spawn(async move {
            let mut interval = interval(reporting_interval);

            while is_collecting.load(Ordering::Relaxed) {
                interval.tick().await;

                if let Err(e) = reporter.generate_periodic_report().await {
                    eprintln!("Failed to generate periodic report: {e}");
                }
            }
        });
    }

    async fn export_prometheus_format(
        &self,
        snapshot: &MetricsSnapshot,
    ) -> Result<String, AppError> {
        let mut output = String::new();

        // 系统指标
        output.push_str("# HELP system_cpu_usage CPU usage percentage\n");
        output.push_str("# TYPE system_cpu_usage gauge\n");
        output.push_str(&format!(
            "system_cpu_usage {{}} {}\n",
            snapshot.system_metrics.cpu_usage
        ));

        output.push_str("# HELP system_memory_usage Memory usage in bytes\n");
        output.push_str("# TYPE system_memory_usage gauge\n");
        output.push_str(&format!(
            "system_memory_usage {{}} {}\n",
            snapshot.system_metrics.memory_usage
        ));

        // 应用指标
        output.push_str("# HELP app_requests_total Total number of requests\n");
        output.push_str("# TYPE app_requests_total counter\n");
        output.push_str(&format!(
            "app_requests_total {{}} {}\n",
            snapshot.application_metrics.total_requests
        ));

        output.push_str("# HELP app_request_duration_seconds Request duration in seconds\n");
        output.push_str("# TYPE app_request_duration_seconds histogram\n");
        output.push_str(&format!(
            "app_request_duration_seconds {{}} {}\n",
            snapshot
                .application_metrics
                .average_request_duration
                .as_secs_f64()
        ));

        Ok(output)
    }

    async fn export_influxdb_format(&self, snapshot: &MetricsSnapshot) -> Result<String, AppError> {
        let mut output = String::new();
        let timestamp = snapshot.timestamp.duration_since(UNIX_EPOCH)?.as_nanos();

        // 系统指标
        output.push_str(&format!(
            "system_metrics cpu_usage={},memory_usage={} {}\n",
            snapshot.system_metrics.cpu_usage, snapshot.system_metrics.memory_usage, timestamp
        ));

        // 应用指标
        output.push_str(&format!(
            "application_metrics total_requests={},successful_requests={},failed_requests={} {}\n",
            snapshot.application_metrics.total_requests,
            snapshot.application_metrics.successful_requests,
            snapshot.application_metrics.failed_requests,
            timestamp
        ));

        Ok(output)
    }

    async fn export_csv_format(&self, snapshot: &MetricsSnapshot) -> Result<String, AppError> {
        let mut output = String::new();

        // CSV 头部
        output.push_str("timestamp,metric_type,metric_name,value\n");

        let timestamp = snapshot.timestamp.duration_since(UNIX_EPOCH)?.as_secs();

        // 系统指标
        output.push_str(&format!(
            "{},system,cpu_usage,{}\n",
            timestamp, snapshot.system_metrics.cpu_usage
        ));
        output.push_str(&format!(
            "{},system,memory_usage,{}\n",
            timestamp, snapshot.system_metrics.memory_usage
        ));

        // 应用指标
        output.push_str(&format!(
            "{},application,total_requests,{}\n",
            timestamp, snapshot.application_metrics.total_requests
        ));
        output.push_str(&format!(
            "{},application,successful_requests,{}\n",
            timestamp, snapshot.application_metrics.successful_requests
        ));

        Ok(output)
    }
}

#[async_trait::async_trait]
impl PerformanceOptimizable for MetricsCollector {
    async fn optimize(&self) -> Result<(), AppError> {
        // 清理旧的指标数据
        self.aggregator.cleanup_old_data().await?;

        // 优化指标收集频率
        self.optimize_collection_frequency().await?;

        Ok(())
    }

    async fn get_stats(&self) -> Result<serde_json::Value, AppError> {
        let snapshot = self.get_metrics_snapshot().await?;
        Ok(serde_json::to_value(snapshot)?)
    }

    async fn reset_stats(&self) -> Result<(), AppError> {
        self.system_metrics.reset().await;
        self.application_metrics.reset().await;
        self.custom_metrics.reset().await;
        self.aggregator.reset().await;

        Ok(())
    }
}

impl MetricsCollector {
    async fn optimize_collection_frequency(&self) -> Result<(), AppError> {
        // 根据系统负载动态调整收集频率
        let cpu_usage = self.system_metrics.get_cpu_usage().await;

        if cpu_usage > 80.0 {
            // 高负载时降低收集频率
            // 这里可以动态调整收集间隔
        } else if cpu_usage < 20.0 {
            // 低负载时可以增加收集频率
        }

        Ok(())
    }
}

/// 系统指标
pub struct SystemMetrics {
    cpu_usage: Arc<RwLock<f64>>,
    memory_usage: Arc<RwLock<u64>>,
    disk_usage: Arc<RwLock<u64>>,
    network_io: Arc<RwLock<NetworkIO>>,
    load_average: Arc<RwLock<LoadAverage>>,
    process_count: Arc<RwLock<u32>>,
    uptime: Arc<RwLock<Duration>>,
    start_time: Instant,
}

impl Default for SystemMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMetrics {
    pub fn new() -> Self {
        Self {
            cpu_usage: Arc::new(RwLock::new(0.0)),
            memory_usage: Arc::new(RwLock::new(0)),
            disk_usage: Arc::new(RwLock::new(0)),
            network_io: Arc::new(RwLock::new(NetworkIO::default())),
            load_average: Arc::new(RwLock::new(LoadAverage::default())),
            process_count: Arc::new(RwLock::new(0)),
            uptime: Arc::new(RwLock::new(Duration::from_secs(0))),
            start_time: Instant::now(),
        }
    }

    pub async fn collect(&self) {
        // 收集CPU使用率
        if let Ok(cpu) = self.get_cpu_usage_from_system().await {
            *self.cpu_usage.write().await = cpu;
        }

        // 收集内存使用
        if let Ok(memory) = self.get_memory_usage_from_system().await {
            *self.memory_usage.write().await = memory;
        }

        // 收集磁盘使用
        if let Ok(disk) = self.get_disk_usage_from_system().await {
            *self.disk_usage.write().await = disk;
        }

        // 收集网络IO
        if let Ok(network) = self.get_network_io_from_system().await {
            *self.network_io.write().await = network;
        }

        // 更新运行时间
        *self.uptime.write().await = self.start_time.elapsed();
    }

    pub async fn get_snapshot(&self) -> SystemMetricsSnapshot {
        SystemMetricsSnapshot {
            cpu_usage: *self.cpu_usage.read().await,
            memory_usage: *self.memory_usage.read().await,
            disk_usage: *self.disk_usage.read().await,
            network_io: self.network_io.read().await.clone(),
            load_average: self.load_average.read().await.clone(),
            process_count: *self.process_count.read().await,
            uptime: *self.uptime.read().await,
        }
    }

    pub async fn get_cpu_usage(&self) -> f64 {
        *self.cpu_usage.read().await
    }

    pub async fn reset(&self) {
        *self.cpu_usage.write().await = 0.0;
        *self.memory_usage.write().await = 0;
        *self.disk_usage.write().await = 0;
        *self.network_io.write().await = NetworkIO::default();
        *self.load_average.write().await = LoadAverage::default();
        *self.process_count.write().await = 0;
    }

    // 系统指标收集的具体实现
    async fn get_cpu_usage_from_system(&self) -> Result<f64, AppError> {
        // 实际实现中会调用系统API获取CPU使用率
        // 这里返回模拟数据
        Ok(rand::random::<f64>() * 100.0)
    }

    async fn get_memory_usage_from_system(&self) -> Result<u64, AppError> {
        // 实际实现中会调用系统API获取内存使用
        Ok(1024 * 1024 * 1024) // 1GB
    }

    async fn get_disk_usage_from_system(&self) -> Result<u64, AppError> {
        // 实际实现中会调用系统API获取磁盘使用
        Ok(10 * 1024 * 1024 * 1024) // 10GB
    }

    async fn get_network_io_from_system(&self) -> Result<NetworkIO, AppError> {
        // 实际实现中会调用系统API获取网络IO
        Ok(NetworkIO {
            bytes_sent: 1024 * 1024,
            bytes_received: 2 * 1024 * 1024,
            packets_sent: 1000,
            packets_received: 2000,
        })
    }
}

/// 应用指标
pub struct ApplicationMetrics {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    request_durations: Arc<Mutex<VecDeque<Duration>>>,

    documents_processed: AtomicU64,
    processing_durations: Arc<Mutex<VecDeque<Duration>>>,
    processing_errors: DashMap<String, AtomicU64>,

    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    cache_operations: DashMap<String, AtomicU64>,

    active_connections: AtomicUsize,
    queue_size: AtomicUsize,

    error_counts: DashMap<String, AtomicU64>,
}

impl Default for ApplicationMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationMetrics {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            request_durations: Arc::new(Mutex::new(VecDeque::new())),
            documents_processed: AtomicU64::new(0),
            processing_durations: Arc::new(Mutex::new(VecDeque::new())),
            processing_errors: DashMap::new(),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            cache_operations: DashMap::new(),
            active_connections: AtomicUsize::new(0),
            queue_size: AtomicUsize::new(0),
            error_counts: DashMap::new(),
        }
    }

    pub async fn record_request(&self, duration: Duration, success: bool) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);

        if success {
            self.successful_requests.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failed_requests.fetch_add(1, Ordering::Relaxed);
        }

        let mut durations = self.request_durations.lock().await;
        durations.push_back(duration);

        // 保持最近1000个请求的持续时间
        if durations.len() > 1000 {
            durations.pop_front();
        }
    }

    pub async fn record_document_processing(
        &self,
        format: &str,
        _size: u64,
        duration: Duration,
        success: bool,
    ) {
        self.documents_processed.fetch_add(1, Ordering::Relaxed);

        if success {
            let mut durations = self.processing_durations.lock().await;
            durations.push_back(duration);

            if durations.len() > 1000 {
                durations.pop_front();
            }
        } else {
            self.processing_errors
                .entry(format.to_string())
                .or_insert_with(|| AtomicU64::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn record_cache_operation(&self, cache_type: &str, operation: &str, hit: bool) {
        if hit {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        let key = format!("{cache_type}:{operation}");
        self.cache_operations
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_error(&self, error_type: &str, error_code: &str) {
        let key = format!("{error_type}:{error_code}");
        self.error_counts
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub async fn collect(&self) {
        // 定期收集应用指标
        // 这里可以添加额外的指标收集逻辑
    }

    pub async fn get_snapshot(&self) -> ApplicationMetricsSnapshot {
        let request_durations = self.request_durations.lock().await;
        let processing_durations = self.processing_durations.lock().await;

        let average_request_duration = if !request_durations.is_empty() {
            let total: Duration = request_durations.iter().sum();
            total / request_durations.len() as u32
        } else {
            Duration::from_secs(0)
        };

        let average_processing_duration = if !processing_durations.is_empty() {
            let total: Duration = processing_durations.iter().sum();
            total / processing_durations.len() as u32
        } else {
            Duration::from_secs(0)
        };

        ApplicationMetricsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            average_request_duration,
            documents_processed: self.documents_processed.load(Ordering::Relaxed),
            average_processing_duration,
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            active_connections: self.active_connections.load(Ordering::Relaxed),
            queue_size: self.queue_size.load(Ordering::Relaxed),
        }
    }

    pub async fn reset(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.successful_requests.store(0, Ordering::Relaxed);
        self.failed_requests.store(0, Ordering::Relaxed);
        self.request_durations.lock().await.clear();
        self.documents_processed.store(0, Ordering::Relaxed);
        self.processing_durations.lock().await.clear();
        self.processing_errors.clear();
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.cache_operations.clear();
        self.active_connections.store(0, Ordering::Relaxed);
        self.queue_size.store(0, Ordering::Relaxed);
        self.error_counts.clear();
    }
}

/// 自定义指标
pub struct CustomMetrics {
    counters: DashMap<String, AtomicU64>,
    gauges: DashMap<String, Arc<RwLock<f64>>>,
    histograms: DashMap<String, Arc<Mutex<Vec<f64>>>>,
    timers: DashMap<String, Arc<Mutex<VecDeque<Duration>>>>,
}

impl Default for CustomMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomMetrics {
    pub fn new() -> Self {
        Self {
            counters: DashMap::new(),
            gauges: DashMap::new(),
            histograms: DashMap::new(),
            timers: DashMap::new(),
        }
    }

    pub async fn record_metric(&self, name: &str, value: f64, _tags: HashMap<String, String>) {
        // 根据指标类型记录
        self.set_gauge(name, value, _tags).await;
    }

    pub async fn increment_counter(&self, name: &str, _tags: HashMap<String, String>) {
        self.counters
            .entry(name.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_histogram(&self, name: &str, value: f64, _tags: HashMap<String, String>) {
        let histogram_arc = self
            .histograms
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
            .clone();
        let mut histogram = histogram_arc.lock().await;

        histogram.push(value);

        // 保持最近1000个值
        if histogram.len() > 1000 {
            histogram.remove(0);
        }
    }

    pub async fn set_gauge(&self, name: &str, value: f64, _tags: HashMap<String, String>) {
        let gauge = self
            .gauges
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(0.0)));

        *gauge.write().await = value;
    }

    pub async fn record_timer(
        &self,
        name: &str,
        duration: Duration,
        _tags: HashMap<String, String>,
    ) {
        let timer_arc = self
            .timers
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
            .clone();
        let mut timer = timer_arc.lock().await;

        timer.push_back(duration);

        if timer.len() > 1000 {
            timer.pop_front();
        }
    }

    pub async fn get_snapshot(&self) -> CustomMetricsSnapshot {
        let mut counters = HashMap::new();
        let mut gauges = HashMap::new();
        let mut histograms = HashMap::new();
        let mut timers = HashMap::new();

        for entry in self.counters.iter() {
            counters.insert(entry.key().clone(), entry.value().load(Ordering::Relaxed));
        }

        for entry in self.gauges.iter() {
            gauges.insert(entry.key().clone(), *entry.value().read().await);
        }

        for entry in self.histograms.iter() {
            histograms.insert(entry.key().clone(), entry.value().lock().await.clone());
        }

        for entry in self.timers.iter() {
            timers.insert(
                entry.key().clone(),
                entry.value().lock().await.clone().into(),
            );
        }

        CustomMetricsSnapshot {
            counters,
            gauges,
            histograms,
            timers,
        }
    }

    pub async fn reset(&self) {
        self.counters.clear();
        self.gauges.clear();
        self.histograms.clear();
        self.timers.clear();
    }
}

/// 指标聚合器
pub struct MetricsAggregator {
    window_size: Duration,
    aggregated_data: Arc<Mutex<VecDeque<AggregatedMetrics>>>,
}

impl MetricsAggregator {
    pub fn new(window_size: Duration) -> Self {
        Self {
            window_size,
            aggregated_data: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn aggregate_metrics(
        &self,
        system: SystemMetricsSnapshot,
        application: ApplicationMetricsSnapshot,
        custom: CustomMetricsSnapshot,
    ) {
        let aggregated = AggregatedMetrics {
            timestamp: SystemTime::now(),
            system_metrics: system,
            application_metrics: application,
            custom_metrics: custom,
        };

        let mut data = self.aggregated_data.lock().await;
        data.push_back(aggregated);

        // 清理超出窗口的数据
        let cutoff = SystemTime::now() - self.window_size;
        while let Some(front) = data.front() {
            if front.timestamp < cutoff {
                data.pop_front();
            } else {
                break;
            }
        }
    }

    pub async fn get_aggregated_metrics(
        &self,
        window: Duration,
    ) -> Result<AggregatedMetrics, AppError> {
        let data = self.aggregated_data.lock().await;
        let cutoff = SystemTime::now() - window;

        // 计算窗口内的平均值
        let relevant_data: Vec<_> = data.iter().filter(|m| m.timestamp >= cutoff).collect();

        if relevant_data.is_empty() {
            return Err(AppError::Config("No metrics data available".to_string()));
        }

        // 计算平均值（简化实现）
        let avg_cpu = relevant_data
            .iter()
            .map(|m| m.system_metrics.cpu_usage)
            .sum::<f64>()
            / relevant_data.len() as f64;

        let avg_memory = relevant_data
            .iter()
            .map(|m| m.system_metrics.memory_usage)
            .sum::<u64>()
            / relevant_data.len() as u64;

        // 构建聚合结果
        Ok(AggregatedMetrics {
            timestamp: SystemTime::now(),
            system_metrics: SystemMetricsSnapshot {
                cpu_usage: avg_cpu,
                memory_usage: avg_memory,
                ..relevant_data[0].system_metrics.clone()
            },
            application_metrics: relevant_data[0].application_metrics.clone(),
            custom_metrics: relevant_data[0].custom_metrics.clone(),
        })
    }

    pub async fn cleanup_old_data(&self) -> Result<(), AppError> {
        let mut data = self.aggregated_data.lock().await;
        let cutoff = SystemTime::now() - self.window_size * 2; // 保留2倍窗口的数据

        while let Some(front) = data.front() {
            if front.timestamp < cutoff {
                data.pop_front();
            } else {
                break;
            }
        }

        Ok(())
    }

    pub async fn reset(&self) {
        self.aggregated_data.lock().await.clear();
    }
}

/// 指标报告器
pub struct MetricsReporter {
    config: MetricsConfig,
    alert_thresholds: Arc<RwLock<HashMap<String, AlertThreshold>>>,
    report_history: Arc<Mutex<VecDeque<PerformanceReport>>>,
}

impl MetricsReporter {
    pub async fn new(config: MetricsConfig) -> Result<Self, AppError> {
        Ok(Self {
            config,
            alert_thresholds: Arc::new(RwLock::new(HashMap::new())),
            report_history: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub async fn generate_report(&self, period: Duration) -> Result<PerformanceReport, AppError> {
        // 生成性能报告的逻辑
        let report = PerformanceReport {
            id: Uuid::new_v4().to_string(),
            period,
            generated_at: SystemTime::now(),
            summary: ReportSummary::default(),
            detailed_metrics: HashMap::new(),
            recommendations: Vec::new(),
        };

        // 保存报告历史
        let mut history = self.report_history.lock().await;
        history.push_back(report.clone());

        // 保持最近100个报告
        if history.len() > 100 {
            history.pop_front();
        }

        Ok(report)
    }

    pub async fn generate_periodic_report(&self) -> Result<(), AppError> {
        let _report = self.generate_report(self.config.report_interval).await?;
        // 这里可以将报告发送到外部系统
        Ok(())
    }

    pub async fn set_alert_threshold(
        &self,
        metric_name: &str,
        threshold: f64,
        condition: AlertCondition,
    ) -> Result<(), AppError> {
        let mut thresholds = self.alert_thresholds.write().await;
        thresholds.insert(
            metric_name.to_string(),
            AlertThreshold {
                threshold,
                condition,
                enabled: true,
            },
        );

        Ok(())
    }

    pub async fn check_alerts(&self) -> Result<Vec<Alert>, AppError> {
        let thresholds = self.alert_thresholds.read().await;
        let mut alerts = Vec::new();

        // 检查告警条件
        for (metric_name, threshold) in thresholds.iter() {
            if threshold.enabled {
                // 这里需要获取当前指标值并检查是否触发告警
                // 简化实现
                if self.should_trigger_alert(metric_name, threshold).await {
                    alerts.push(Alert {
                        id: Uuid::new_v4().to_string(),
                        metric_name: metric_name.clone(),
                        current_value: 0.0, // 实际值
                        threshold_value: threshold.threshold,
                        condition: threshold.condition,
                        triggered_at: SystemTime::now(),
                        severity: AlertSeverity::Warning,
                        message: format!("Metric {metric_name} triggered alert condition"),
                    });
                }
            }
        }

        Ok(alerts)
    }

    async fn should_trigger_alert(&self, _metric_name: &str, _threshold: &AlertThreshold) -> bool {
        // 实际实现中会检查指标值
        false
    }
}

// 数据结构定义

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub timestamp: SystemTime,
    pub system_metrics: SystemMetricsSnapshot,
    pub application_metrics: ApplicationMetricsSnapshot,
    pub custom_metrics: CustomMetricsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetricsSnapshot {
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub disk_usage: u64,
    pub network_io: NetworkIO,
    pub load_average: LoadAverage,
    pub process_count: u32,
    pub uptime: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationMetricsSnapshot {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub average_request_duration: Duration,
    pub documents_processed: u64,
    pub average_processing_duration: Duration,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub active_connections: usize,
    pub queue_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMetricsSnapshot {
    pub counters: HashMap<String, u64>,
    pub gauges: HashMap<String, f64>,
    pub histograms: HashMap<String, Vec<f64>>,
    pub timers: HashMap<String, Vec<Duration>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkIO {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packets_sent: u64,
    pub packets_received: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadAverage {
    pub one_minute: f64,
    pub five_minutes: f64,
    pub fifteen_minutes: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedMetrics {
    pub timestamp: SystemTime,
    pub system_metrics: SystemMetricsSnapshot,
    pub application_metrics: ApplicationMetricsSnapshot,
    pub custom_metrics: CustomMetricsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub id: String,
    pub period: Duration,
    pub generated_at: SystemTime,
    pub summary: ReportSummary,
    pub detailed_metrics: HashMap<String, serde_json::Value>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReportSummary {
    pub average_cpu_usage: f64,
    pub peak_memory_usage: u64,
    pub total_requests: u64,
    pub error_rate: f64,
    pub average_response_time: Duration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ExportFormat {
    Json,
    Prometheus,
    InfluxDB,
    Csv,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AlertCondition {
    GreaterThan,
    LessThan,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertThreshold {
    pub threshold: f64,
    pub condition: AlertCondition,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub metric_name: String,
    pub current_value: f64,
    pub threshold_value: f64,
    pub condition: AlertCondition,
    pub triggered_at: SystemTime,
    pub severity: AlertSeverity,
    pub message: String,
}
