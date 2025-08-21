use crate::utils::health_check::{HealthCheckResult, HealthStatus, SystemHealthStatus};
use crate::utils::logging::{LogLevel, StructuredLogger};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

/// 告警级别
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
    Emergency,
}

impl std::fmt::Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertLevel::Info => write!(f, "INFO"),
            AlertLevel::Warning => write!(f, "WARNING"),
            AlertLevel::Critical => write!(f, "CRITICAL"),
            AlertLevel::Emergency => write!(f, "EMERGENCY"),
        }
    }
}

impl From<HealthStatus> for AlertLevel {
    fn from(status: HealthStatus) -> Self {
        match status {
            HealthStatus::Healthy => AlertLevel::Info,
            HealthStatus::Degraded => AlertLevel::Warning,
            HealthStatus::Unhealthy => AlertLevel::Critical,
            HealthStatus::Unknown => AlertLevel::Warning,
        }
    }
}

/// 告警规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub component: Option<String>, // None表示适用于所有组件
    pub condition: AlertCondition,
    pub level: AlertLevel,
    pub cooldown: Duration,
    pub enabled: bool,
}

impl AlertRule {
    pub fn new(
        id: String,
        name: String,
        description: String,
        condition: AlertCondition,
        level: AlertLevel,
    ) -> Self {
        Self {
            id,
            name,
            description,
            component: None,
            condition,
            level,
            cooldown: Duration::from_secs(300), // 默认5分钟冷却
            enabled: true,
        }
    }

    pub fn with_component(mut self, component: String) -> Self {
        self.component = Some(component);
        self
    }

    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// 检查规则是否匹配给定的健康检查结果
    pub fn matches(&self, result: &HealthCheckResult) -> bool {
        if !self.enabled {
            return false;
        }

        // 检查组件匹配
        if let Some(ref component) = self.component {
            if &result.component != component {
                return false;
            }
        }

        // 检查条件匹配
        self.condition.evaluate(result)
    }

    /// 检查规则是否匹配系统健康状态
    pub fn matches_system(&self, status: &SystemHealthStatus) -> bool {
        if !self.enabled {
            return false;
        }

        // 如果指定了组件，检查该组件
        if let Some(ref component) = self.component {
            if let Some(component_result) = status.get_component_status(component) {
                return self.condition.evaluate(component_result);
            }
            return false;
        }

        // 否则检查整体状态
        self.condition.evaluate_system(status)
    }
}

/// 告警条件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertCondition {
    /// 健康状态等于指定状态
    HealthStatusEquals(HealthStatus),
    /// 健康状态不等于指定状态
    HealthStatusNotEquals(HealthStatus),
    /// 响应时间超过阈值（毫秒）
    ResponseTimeExceeds(u64),
    /// 连续失败次数超过阈值
    ConsecutiveFailuresExceeds(u32),
    /// 错误率超过阈值（百分比）
    ErrorRateExceeds(f64),
    /// 自定义条件（使用详细信息中的键值对）
    DetailContains(String, String),
    /// 详细信息中的数值超过阈值
    DetailValueExceeds(String, f64),
    /// 组合条件（AND）
    And(Vec<AlertCondition>),
    /// 组合条件（OR）
    Or(Vec<AlertCondition>),
}

impl AlertCondition {
    /// 评估条件是否满足
    pub fn evaluate(&self, result: &HealthCheckResult) -> bool {
        match self {
            AlertCondition::HealthStatusEquals(status) => result.status == *status,
            AlertCondition::HealthStatusNotEquals(status) => result.status != *status,
            AlertCondition::ResponseTimeExceeds(threshold) => result.response_time_ms > *threshold,
            AlertCondition::DetailContains(key, value) => result.details.get(key) == Some(value),
            AlertCondition::DetailValueExceeds(key, threshold) => result
                .details
                .get(key)
                .and_then(|v| v.parse::<f64>().ok())
                .is_some_and(|v| v > *threshold),
            AlertCondition::And(conditions) => conditions.iter().all(|c| c.evaluate(result)),
            AlertCondition::Or(conditions) => conditions.iter().any(|c| c.evaluate(result)),
            // 这些条件需要历史数据，暂时返回false
            AlertCondition::ConsecutiveFailuresExceeds(_) => false,
            AlertCondition::ErrorRateExceeds(_) => false,
        }
    }

