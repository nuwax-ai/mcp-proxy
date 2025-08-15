use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// 指标类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
}

/// 指标标签
pub type Labels = HashMap<String, String>;

/// 计数器指标
#[derive(Debug)]
pub struct Counter {
    value: AtomicU64,
    labels: Labels,
}

impl Counter {
    pub fn new(labels: Labels) -> Self {
        Self {
            value: AtomicU64::new(0),
            labels,
        }
    }

    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, value: u64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.value.store(0, Ordering::Relaxed);
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }
}

/// 仪表指标
#[derive(Debug)]
pub struct Gauge {
    value: AtomicU64,
    labels: Labels,
}

impl Gauge {
    pub fn new(labels: Labels) -> Self {
        Self {
            value: AtomicU64::new(0),
            labels,
        }
    }

    pub fn set(&self, value: u64) {
        self.value.store(value, Ordering::Relaxed);
    }

    pub fn inc(&self) {
        self.add(1);
    }

    pub fn dec(&self) {
        self.sub(1);
    }

    pub fn add(&self, value: u64) {
        self.value.fetch_add(value, Ordering::Relaxed);
    }

    pub fn sub(&self, value: u64) {
        self.value.fetch_sub(value, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }
}

/// 直方图桶
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramBucket {
    pub upper_bound: f64,
    pub count: u64,
}

/// 直方图指标
#[derive(Debug)]
pub struct Histogram {
    buckets: Vec<AtomicU64>,
    bucket_bounds: Vec<f64>,
    sum: AtomicU64, // 以微秒为单位存储
    count: AtomicU64,
    labels: Labels,
}

impl Histogram {
    pub fn new(bucket_bounds: Vec<f64>, labels: Labels) -> Self {
        let buckets = bucket_bounds.iter().map(|_| AtomicU64::new(0)).collect();
        
        Self {
            buckets,
            bucket_bounds,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
            labels,
        }
    }

