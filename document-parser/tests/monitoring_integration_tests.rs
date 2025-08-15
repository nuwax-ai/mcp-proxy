use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use tokio::time::sleep;
use document_parser::utils::{
    logging::{EnhancedLoggingSystem, LoggingConfig, LogFormat, LogOutputTarget, CorrelationContext},
    metrics::{MetricsRegistry, PerformanceMonitor, AsyncMetricsCollector},
    health_check::{EnhancedHealthCheckManager, HealthCheckConfig, HealthChecker, HealthCheckResult, HealthStatus},
};

/// 模拟健康检查器
struct MockHealthChecker {
    name: String,
    should_fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
    response_delay: Duration,
}

impl MockHealthChecker {
    fn new(name: String, response_delay: Duration) -> Self {
        Self {
            name,
            should_fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            response_delay,
        }
    }

    fn set_should_fail(&self, should_fail: bool) {
        self.should_fail.store(should_fail, std::sync::atomic::Ordering::Relaxed);
    }
}

#[async_trait::async_trait]
impl HealthChecker for MockHealthChecker {
    async fn check_health(&self) -> HealthCheckResult {
        // 模拟检查延迟
        sleep(self.response_delay).await;

        let status = if self.should_fail.load(std::sync::atomic::Ordering::Relaxed) {
            HealthStatus::Unhealthy
        } else {
            HealthStatus::Healthy
        };

        let mut result = HealthCheckResult::new(
            self.name.clone(),
            status,
            format!("Mock health check for {}", self.name),
        );

        result.add_detail("mock".to_string(), "true".to_string());
        result.add_detail("delay_ms".to_string(), self.response_delay.as_millis().to_string());

        result.with_response_time(self.response_delay)
    }