    /// 评估系统级条件
    pub fn evaluate_system(&self, status: &SystemHealthStatus) -> bool {
        match self {
            AlertCondition::HealthStatusEquals(health_status) => {
                status.overall_status == *health_status
            }
            AlertCondition::HealthStatusNotEquals(health_status) => {
                status.overall_status != *health_status
            }
            AlertCondition::ErrorRateExceeds(threshold) => {
                let total = status.components.len() as f64;
                if total == 0.0 {
                    return false;
                }
                let error_rate = (status.unhealthy_count as f64 / total) * 100.0;
                error_rate > *threshold
            }
            AlertCondition::And(conditions) => conditions.iter().all(|c| c.evaluate_system(status)),
            AlertCondition::Or(conditions) => conditions.iter().any(|c| c.evaluate_system(status)),
            _ => false, // 其他条件不适用于系统级评估
        }
    }
}

/// 告警事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub level: AlertLevel,
    pub component: Option<String>,
    pub message: String,
    pub details: HashMap<String, String>,
    pub timestamp: u64,
    pub resolved: bool,
    pub resolved_at: Option<u64>,
}

impl AlertEvent {
    pub fn new(
        rule: &AlertRule,
        component: Option<String>,
        message: String,
        details: HashMap<String, String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            level: rule.level.clone(),
            component,
            message,
            details,
            timestamp: SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            resolved: false,
            resolved_at: None,
        }
    }

    pub fn resolve(&mut self) {
        self.resolved = true;
        self.resolved_at = Some(
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
    }

    pub fn is_active(&self) -> bool {
        !self.resolved
    }

    pub fn duration(&self) -> Option<Duration> {
        if let Some(resolved_at) = self.resolved_at {
            Some(Duration::from_secs(resolved_at - self.timestamp))
        } else {
            Some(Duration::from_secs(
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    - self.timestamp,
            ))
        }
    }
}