    pub fn observe(&self, value: f64) {
        // 更新总和（转换为微秒）
        let micros = (value * 1_000_000.0) as u64;
        self.sum.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // 更新桶计数
        for (i, &bound) in self.bucket_bounds.iter().enumerate() {
            if value <= bound {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn observe_duration(&self, duration: Duration) {
        self.observe(duration.as_secs_f64());
    }

    pub fn get_buckets(&self) -> Vec<HistogramBucket> {
        self.bucket_bounds
            .iter()
            .zip(self.buckets.iter())
            .map(|(&bound, bucket)| HistogramBucket {
                upper_bound: bound,
                count: bucket.load(Ordering::Relaxed),
            })
            .collect()
    }

    pub fn get_sum(&self) -> f64 {
        self.sum.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn get_count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn get_average(&self) -> f64 {
        let count = self.get_count();
        if count == 0 {
            0.0
        } else {
            self.get_sum() / count as f64
        }
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }
}

/// 摘要统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryStats {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
}

/// 摘要指标
#[derive(Debug)]
pub struct Summary {
    values: Arc<RwLock<Vec<f64>>>,
    max_samples: usize,
    labels: Labels,
}

impl Summary {
    pub fn new(max_samples: usize, labels: Labels) -> Self {
        Self {
            values: Arc::new(RwLock::new(Vec::new())),
            max_samples,
            labels,
        }
    }

    pub async fn observe(&self, value: f64) {
        let mut values = self.values.write().await;
        values.push(value);
        
        // 保持样本数量在限制内
        if values.len() > self.max_samples {
            values.remove(0);
        }
    }

    pub async fn observe_duration(&self, duration: Duration) {
        self.observe(duration.as_secs_f64()).await;
    }

    pub async fn get_stats(&self) -> SummaryStats {
        let values = self.values.read().await;
        
        if values.is_empty() {
            return SummaryStats {
                count: 0,
                sum: 0.0,
                min: 0.0,
                max: 0.0,
                avg: 0.0,
                p50: 0.0,
                p90: 0.0,
                p95: 0.0,
                p99: 0.0,
            };
        }

        let mut sorted_values = values.clone();
        sorted_values.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let count = sorted_values.len() as u64;
        let sum: f64 = sorted_values.iter().sum();
        let min = sorted_values[0];
        let max = sorted_values[sorted_values.len() - 1];
        let avg = sum / count as f64;

        let p50 = Self::percentile(&sorted_values, 0.5);
        let p90 = Self::percentile(&sorted_values, 0.9);
        let p95 = Self::percentile(&sorted_values, 0.95);
        let p99 = Self::percentile(&sorted_values, 0.99);

        SummaryStats {
            count,
            sum,
            min,
            max,
            avg,
            p50,
            p90,
            p95,
            p99,
        }
    }

    fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
        if sorted_values.is_empty() {
            return 0.0;
        }

        let index = (percentile * (sorted_values.len() - 1) as f64) as usize;
        sorted_values[index.min(sorted_values.len() - 1)]
    }

    pub fn labels(&self) -> &Labels {
        &self.labels
    }
}

/// 指标注册表
#[derive(Debug)]
pub struct MetricsRegistry {
    counters: Arc<RwLock<HashMap<String, Arc<Counter>>>>,
    gauges: Arc<RwLock<HashMap<String, Arc<Gauge>>>>,
    histograms: Arc<RwLock<HashMap<String, Arc<Histogram>>>>,
    summaries: Arc<RwLock<HashMap<String, Arc<Summary>>>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(RwLock::new(HashMap::new())),
            gauges: Arc::new(RwLock::new(HashMap::new())),
            histograms: Arc::new(RwLock::new(HashMap::new())),
            summaries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册计数器
    pub async fn register_counter(&self, name: String, labels: Labels) -> Arc<Counter> {
        let counter = Arc::new(Counter::new(labels));
        let mut counters = self.counters.write().await;
        counters.insert(name, counter.clone());
        counter
    }

    /// 注册仪表
    pub async fn register_gauge(&self, name: String, labels: Labels) -> Arc<Gauge> {
        let gauge = Arc::new(Gauge::new(labels));
        let mut gauges = self.gauges.write().await;
        gauges.insert(name, gauge.clone());
        gauge
    }

    /// 注册直方图
    pub async fn register_histogram(&self, name: String, bucket_bounds: Vec<f64>, labels: Labels) -> Arc<Histogram> {
        let histogram = Arc::new(Histogram::new(bucket_bounds, labels));
        let mut histograms = self.histograms.write().await;
        histograms.insert(name, histogram.clone());
        histogram
    }

    /// 注册摘要
    pub async fn register_summary(&self, name: String, max_samples: usize, labels: Labels) -> Arc<Summary> {
        let summary = Arc::new(Summary::new(max_samples, labels));
        let mut summaries = self.summaries.write().await;
        summaries.insert(name, summary.clone());
        summary
    }

    /// 获取计数器
    pub async fn get_counter(&self, name: &str) -> Option<Arc<Counter>> {
        let counters = self.counters.read().await;
        counters.get(name).cloned()
    }

    /// 获取仪表
    pub async fn get_gauge(&self, name: &str) -> Option<Arc<Gauge>> {
        let gauges = self.gauges.read().await;
        gauges.get(name).cloned()
    }

    /// 获取直方图
    pub async fn get_histogram(&self, name: &str) -> Option<Arc<Histogram>> {
        let histograms = self.histograms.read().await;
        histograms.get(name).cloned()
    }

    /// 获取摘要
    pub async fn get_summary(&self, name: &str) -> Option<Arc<Summary>> {
        let summaries = self.summaries.read().await;
        summaries.get(name).cloned()
    }

    /// 导出所有指标为Prometheus格式
    pub async fn export_prometheus(&self) -> String {
        let mut output = String::new();

        // 导出计数器
        let counters = self.counters.read().await;
        for (name, counter) in counters.iter() {
            output.push_str(&format!("# TYPE {name} counter\n"));
            let labels_str = Self::format_labels(counter.labels());
            output.push_str(&format!("{}{} {}\n", name, labels_str, counter.get()));
        }

        // 导出仪表
        let gauges = self.gauges.read().await;
        for (name, gauge) in gauges.iter() {
            output.push_str(&format!("# TYPE {name} gauge\n"));
            let labels_str = Self::format_labels(gauge.labels());
            output.push_str(&format!("{}{} {}\n", name, labels_str, gauge.get()));
        }

        // 导出直方图
        let histograms = self.histograms.read().await;
        for (name, histogram) in histograms.iter() {
            output.push_str(&format!("# TYPE {name} histogram\n"));
            let base_labels = Self::format_labels(histogram.labels());
            
            // 导出桶
            for bucket in histogram.get_buckets() {
                let mut labels = histogram.labels().clone();
                labels.insert("le".to_string(), bucket.upper_bound.to_string());
                let labels_str = Self::format_labels(&labels);
                output.push_str(&format!("{}_bucket{} {}\n", name, labels_str, bucket.count));
            }
            
            // 导出总和和计数
            output.push_str(&format!("{}_sum{} {}\n", name, base_labels, histogram.get_sum()));
            output.push_str(&format!("{}_count{} {}\n", name, base_labels, histogram.get_count()));
        }

        output
    }

    /// 导出所有指标为JSON格式
    pub async fn export_json(&self) -> Result<String, serde_json::Error> {
        let mut metrics = serde_json::Map::new();

        // 导出计数器
        let counters = self.counters.read().await;
        let mut counter_metrics = serde_json::Map::new();
        for (name, counter) in counters.iter() {
            let mut metric = serde_json::Map::new();
            metric.insert("type".to_string(), serde_json::Value::String("counter".to_string()));
            metric.insert("value".to_string(), serde_json::Value::Number(counter.get().into()));
            metric.insert("labels".to_string(), serde_json::to_value(counter.labels())?);
            counter_metrics.insert(name.clone(), serde_json::Value::Object(metric));
        }
        metrics.insert("counters".to_string(), serde_json::Value::Object(counter_metrics));

        // 导出仪表
        let gauges = self.gauges.read().await;
        let mut gauge_metrics = serde_json::Map::new();
        for (name, gauge) in gauges.iter() {
            let mut metric = serde_json::Map::new();
            metric.insert("type".to_string(), serde_json::Value::String("gauge".to_string()));
            metric.insert("value".to_string(), serde_json::Value::Number(gauge.get().into()));
            metric.insert("labels".to_string(), serde_json::to_value(gauge.labels())?);
            gauge_metrics.insert(name.clone(), serde_json::Value::Object(metric));
        }
        metrics.insert("gauges".to_string(), serde_json::Value::Object(gauge_metrics));

        // 导出直方图
        let histograms = self.histograms.read().await;
        let mut histogram_metrics = serde_json::Map::new();
        for (name, histogram) in histograms.iter() {
            let mut metric = serde_json::Map::new();
            metric.insert("type".to_string(), serde_json::Value::String("histogram".to_string()));
            metric.insert("buckets".to_string(), serde_json::to_value(histogram.get_buckets())?);
            metric.insert("sum".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(histogram.get_sum()).unwrap_or_else(|| serde_json::Number::from(0))));
            metric.insert("count".to_string(), serde_json::Value::Number(histogram.get_count().into()));
            metric.insert("average".to_string(), serde_json::Value::Number(serde_json::Number::from_f64(histogram.get_average()).unwrap_or_else(|| serde_json::Number::from(0))));
            metric.insert("labels".to_string(), serde_json::to_value(histogram.labels())?);
            histogram_metrics.insert(name.clone(), serde_json::Value::Object(metric));
        }
        metrics.insert("histograms".to_string(), serde_json::Value::Object(histogram_metrics));

        // 导出摘要
        let summaries = self.summaries.read().await;
        let mut summary_metrics = serde_json::Map::new();
        for (name, summary) in summaries.iter() {
            let stats = summary.get_stats().await;
            let mut metric = serde_json::Map::new();
            metric.insert("type".to_string(), serde_json::Value::String("summary".to_string()));
            metric.insert("stats".to_string(), serde_json::to_value(stats)?);
            metric.insert("labels".to_string(), serde_json::to_value(summary.labels())?);
            summary_metrics.insert(name.clone(), serde_json::Value::Object(metric));
        }
        metrics.insert("summaries".to_string(), serde_json::Value::Object(summary_metrics));

        serde_json::to_string_pretty(&metrics)
    }

    /// 格式化标签为Prometheus格式
    fn format_labels(labels: &Labels) -> String {
        if labels.is_empty() {
            return String::new();
        }

        let mut label_pairs: Vec<String> = labels
            .iter()
            .map(|(k, v)| format!("{k}=\"{v}\""))
            .collect();
        
        label_pairs.sort();
        format!("{{{}}}", label_pairs.join(","))
    }

    /// 重置所有指标
    pub async fn reset_all(&self) {
        // 重置计数器
        let counters = self.counters.read().await;
        for counter in counters.values() {
            counter.reset();
        }

        // 仪表不需要重置，因为它们表示当前状态
        // 直方图和摘要也不重置，因为它们累积历史数据
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// 异步指标收集器
#[derive(Debug)]
pub struct AsyncMetricsCollector {
    registry: Arc<MetricsRegistry>,
    collection_interval: Duration,
    is_running: Arc<std::sync::atomic::AtomicBool>,
}

impl AsyncMetricsCollector {
    pub fn new(registry: Arc<MetricsRegistry>, collection_interval: Duration) -> Self {
        Self {
            registry,
            collection_interval,
            is_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 启动异步指标收集
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.is_running.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Err("Metrics collector is already running".into());
        }

        let registry = self.registry.clone();
        let interval = self.collection_interval;
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);

            while is_running.load(std::sync::atomic::Ordering::SeqCst) {
                interval_timer.tick().await;

                // 收集系统指标
                if let Err(e) = Self::collect_system_metrics(&registry).await {
                    tracing::warn!("Failed to collect system metrics: {}", e);
                }

                // 收集应用指标
                if let Err(e) = Self::collect_application_metrics(&registry).await {
                    tracing::warn!("Failed to collect application metrics: {}", e);
                }
            }
        });

        Ok(())
    }