    fn component_name(&self) -> &str {
        &self.name
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

#[tokio::test]
async fn test_enhanced_logging_system() {
    // 创建临时日志文件
    let temp_dir = tempfile::tempdir().unwrap();
    let log_file = temp_dir.path().join("test.log");

    let config = LoggingConfig {
        level: "debug".to_string(),
        format: LogFormat::Json,
        output: LogOutputTarget::Both,
        file_path: Some(log_file.to_string_lossy().to_string()),
        enable_console: true,
        enable_json: true,
        enable_correlation: true,
        service_name: "test-service".to_string(),
        service_version: "1.0.0".to_string(),
        environment: "test".to_string(),
        ..Default::default()
    };

    // 初始化日志系统
    let logging_system = EnhancedLoggingSystem::init(config).unwrap();

    // 设置关联上下文
    let correlation = CorrelationContext::new()
        .with_request_id("req-123".to_string())
        .with_task_id("task-456".to_string())
        .with_user_id("user-789".to_string());

    logging_system.set_correlation_context(correlation).await;

    // 生成一些日志
    tracing::info!("测试信息日志");
    tracing::warn!(error_code = "E001", "测试警告日志");
    tracing::error!(component = "test", "测试错误日志");

    // 创建带有关联上下文的span
    let span = logging_system.create_span("test_operation").await;
    let _enter = span.enter();

    tracing::info!("在span中的日志");

    // 等待日志写入
    sleep(Duration::from_millis(100)).await;

    // 验证日志文件存在
    assert!(log_file.exists());

    // 读取日志文件内容
    let log_content = tokio::fs::read_to_string(&log_file).await.unwrap();
    assert!(!log_content.is_empty());
    
    // 验证JSON格式的日志包含关联信息
    assert!(log_content.contains("req-123"));
    assert!(log_content.contains("task-456"));
    assert!(log_content.contains("user-789"));
    assert!(log_content.contains("test-service"));

    println!("日志内容预览:\n{}", &log_content[..log_content.len().min(500)]);
}

#[tokio::test]
async fn test_metrics_registry_and_async_collector() {
    let registry = Arc::new(MetricsRegistry::new());

    // 注册一些测试指标
    let counter = registry.register_counter(
        "test_counter".to_string(),
        HashMap::from([("service".to_string(), "test".to_string())]),
    ).await;

    let gauge = registry.register_gauge(
        "test_gauge".to_string(),
        HashMap::new(),
    ).await;

    let histogram = registry.register_histogram(
        "test_histogram".to_string(),
        vec![0.1, 0.5, 1.0, 2.0, 5.0],
        HashMap::new(),
    ).await;

    // 使用指标
    counter.inc();
    counter.add(5);
    assert_eq!(counter.get(), 6);

    gauge.set(42);
    gauge.inc();
    assert_eq!(gauge.get(), 43);

    histogram.observe(0.3);
    histogram.observe(1.5);
    histogram.observe(3.0);
    assert_eq!(histogram.get_count(), 3);

    // 测试异步指标收集器
    let collector = AsyncMetricsCollector::new(registry.clone(), Duration::from_millis(100));
    collector.start().await.unwrap();

    // 等待几个收集周期
    sleep(Duration::from_millis(300)).await;

    collector.stop();

    // 导出指标
    let prometheus_export = registry.export_prometheus().await;
    assert!(prometheus_export.contains("test_counter"));
    assert!(prometheus_export.contains("test_gauge"));
    assert!(prometheus_export.contains("test_histogram"));

    let json_export = registry.export_json().await.unwrap();
    assert!(json_export.contains("test_counter"));
    assert!(json_export.contains("test_gauge"));
    assert!(json_export.contains("test_histogram"));

    println!("Prometheus导出:\n{}", prometheus_export);
    println!("JSON导出:\n{}", json_export);
}

#[tokio::test]
async fn test_performance_monitor_with_async_collection() {
    let registry = Arc::new(MetricsRegistry::new());
    let monitor = PerformanceMonitor::with_async_collector(
        registry.clone(),
        Duration::from_millis(50),
    );

    // 初始化标准指标
    monitor.init_standard_metrics().await;

    // 启动异步收集
    monitor.start_collection().await.unwrap();

    // 模拟一些活动
    monitor.record_http_request("GET", 200, Duration::from_millis(150)).await;
    monitor.record_http_request("POST", 201, Duration::from_millis(300)).await;
    monitor.record_http_request("GET", 404, Duration::from_millis(50)).await;

    monitor.record_task_processing(Duration::from_secs(5), true).await;
    monitor.record_task_processing(Duration::from_secs(10), false).await;

    monitor.update_active_tasks(3).await;
    monitor.update_memory_usage(1024 * 1024 * 512).await; // 512MB
    monitor.update_cpu_usage(0.75).await; // 75%

    // 等待指标收集
    sleep(Duration::from_millis(200)).await;

    // 验证指标
    let http_counter = registry.get_counter("http_requests_total").await;
    assert!(http_counter.is_some());

    let task_counter = registry.get_counter("tasks_processed_total").await;
    assert!(task_counter.is_some());
    assert_eq!(task_counter.unwrap().get(), 2);

    let active_tasks_gauge = registry.get_gauge("tasks_active").await;
    assert!(active_tasks_gauge.is_some());
    assert_eq!(active_tasks_gauge.unwrap().get(), 3);

    // 停止收集
    monitor.stop_collection();

    // 导出指标验证
    let metrics_export = registry.export_json().await.unwrap();
    assert!(metrics_export.contains("http_requests_total"));
    assert!(metrics_export.contains("tasks_processed_total"));
    assert!(metrics_export.contains("tasks_active"));

    println!("性能监控指标:\n{}", metrics_export);
}

#[tokio::test]
async fn test_enhanced_health_check_manager() {
    let config = HealthCheckConfig {
        check_interval: Duration::from_millis(100),
        timeout: Duration::from_millis(500),
        enable_detailed_checks: true,
        enable_system_metrics: true,
        ..Default::default()
    };

    let registry = Arc::new(MetricsRegistry::new());
    let manager = EnhancedHealthCheckManager::new(config).with_metrics(registry.clone());

    // 注册模拟健康检查器
    let checker1 = Arc::new(MockHealthChecker::new("service1".to_string(), Duration::from_millis(50)));
    let checker2 = Arc::new(MockHealthChecker::new("service2".to_string(), Duration::from_millis(100)));
    let checker3 = Arc::new(MockHealthChecker::new("service3".to_string(), Duration::from_millis(150)));

    manager.register_checker(checker1.clone()).await;
    manager.register_checker(checker2.clone()).await;
    manager.register_checker(checker3.clone()).await;

    assert_eq!(manager.get_checker_count().await, 3);

    // 执行健康检查
    let status = manager.check_all().await;
    assert_eq!(status.overall_status, HealthStatus::Healthy);
    assert_eq!(status.healthy_count, 3);
    assert_eq!(status.unhealthy_count, 0);

    // 设置一个检查器失败
    checker2.set_should_fail(true);

    let status = manager.check_all().await;
    assert_eq!(status.overall_status, HealthStatus::Unhealthy);
    assert_eq!(status.healthy_count, 2);
    assert_eq!(status.unhealthy_count, 1);

    // 测试单个组件检查
    let component_result = manager.check_component("service1").await;
    assert!(component_result.is_some());
    assert!(component_result.unwrap().is_healthy());

    let component_result = manager.check_component("service2").await;
    assert!(component_result.is_some());
    assert!(component_result.unwrap().is_unhealthy());

    // 测试不存在的组件
    let component_result = manager.check_component("nonexistent").await;
    assert!(component_result.is_none());

    // 启动定期检查
    manager.start_periodic_checks().await.unwrap();
    assert!(manager.is_running());

    // 等待几个检查周期
    sleep(Duration::from_millis(350)).await;

    // 获取最后的检查结果
    let last_check = manager.get_last_check().await;
    assert!(last_check.is_some());

    let last_status = last_check.unwrap();
    assert_eq!(last_status.components.len(), 3);

    // 停止定期检查
    manager.stop_periodic_checks();
    assert!(!manager.is_running());

    // 验证健康检查指标
    let health_counter = registry.get_counter("health_checks_total").await;
    if let Some(counter) = health_counter {
        assert!(counter.get() > 0);
        println!("健康检查执行次数: {}", counter.get());
    }

    println!("最后健康检查状态: {:?}", last_status.overall_status);
}

#[tokio::test]
async fn test_health_check_timeout_handling() {
    let config = HealthCheckConfig {
        check_interval: Duration::from_millis(200),
        timeout: Duration::from_millis(100), // 短超时时间
        ..Default::default()
    };

    let manager = EnhancedHealthCheckManager::new(config);

    // 注册一个响应慢的检查器
    let slow_checker = Arc::new(MockHealthChecker::new(
        "slow_service".to_string(),
        Duration::from_millis(200), // 超过超时时间
    ));

    manager.register_checker(slow_checker).await;

    // 执行健康检查
    let status = manager.check_all().await;
    assert_eq!(status.overall_status, HealthStatus::Unhealthy);
    assert_eq!(status.unhealthy_count, 1);

    // 验证超时消息
    let component = status.get_component_status("slow_service").unwrap();
    assert!(component.message.contains("timeout"));

    println!("超时检查结果: {:?}", component);
}

#[tokio::test]
async fn test_correlation_context_propagation() {
    let config = LoggingConfig {
        level: "info".to_string(),
        enable_correlation: true,
        ..Default::default()
    };

    let logging_system = EnhancedLoggingSystem::init(config).unwrap();

    // 生成关联ID
    let request_id = logging_system.generate_request_id().await;
    let trace_id = logging_system.generate_trace_id().await;

    assert!(!request_id.is_empty());
    assert!(!trace_id.is_empty());

    // 获取关联上下文
    let context = logging_system.get_correlation_context().await;
    assert_eq!(context.request_id, Some(request_id.clone()));
    assert_eq!(context.trace_id, Some(trace_id.clone()));

    // 创建带有关联上下文的span
    let span = logging_system.create_span("test_correlation").await;
    let _enter = span.enter();

    tracing::info!("测试关联上下文传播");

    // 验证关联字段
    let fields = context.to_fields();
    assert!(fields.contains_key("request_id"));
    assert!(fields.contains_key("trace_id"));
    assert_eq!(fields.get("request_id"), Some(&request_id));
    assert_eq!(fields.get("trace_id"), Some(&trace_id));

    println!("关联上下文字段: {:?}", fields);
}

#[tokio::test]
async fn test_metrics_export_formats() {
    let registry = Arc::new(MetricsRegistry::new());

    // 创建各种类型的指标
    let counter = registry.register_counter(
        "export_test_counter".to_string(),
        HashMap::from([
            ("service".to_string(), "test".to_string()),
            ("version".to_string(), "1.0".to_string()),
        ]),
    ).await;

    let gauge = registry.register_gauge(
        "export_test_gauge".to_string(),
        HashMap::from([("unit".to_string(), "bytes".to_string())]),
    ).await;

    let histogram = registry.register_histogram(
        "export_test_histogram".to_string(),
        vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0],
        HashMap::from([("operation".to_string(), "test".to_string())]),
    ).await;

    let summary = registry.register_summary(
        "export_test_summary".to_string(),
        1000,
        HashMap::new(),
    ).await;

    // 添加一些数据
    counter.add(42);
    gauge.set(1024);
    
    histogram.observe(0.3);
    histogram.observe(1.5);
    histogram.observe(3.0);
    histogram.observe(7.0);

    summary.observe(0.1).await;
    summary.observe(0.5).await;
    summary.observe(1.2).await;
    summary.observe(2.8).await;

    // 测试Prometheus格式导出
    let prometheus_export = registry.export_prometheus().await;
    
    // 验证Prometheus格式
    assert!(prometheus_export.contains("# TYPE export_test_counter counter"));
    assert!(prometheus_export.contains("# TYPE export_test_gauge gauge"));
    assert!(prometheus_export.contains("# TYPE export_test_histogram histogram"));
    
    assert!(prometheus_export.contains("export_test_counter{service=\"test\",version=\"1.0\"} 42"));
    assert!(prometheus_export.contains("export_test_gauge{unit=\"bytes\"} 1024"));
    assert!(prometheus_export.contains("export_test_histogram_bucket"));
    assert!(prometheus_export.contains("export_test_histogram_sum"));
    assert!(prometheus_export.contains("export_test_histogram_count"));

    // 测试JSON格式导出
    let json_export = registry.export_json().await.unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_export).unwrap();
    