/// 告警通知器trait
#[async_trait::async_trait]
pub trait AlertNotifier: Send + Sync {
    async fn send_alert(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn send_resolution(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn name(&self) -> &str;
}

/// 日志告警通知器
pub struct LogAlertNotifier {
    logger: Arc<StructuredLogger>,
}

impl LogAlertNotifier {
    pub fn new(logger: Arc<StructuredLogger>) -> Self {
        Self { logger }
    }
}

#[async_trait::async_trait]
impl AlertNotifier for LogAlertNotifier {
    async fn send_alert(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let log_level = match event.level {
            AlertLevel::Info => LogLevel::Info,
            AlertLevel::Warning => LogLevel::Warn,
            AlertLevel::Critical => LogLevel::Error,
            AlertLevel::Emergency => LogLevel::Error,
        };

        let mut fields = HashMap::new();
        fields.insert("alert_id".to_string(), event.id.clone());
        fields.insert("rule_id".to_string(), event.rule_id.clone());
        fields.insert("rule_name".to_string(), event.rule_name.clone());
        fields.insert("alert_level".to_string(), event.level.to_string());

        if let Some(ref component) = event.component {
            fields.insert("component".to_string(), component.clone());
        }

        for (k, v) in &event.details {
            fields.insert(format!("detail_{k}"), v.clone());
        }

        let entry = crate::utils::logging::LogEntry::new(
            log_level,
            format!("ALERT: {}", event.message),
            "alerting".to_string(),
        )
        .with_field("alert_id", &event.id)
        .with_field("rule_id", &event.rule_id)
        .with_field("rule_name", &event.rule_name)
        .with_field("alert_level", event.level.to_string());

        let mut final_entry = entry;
        for (k, v) in &fields {
            final_entry = final_entry.with_field(k, v);
        }

        self.logger.log(final_entry).await;

        Ok(())
    }

    async fn send_resolution(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut fields = HashMap::new();
        fields.insert("alert_id".to_string(), event.id.clone());
        fields.insert("rule_id".to_string(), event.rule_id.clone());
        fields.insert("rule_name".to_string(), event.rule_name.clone());
        fields.insert("alert_level".to_string(), event.level.to_string());

        if let Some(duration) = event.duration() {
            fields.insert(
                "duration_seconds".to_string(),
                duration.as_secs().to_string(),
            );
        }

        if let Some(ref component) = event.component {
            fields.insert("component".to_string(), component.clone());
        }

        let entry = crate::utils::logging::LogEntry::new(
            LogLevel::Info,
            format!("ALERT RESOLVED: {}", event.message),
            "alerting".to_string(),
        )
        .with_field("alert_id", &event.id)
        .with_field("rule_id", &event.rule_id)
        .with_field("rule_name", &event.rule_name)
        .with_field("alert_level", event.level.to_string());

        let mut final_entry = entry;
        for (k, v) in &fields {
            final_entry = final_entry.with_field(k, v);
        }

        self.logger.log(final_entry).await;

        Ok(())
    }

    fn name(&self) -> &str {
        "log"
    }
}

/// 控制台告警通知器
#[derive(Debug)]
pub struct ConsoleAlertNotifier;

impl Default for ConsoleAlertNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleAlertNotifier {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl AlertNotifier for ConsoleAlertNotifier {
    async fn send_alert(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let color_code = match event.level {
            AlertLevel::Info => "\x1b[36m",      // 青色
            AlertLevel::Warning => "\x1b[33m",   // 黄色
            AlertLevel::Critical => "\x1b[31m",  // 红色
            AlertLevel::Emergency => "\x1b[35m", // 紫色
        };
        let reset_code = "\x1b[0m";

        println!(
            "{}🚨 ALERT [{}] {}: {}{}\n   Rule: {} ({})\n   Component: {}\n   Time: {}",
            color_code,
            event.level,
            event.component.as_deref().unwrap_or("SYSTEM"),
            event.message,
            reset_code,
            event.rule_name,
            event.rule_id,
            event.component.as_deref().unwrap_or("N/A"),
            chrono::DateTime::from_timestamp(event.timestamp as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        );

        if !event.details.is_empty() {
            println!("   Details:");
            for (key, value) in &event.details {
                println!("     {key}: {value}");
            }
        }
        println!();

        Ok(())
    }

    async fn send_resolution(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let duration_str = event
            .duration()
            .map(|d| format!("{:.1}s", d.as_secs_f64()))
            .unwrap_or_else(|| "Unknown".to_string());

        println!(
            "\x1b[32m✅ ALERT RESOLVED [{}] {}: {}\x1b[0m\n   Rule: {} ({})\n   Duration: {}\n",
            event.level,
            event.component.as_deref().unwrap_or("SYSTEM"),
            event.message,
            event.rule_name,
            event.rule_id,
            duration_str
        );

        Ok(())
    }

    fn name(&self) -> &str {
        "console"
    }
}

/// Webhook告警通知器
#[derive(Debug)]
pub struct WebhookAlertNotifier {
    webhook_url: String,
    client: reqwest::Client,
}

impl WebhookAlertNotifier {
    pub fn new(webhook_url: String) -> Self {
        Self {
            webhook_url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl AlertNotifier for WebhookAlertNotifier {
    async fn send_alert(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "type": "alert",
            "event": event
        });

        let response = self
            .client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(
                format!("Webhook request failed with status: {}", response.status()).into(),
            );
        }

        Ok(())
    }

    async fn send_resolution(
        &self,
        event: &AlertEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "type": "resolution",
            "event": event
        });

        let response = self
            .client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(
                format!("Webhook request failed with status: {}", response.status()).into(),
            );
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "webhook"
    }
}

/// 告警历史记录
#[derive(Debug)]
struct AlertHistory {
    rule_id: String,
    last_triggered: Option<Instant>,
    consecutive_failures: u32,
    total_triggers: u64,
}

impl AlertHistory {
    fn new(rule_id: String) -> Self {
        Self {
            rule_id,
            last_triggered: None,
            consecutive_failures: 0,
            total_triggers: 0,
        }
    }

    fn can_trigger(&self, cooldown: Duration) -> bool {
        match self.last_triggered {
            Some(last) => last.elapsed() >= cooldown,
            None => true,
        }
    }

    fn record_trigger(&mut self) {
        self.last_triggered = Some(Instant::now());
        self.consecutive_failures += 1;
        self.total_triggers += 1;
    }

    fn reset_failures(&mut self) {
        self.consecutive_failures = 0;
    }
}

/// 告警管理器
pub struct AlertManager {
    rules: Arc<RwLock<HashMap<String, AlertRule>>>,
    notifiers: Arc<RwLock<Vec<Arc<dyn AlertNotifier>>>>,
    active_alerts: Arc<RwLock<HashMap<String, AlertEvent>>>,
    alert_history: Arc<RwLock<HashMap<String, AlertHistory>>>,
    event_sender: mpsc::UnboundedSender<AlertEvent>,
    event_receiver: Arc<RwLock<Option<mpsc::UnboundedReceiver<AlertEvent>>>>,
}

impl AlertManager {
    pub fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            rules: Arc::new(RwLock::new(HashMap::new())),
            notifiers: Arc::new(RwLock::new(Vec::new())),
            active_alerts: Arc::new(RwLock::new(HashMap::new())),
            alert_history: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            event_receiver: Arc::new(RwLock::new(Some(event_receiver))),
        }
    }