    /// 停止异步指标收集
    pub fn stop(&self) {
        self.is_running.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// 收集系统指标
    async fn collect_system_metrics(registry: &MetricsRegistry) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 内存使用情况
        if let Ok(memory_info) = Self::get_memory_usage().await {
            if let Some(gauge) = registry.get_gauge("system_memory_used_bytes").await {
                gauge.set(memory_info.used);
            }
            if let Some(gauge) = registry.get_gauge("system_memory_total_bytes").await {
                gauge.set(memory_info.total);
            }
            if let Some(gauge) = registry.get_gauge("system_memory_usage_percent").await {
                let usage_percent = (memory_info.used as f64 / memory_info.total as f64 * 100.0) as u64;
                gauge.set(usage_percent);
            }
        }

        // CPU使用情况
        if let Ok(cpu_usage) = Self::get_cpu_usage().await {
            if let Some(gauge) = registry.get_gauge("system_cpu_usage_percent").await {
                gauge.set((cpu_usage * 100.0) as u64);
            }
        }

        // 磁盘使用情况
        if let Ok(disk_info) = Self::get_disk_usage(".").await {
            if let Some(gauge) = registry.get_gauge("system_disk_used_bytes").await {
                gauge.set(disk_info.used);
            }
            if let Some(gauge) = registry.get_gauge("system_disk_total_bytes").await {
                gauge.set(disk_info.total);
            }
            if let Some(gauge) = registry.get_gauge("system_disk_usage_percent").await {
                let usage_percent = (disk_info.used as f64 / disk_info.total as f64 * 100.0) as u64;
                gauge.set(usage_percent);
            }
        }

        Ok(())
    }