    // 验证JSON结构
    assert!(json_value["counters"]["export_test_counter"].is_object());
    assert!(json_value["gauges"]["export_test_gauge"].is_object());
    assert!(json_value["histograms"]["export_test_histogram"].is_object());
    assert!(json_value["summaries"]["export_test_summary"].is_object());

    // 验证数据值
    assert_eq!(json_value["counters"]["export_test_counter"]["value"], 42);
    assert_eq!(json_value["gauges"]["export_test_gauge"]["value"], 1024);
    assert_eq!(json_value["histograms"]["export_test_histogram"]["count"], 4);

    println!("Prometheus导出格式:\n{}", prometheus_export);
    println!("JSON导出格式:\n{}", json_export);
}

#[tokio::test]
async fn test_integrated_monitoring_system() {
    // 创建完整的监控系统
    let registry = Arc::new(MetricsRegistry::new());
    
    // 初始化日志系统
    let logging_config = LoggingConfig {
        level: "info".to_string(),
        enable_correlation: true,
        service_name: "integrated-test".to_string(),
        ..Default::default()
    };
    let logging_system = EnhancedLoggingSystem::init(logging_config).unwrap();

    // 初始化性能监控
    let monitor = PerformanceMonitor::with_async_collector(
        registry.clone(),
        Duration::from_millis(50),
    );
    monitor.init_standard_metrics().await;
    monitor.start_collection().await.unwrap();

    // 初始化健康检查
    let health_config = HealthCheckConfig {
        check_interval: Duration::from_millis(100),
        ..Default::default()
    };
    let health_manager = EnhancedHealthCheckManager::new(health_config)
        .with_metrics(registry.clone());

    // 注册健康检查器
    let checker = Arc::new(MockHealthChecker::new("integrated_service".to_string(), Duration::from_millis(10)));
    health_manager.register_checker(checker).await;

    // 启动健康检查
    health_manager.start_periodic_checks().await.unwrap();

    // 设置关联上下文
    let request_id = logging_system.generate_request_id().await;
    let correlation = CorrelationContext::new()
        .with_request_id(request_id.clone())
        .with_task_id("integration-test-task".to_string());
    logging_system.set_correlation_context(correlation).await;

    // 模拟一些系统活动
    let span = logging_system.create_span("integration_test").await;
    let _enter = span.enter();

    tracing::info!("开始集成测试");

    // 记录一些指标
    monitor.record_http_request("GET", 200, Duration::from_millis(100)).await;
    monitor.record_task_processing(Duration::from_secs(2), true).await;
    monitor.update_active_tasks(5).await;

    // 等待系统运行
    sleep(Duration::from_millis(300)).await;

    tracing::info!("集成测试运行中");

    // 检查健康状态
    let health_status = health_manager.check_all().await;
    assert!(health_status.is_healthy());

    // 获取指标
    let metrics_json = registry.export_json().await.unwrap();
    assert!(metrics_json.contains("http_requests_total"));
    assert!(metrics_json.contains("tasks_processed_total"));

    tracing::info!("集成测试完成");

    // 清理
    monitor.stop_collection();
    health_manager.stop_periodic_checks();

    println!("集成测试请求ID: {}", request_id);
    println!("健康状态: {:?}", health_status.overall_status);
    println!("指标摘要: {} 个指标类型", serde_json::from_str::<serde_json::Value>(&metrics_json).unwrap().as_object().unwrap().len());
}