    /// 添加告警规则
    pub async fn add_rule(&self, rule: AlertRule) {
        let mut rules = self.rules.write().await;
        rules.insert(rule.id.clone(), rule);
    }

    /// 移除告警规则
    pub async fn remove_rule(&self, rule_id: &str) -> bool {
        let mut rules = self.rules.write().await;
        rules.remove(rule_id).is_some()
    }

    /// 获取告警规则
    pub async fn get_rule(&self, rule_id: &str) -> Option<AlertRule> {
        let rules = self.rules.read().await;
        rules.get(rule_id).cloned()
    }

    /// 获取所有告警规则
    pub async fn get_all_rules(&self) -> Vec<AlertRule> {
        let rules = self.rules.read().await;
        rules.values().cloned().collect()
    }

    /// 添加通知器
    pub async fn add_notifier(&self, notifier: Arc<dyn AlertNotifier>) {
        let mut notifiers = self.notifiers.write().await;
        notifiers.push(notifier);
    }

    /// 处理健康检查结果
    pub async fn process_health_check(&self, result: &HealthCheckResult) {
        let rules = self.rules.read().await;

        for rule in rules.values() {
            if rule.matches(result) {
                self.trigger_alert(
                    rule,
                    Some(result.component.clone()),
                    &result.message,
                    result.details.clone(),
                )
                .await;
            }
        }
    }

    /// 处理系统健康状态
    pub async fn process_system_health(&self, status: &SystemHealthStatus) {
        let rules = self.rules.read().await;

        for rule in rules.values() {
            if rule.matches_system(status) {
                let message = format!(
                    "System health issue: {} healthy, {} degraded, {} unhealthy",
                    status.healthy_count, status.degraded_count, status.unhealthy_count
                );

                let mut details = HashMap::new();
                details.insert(
                    "overall_status".to_string(),
                    status.overall_status.to_string(),
                );
                details.insert(
                    "healthy_count".to_string(),
                    status.healthy_count.to_string(),
                );
                details.insert(
                    "degraded_count".to_string(),
                    status.degraded_count.to_string(),
                );
                details.insert(
                    "unhealthy_count".to_string(),
                    status.unhealthy_count.to_string(),
                );
                details.insert(
                    "total_response_time_ms".to_string(),
                    status.total_response_time_ms.to_string(),
                );

                self.trigger_alert(rule, None, &message, details).await;
            }
        }
    }