    /// 收集应用指标
    async fn collect_application_metrics(registry: &MetricsRegistry) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 运行时间
        let uptime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        if let Some(gauge) = registry.get_gauge("application_uptime_seconds").await {
            gauge.set(uptime);
        }

        // Tokio运行时指标
        if let Some(gauge) = registry.get_gauge("tokio_active_tasks").await {
            // 这里需要实际的Tokio指标，暂时使用占位符
            gauge.set(0);
        }

        Ok(())
    }

    /// 获取内存使用情况
    async fn get_memory_usage() -> Result<MemoryInfo, Box<dyn std::error::Error + Send + Sync>> {
        tokio::task::spawn_blocking(|| {
            #[cfg(target_os = "macos")]
            {
                use std::process::Command;
                let output = Command::new("vm_stat").output()?;
                let output_str = String::from_utf8(output.stdout)?;

                let mut free_pages = 0u64;
                let mut active_pages = 0u64;
                let mut inactive_pages = 0u64;
                let mut wired_pages = 0u64;

                for line in output_str.lines() {
                    if line.contains("Pages free:") {
                        free_pages = Self::extract_pages(line)?;
                    } else if line.contains("Pages active:") {
                        active_pages = Self::extract_pages(line)?;
                    } else if line.contains("Pages inactive:") {
                        inactive_pages = Self::extract_pages(line)?;
                    } else if line.contains("Pages wired down:") {
                        wired_pages = Self::extract_pages(line)?;
                    }
                }

                let page_size = 4096u64;
                let total = (free_pages + active_pages + inactive_pages + wired_pages) * page_size;
                let used = (active_pages + inactive_pages + wired_pages) * page_size;

                Ok(MemoryInfo { total, used })
            }

            #[cfg(target_os = "linux")]
            {
                let meminfo = std::fs::read_to_string("/proc/meminfo")?;
                let mut total = 0u64;
                let mut available = 0u64;

                for line in meminfo.lines() {
                    if line.starts_with("MemTotal:") {
                        total = Self::extract_kb_value(line)? * 1024;
                    } else if line.starts_with("MemAvailable:") {
                        available = Self::extract_kb_value(line)? * 1024;
                    }
                }

                let used = total - available;
                Ok(MemoryInfo { total, used })
            }

            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                Err("Unsupported platform for memory info".into())
            }
        }).await?
    }

    /// 获取CPU使用率
    async fn get_cpu_usage() -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
        tokio::task::spawn_blocking(|| {
            // 简化的CPU使用率获取，实际实现需要更复杂的逻辑
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            {
                use std::process::Command;
                let output = Command::new("top").arg("-l").arg("1").arg("-n").arg("0").output()?;
                let output_str = String::from_utf8(output.stdout)?;
                
                // 解析top输出获取CPU使用率
                for line in output_str.lines() {
                    if line.contains("CPU usage:") {
                        // 简化解析，实际需要更精确的解析
                        return Ok(0.0);
                    }
                }
                Ok(0.0)
            }

            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                Ok(0.0)
            }
        }).await?
    }

    /// 获取磁盘使用情况
    async fn get_disk_usage(path: &str) -> Result<DiskInfo, Box<dyn std::error::Error + Send + Sync>> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            use std::process::Command;
            let output = Command::new("df").arg("-k").arg(&path).output()?;
            let output_str = String::from_utf8(output.stdout)?;
            let lines: Vec<&str> = output_str.lines().collect();

            if lines.len() >= 2 {
                let parts: Vec<&str> = lines[1].split_whitespace().collect();
                if parts.len() >= 4 {
                    let total = parts[1].parse::<u64>()? * 1024;
                    let used = parts[2].parse::<u64>()? * 1024;
                    return Ok(DiskInfo { total, used });
                }
            }

            Err("Failed to parse df output".into())
        }).await?
    }

    fn extract_pages(line: &str) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let page_str = parts[2].trim_end_matches('.');
            Ok(page_str.parse()?)
        } else {
            Err("Invalid vm_stat line format".into())
        }
    }

    fn extract_kb_value(line: &str) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            Ok(parts[1].parse()?)
        } else {
            Err("Invalid meminfo line format".into())
        }
    }
}

#[derive(Debug)]
struct MemoryInfo {
    total: u64,
    used: u64,
}

#[derive(Debug)]
struct DiskInfo {
    total: u64,
    used: u64,
}

/// 性能监控器
#[derive(Debug)]
pub struct PerformanceMonitor {
    registry: Arc<MetricsRegistry>,
    start_time: Instant,
    collector: Option<AsyncMetricsCollector>,
}

impl PerformanceMonitor {
    pub fn new(registry: Arc<MetricsRegistry>) -> Self {
        Self {
            registry,
            start_time: Instant::now(),
            collector: None,
        }
    }

    /// 创建带有异步收集器的性能监控器
    pub fn with_async_collector(registry: Arc<MetricsRegistry>, collection_interval: Duration) -> Self {
        let collector = AsyncMetricsCollector::new(registry.clone(), collection_interval);
        Self {
            registry,
            start_time: Instant::now(),
            collector: Some(collector),
        }
    }

    /// 启动异步指标收集
    pub async fn start_collection(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref collector) = self.collector {
            collector.start().await?;
        }
        Ok(())
    }

    /// 停止异步指标收集
    pub fn stop_collection(&self) {
        if let Some(ref collector) = self.collector {
            collector.stop();
        }
    }

    /// 初始化标准指标
    pub async fn init_standard_metrics(&self) {
        // HTTP请求指标
        for method in &["GET", "POST", "PUT", "DELETE", "PATCH"] {
            self.registry.register_counter(
                "http_requests_total".to_string(),
                HashMap::from([("method".to_string(), method.to_string())]),
            ).await;
        }
        
        self.registry.register_histogram(
            "http_request_duration_seconds".to_string(),
            vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
            HashMap::new(),
        ).await;

        self.registry.register_counter(
            "http_requests_errors_total".to_string(),
            HashMap::new(),
        ).await;

        // 任务处理指标
        self.registry.register_counter(
            "tasks_processed_total".to_string(),
            HashMap::new(),
        ).await;
        
        self.registry.register_counter(
            "tasks_failed_total".to_string(),
            HashMap::new(),
        ).await;
        
        self.registry.register_gauge(
            "tasks_active".to_string(),
            HashMap::new(),
        ).await;
        
        self.registry.register_gauge(
            "tasks_queued".to_string(),
            HashMap::new(),
        ).await;
        
        self.registry.register_histogram(
            "task_processing_duration_seconds".to_string(),
            vec![1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0, 1800.0, 3600.0],
            HashMap::new(),
        ).await;

        // 文档解析指标
        self.registry.register_counter(
            "documents_parsed_total".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_counter(
            "documents_parse_errors_total".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_histogram(
            "document_parse_duration_seconds".to_string(),
            vec![1.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0, 1800.0, 3600.0],
            HashMap::new(),
        ).await;

        self.registry.register_histogram(
            "document_size_bytes".to_string(),
            vec![1024.0, 10240.0, 102400.0, 1048576.0, 10485760.0, 104857600.0, 1073741824.0],
            HashMap::new(),
        ).await;

        // OSS操作指标
        self.registry.register_counter(
            "oss_operations_total".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_counter(
            "oss_operations_errors_total".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_histogram(
            "oss_operation_duration_seconds".to_string(),
            vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0],
            HashMap::new(),
        ).await;

        // 系统资源指标
        self.registry.register_gauge(
            "system_memory_used_bytes".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "system_memory_total_bytes".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "system_memory_usage_percent".to_string(),
            HashMap::new(),
        ).await;
        
        self.registry.register_gauge(
            "system_cpu_usage_percent".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "system_disk_used_bytes".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "system_disk_total_bytes".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "system_disk_usage_percent".to_string(),
            HashMap::new(),
        ).await;

        // 应用指标
        self.registry.register_gauge(
            "application_uptime_seconds".to_string(),
            HashMap::new(),
        ).await;

        self.registry.register_gauge(
            "tokio_active_tasks".to_string(),
            HashMap::new(),
        ).await;
    }

    /// 记录HTTP请求
    pub async fn record_http_request(&self, _method: &str, _status_code: u16, duration: Duration) {
        // 增加请求计数
        if let Some(counter) = self.registry.get_counter("http_requests_total").await {
            counter.inc();
        }

        // 记录请求持续时间
        if let Some(histogram) = self.registry.get_histogram("http_request_duration_seconds").await {
            histogram.observe_duration(duration);
        }
    }

    /// 记录任务处理
    pub async fn record_task_processing(&self, duration: Duration, _success: bool) {
        // 增加处理计数
        if let Some(counter) = self.registry.get_counter("tasks_processed_total").await {
            counter.inc();
        }

        // 记录处理持续时间
        if let Some(histogram) = self.registry.get_histogram("task_processing_duration_seconds").await {
            histogram.observe_duration(duration);
        }
    }

    /// 更新活跃任务数
    pub async fn update_active_tasks(&self, count: u64) {
        if let Some(gauge) = self.registry.get_gauge("tasks_active").await {
            gauge.set(count);
        }
    }

    /// 更新内存使用量
    pub async fn update_memory_usage(&self, bytes: u64) {
        if let Some(gauge) = self.registry.get_gauge("memory_usage_bytes").await {
            gauge.set(bytes);
        }
    }

    /// 更新CPU使用率
    pub async fn update_cpu_usage(&self, percent: f64) {
        if let Some(gauge) = self.registry.get_gauge("cpu_usage_percent").await {
            gauge.set((percent * 100.0) as u64);
        }
    }

    /// 获取运行时间
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// 获取指标注册表
    pub fn registry(&self) -> &Arc<MetricsRegistry> {
        &self.registry
    }
}

/// 计时器辅助结构
pub struct Timer {
    start: Instant,
    histogram: Option<Arc<Histogram>>,
    summary: Option<Arc<Summary>>,
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Timer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            histogram: None,
            summary: None,
        }
    }

    pub fn with_histogram(histogram: Arc<Histogram>) -> Self {
        Self {
            start: Instant::now(),
            histogram: Some(histogram),
            summary: None,
        }
    }

    pub fn with_summary(summary: Arc<Summary>) -> Self {
        Self {
            start: Instant::now(),
            histogram: None,
            summary: Some(summary),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub async fn stop(self) -> Duration {
        let duration = self.elapsed();
        
        if let Some(histogram) = self.histogram {
            histogram.observe_duration(duration);
        }
        
        if let Some(summary) = self.summary {
            summary.observe_duration(duration).await;
        }
        
        duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_counter() {
        let counter = Counter::new(HashMap::new());
        
        assert_eq!(counter.get(), 0);
        
        counter.inc();
        assert_eq!(counter.get(), 1);
        
        counter.add(5);
        assert_eq!(counter.get(), 6);
        
        counter.reset();
        assert_eq!(counter.get(), 0);
    }

    #[tokio::test]
    async fn test_gauge() {
        let gauge = Gauge::new(HashMap::new());
        
        assert_eq!(gauge.get(), 0);
        
        gauge.set(10);
        assert_eq!(gauge.get(), 10);
        
        gauge.inc();
        assert_eq!(gauge.get(), 11);
        
        gauge.dec();
        assert_eq!(gauge.get(), 10);
        
        gauge.add(5);
        assert_eq!(gauge.get(), 15);
        
        gauge.sub(3);
        assert_eq!(gauge.get(), 12);
    }

    #[tokio::test]
    async fn test_histogram() {
        let histogram = Histogram::new(
            vec![0.1, 0.5, 1.0, 2.0, 5.0],
            HashMap::new(),
        );
        
        histogram.observe(0.05);
        histogram.observe(0.3);
        histogram.observe(1.5);
        histogram.observe(3.0);
        
        assert_eq!(histogram.get_count(), 4);
        assert!(histogram.get_sum() > 0.0);
        assert!(histogram.get_average() > 0.0);
        
        let buckets = histogram.get_buckets();
        assert_eq!(buckets.len(), 5);
        assert_eq!(buckets[0].count, 1); // 0.05 <= 0.1
        assert_eq!(buckets[1].count, 2); // 0.05, 0.3 <= 0.5
    }

    #[tokio::test]
    async fn test_summary() {
        let summary = Summary::new(1000, HashMap::new());
        
        summary.observe(1.0).await;
        summary.observe(2.0).await;
        summary.observe(3.0).await;
        summary.observe(4.0).await;
        summary.observe(5.0).await;
        
        let stats = summary.get_stats().await;
        assert_eq!(stats.count, 5);
        assert_eq!(stats.sum, 15.0);
        assert_eq!(stats.avg, 3.0);
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 5.0);
        assert_eq!(stats.p50, 3.0);
    }

    #[tokio::test]
    async fn test_metrics_registry() {
        let registry = MetricsRegistry::new();
        
        // 注册指标
        let counter = registry.register_counter(
            "test_counter".to_string(),
            HashMap::new(),
        ).await;
        
        let gauge = registry.register_gauge(
            "test_gauge".to_string(),
            HashMap::new(),
        ).await;
        
        // 使用指标
        counter.inc();
        gauge.set(42);
        
        // 获取指标
        let retrieved_counter = registry.get_counter("test_counter").await.unwrap();
        assert_eq!(retrieved_counter.get(), 1);
        
        let retrieved_gauge = registry.get_gauge("test_gauge").await.unwrap();
        assert_eq!(retrieved_gauge.get(), 42);
        
        // 导出指标
        let json_export = registry.export_json().await.unwrap();
        assert!(json_export.contains("test_counter"));
        assert!(json_export.contains("test_gauge"));
    }

    #[tokio::test]
    async fn test_timer() {
        let histogram = Arc::new(Histogram::new(
            vec![0.001, 0.01, 0.1, 1.0],
            HashMap::new(),
        ));
        
        let timer = Timer::with_histogram(histogram.clone());
        
        // 模拟一些工作
        sleep(Duration::from_millis(10)).await;
        
        let duration = timer.stop().await;
        assert!(duration >= Duration::from_millis(10));
        assert_eq!(histogram.get_count(), 1);
    }
}