    /// 触发告警
    async fn trigger_alert(
        &self,
        rule: &AlertRule,
        component: Option<String>,
        message: &str,
        details: HashMap<String, String>,
    ) {
        // 检查冷却时间
        {
            let mut history = self.alert_history.write().await;
            let alert_history = history
                .entry(rule.id.clone())
                .or_insert_with(|| AlertHistory::new(rule.id.clone()));

            if !alert_history.can_trigger(rule.cooldown) {
                return;
            }

            alert_history.record_trigger();
        }

        // 创建告警事件
        let event = AlertEvent::new(rule, component, message.to_string(), details);

        // 存储活跃告警
        {
            let mut active_alerts = self.active_alerts.write().await;
            active_alerts.insert(event.id.clone(), event.clone());
        }

        // 发送事件
        if let Err(e) = self.event_sender.send(event) {
            eprintln!("Failed to send alert event: {e}");
        }
    }

    /// 解决告警
    pub async fn resolve_alert(&self, alert_id: &str) -> bool {
        let mut active_alerts = self.active_alerts.write().await;

        if let Some(mut event) = active_alerts.remove(alert_id) {
            event.resolve();

            // 发送解决事件
            if let Err(e) = self.event_sender.send(event) {
                eprintln!("Failed to send alert resolution event: {e}");
            }

            true
        } else {
            false
        }
    }

    /// 获取活跃告警
    pub async fn get_active_alerts(&self) -> Vec<AlertEvent> {
        let active_alerts = self.active_alerts.read().await;
        active_alerts.values().cloned().collect()
    }

    /// 获取告警统计
    pub async fn get_alert_stats(&self) -> AlertStats {
        let active_alerts = self.active_alerts.read().await;
        let history = self.alert_history.read().await;

        let total_active = active_alerts.len();
        let critical_count = active_alerts
            .values()
            .filter(|a| matches!(a.level, AlertLevel::Critical | AlertLevel::Emergency))
            .count();

        let total_triggered = history.values().map(|h| h.total_triggers).sum();

        AlertStats {
            total_active,
            critical_count,
            total_triggered,
            rules_count: self.rules.read().await.len(),
            notifiers_count: self.notifiers.read().await.len(),
        }
    }

    /// 启动事件处理器
    pub async fn start_event_processor(&self) {
        let notifiers = self.notifiers.clone();
        let receiver = {
            let mut event_receiver = self.event_receiver.write().await;
            event_receiver.take()
        };

        if let Some(mut receiver) = receiver {
            tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    let notifiers_guard = notifiers.read().await;

                    for notifier in notifiers_guard.iter() {
                        let result = if event.resolved {
                            notifier.send_resolution(&event).await
                        } else {
                            notifier.send_alert(&event).await
                        };

                        if let Err(e) = result {
                            eprintln!("Failed to send notification via {}: {}", notifier.name(), e);
                        }
                    }
                }
            });
        }
    }

    /// 创建默认规则
    pub async fn create_default_rules(&self) {
        // 组件不健康规则
        let unhealthy_rule = AlertRule::new(
            "component_unhealthy".to_string(),
            "Component Unhealthy".to_string(),
            "Triggered when a component becomes unhealthy".to_string(),
            AlertCondition::HealthStatusEquals(HealthStatus::Unhealthy),
            AlertLevel::Critical,
        );
        self.add_rule(unhealthy_rule).await;

        // 组件降级规则
        let degraded_rule = AlertRule::new(
            "component_degraded".to_string(),
            "Component Degraded".to_string(),
            "Triggered when a component becomes degraded".to_string(),
            AlertCondition::HealthStatusEquals(HealthStatus::Degraded),
            AlertLevel::Warning,
        );
        self.add_rule(degraded_rule).await;

        // 响应时间过长规则
        let slow_response_rule = AlertRule::new(
            "slow_response".to_string(),
            "Slow Response Time".to_string(),
            "Triggered when response time exceeds 30 seconds".to_string(),
            AlertCondition::ResponseTimeExceeds(30000),
            AlertLevel::Warning,
        );
        self.add_rule(slow_response_rule).await;

        // 系统错误率过高规则
        let high_error_rate_rule = AlertRule::new(
            "high_error_rate".to_string(),
            "High Error Rate".to_string(),
            "Triggered when error rate exceeds 50%".to_string(),
            AlertCondition::ErrorRateExceeds(50.0),
            AlertLevel::Critical,
        );
        self.add_rule(high_error_rate_rule).await;
    }
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 告警统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertStats {
    pub total_active: usize,
    pub critical_count: usize,
    pub total_triggered: u64,
    pub rules_count: usize,
    pub notifiers_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::health_check::HealthCheckResult;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockNotifier {
        name: String,
        alert_count: Arc<AtomicUsize>,
        resolution_count: Arc<AtomicUsize>,
    }

    impl MockNotifier {
        fn new(name: String) -> Self {
            Self {
                name,
                alert_count: Arc::new(AtomicUsize::new(0)),
                resolution_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn get_alert_count(&self) -> usize {
            self.alert_count.load(Ordering::Relaxed)
        }

        fn get_resolution_count(&self) -> usize {
            self.resolution_count.load(Ordering::Relaxed)
        }
    }

    #[async_trait::async_trait]
    impl AlertNotifier for MockNotifier {
        async fn send_alert(
            &self,
            _event: &AlertEvent,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.alert_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn send_resolution(
            &self,
            _event: &AlertEvent,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.resolution_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_alert_rule_matching() {
        let rule = AlertRule::new(
            "test_rule".to_string(),
            "Test Rule".to_string(),
            "Test description".to_string(),
            AlertCondition::HealthStatusEquals(HealthStatus::Unhealthy),
            AlertLevel::Critical,
        );

        let unhealthy_result = HealthCheckResult::new(
            "test_component".to_string(),
            HealthStatus::Unhealthy,
            "Test message".to_string(),
        );

        let healthy_result = HealthCheckResult::new(
            "test_component".to_string(),
            HealthStatus::Healthy,
            "Test message".to_string(),
        );

        assert!(rule.matches(&unhealthy_result));
        assert!(!rule.matches(&healthy_result));
    }

    #[tokio::test]
    async fn test_alert_condition_evaluation() {
        let condition = AlertCondition::And(vec![
            AlertCondition::HealthStatusEquals(HealthStatus::Unhealthy),
            AlertCondition::ResponseTimeExceeds(1000),
        ]);

        let mut result = HealthCheckResult::new(
            "test_component".to_string(),
            HealthStatus::Unhealthy,
            "Test message".to_string(),
        );
        result.response_time_ms = 2000;

        assert!(condition.evaluate(&result));

        result.response_time_ms = 500;
        assert!(!condition.evaluate(&result));
    }

    #[tokio::test]
    async fn test_alert_manager() {
        let manager = AlertManager::new();
        let notifier = Arc::new(MockNotifier::new("test".to_string()));

        manager.add_notifier(notifier.clone()).await;
        manager.start_event_processor().await;

        let rule = AlertRule::new(
            "test_rule".to_string(),
            "Test Rule".to_string(),
            "Test description".to_string(),
            AlertCondition::HealthStatusEquals(HealthStatus::Unhealthy),
            AlertLevel::Critical,
        )
        .with_cooldown(Duration::from_millis(100));

        manager.add_rule(rule).await;

        let unhealthy_result = HealthCheckResult::new(
            "test_component".to_string(),
            HealthStatus::Unhealthy,
            "Test message".to_string(),
        );

        manager.process_health_check(&unhealthy_result).await;

        // 等待事件处理
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(notifier.get_alert_count(), 1);
        assert_eq!(manager.get_active_alerts().await.len(), 1);

        // 测试冷却时间
        manager.process_health_check(&unhealthy_result).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(notifier.get_alert_count(), 1); // 应该还是1，因为冷却时间

        // 等待冷却时间过去
        tokio::time::sleep(Duration::from_millis(100)).await;
        manager.process_health_check(&unhealthy_result).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(notifier.get_alert_count(), 2); // 现在应该是2
    }
}
