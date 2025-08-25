use crate::models::Config;
use crate::VoiceCliError;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{Level, Subscriber};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{format::Writer, FormatEvent, FormatFields},
    prelude::*,
    registry::LookupSpan,
    EnvFilter, Layer,
};

/// Structured logging context for cluster operations
#[derive(Debug, Clone)]
pub struct ClusterLoggingContext {
    pub node_id: String,
    pub service_type: String,
    pub cluster_id: Option<String>,
    pub instance_id: String,
}

impl ClusterLoggingContext {
    pub fn new(node_id: String, service_type: String) -> Self {
        Self {
            node_id,
            service_type,
            cluster_id: None,
            instance_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn with_cluster_id(mut self, cluster_id: String) -> Self {
        self.cluster_id = Some(cluster_id);
        self
    }
}

/// Custom JSON formatter for structured logging
pub struct StructuredJsonFormatter {
    context: Arc<ClusterLoggingContext>,
}

impl StructuredJsonFormatter {
    pub fn new(context: Arc<ClusterLoggingContext>) -> Self {
        Self { context }
    }
}

impl<S, N> FormatEvent<S, N> for StructuredJsonFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        let metadata = event.metadata();

        // Create base log entry
        let mut log_entry = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "level": metadata.level().to_string(),
            "target": metadata.target(),
            "node_id": self.context.node_id,
            "service_type": self.context.service_type,
            "instance_id": self.context.instance_id,
        });

        // Add cluster_id if available
        if let Some(cluster_id) = &self.context.cluster_id {
            log_entry["cluster_id"] = json!(cluster_id);
        }

        // Add file and line information for debug builds
        if let Some(file) = metadata.file() {
            log_entry["file"] = json!(file);
        }
        if let Some(line) = metadata.line() {
            log_entry["line"] = json!(line);
        }

        // Collect event fields
        let mut visitor = JsonVisitor::new();
        event.record(&mut visitor);

        // Add message
        if let Some(message) = visitor.message {
            log_entry["message"] = json!(message);
        }

        // Add custom fields
        for (key, value) in visitor.fields {
            log_entry[key] = value;
        }

        // Add span context if available
        if let Some(span) = ctx.lookup_current() {
            let mut span_fields = HashMap::new();

            // Collect span name
            span_fields.insert("name".to_string(), json!(span.name()));

            if !span_fields.is_empty() {
                log_entry["span"] = json!(span_fields);
            }
        }

        writeln!(writer, "{}", log_entry)
    }
}

/// JSON field visitor for collecting event fields
struct JsonVisitor {
    message: Option<String>,
    fields: HashMap<String, serde_json::Value>,
}

impl JsonVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: HashMap::new(),
        }
    }
}

impl tracing::field::Visit for JsonVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value_str = format!("{:?}", value);

        if field.name() == "message" {
            self.message = Some(value_str);
        } else {
            self.fields
                .insert(field.name().to_string(), json!(value_str));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.insert(field.name().to_string(), json!(value));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
}

/// Initialize structured logging with cluster context
pub fn init_structured_logging(
    config: &Config,
    context: ClusterLoggingContext,
) -> crate::Result<()> {
    // Check if logging is already initialized
    if tracing::dispatcher::has_been_set() {
        tracing::debug!("Logging already initialized, skipping");
        return Ok(());
    }

    let context = Arc::new(context);

    // Create logs directory if it doesn't exist
    let log_dir = config.log_dir_path();
    std::fs::create_dir_all(&log_dir)?;

    // Parse log level
    let level = parse_log_level(&config.logging.level)?;

    // Create file appender with rotation
    let file_appender = RollingFileAppender::new(Rotation::DAILY, &log_dir, "voice-cli.log");

    // Determine if we're in production mode (info level or higher)
    let is_production = matches!(level, Level::INFO | Level::WARN | Level::ERROR);

    // Create console layer (human-readable for development, minimal for production)
    let console_layer = if is_production {
        // Production: minimal console output, only warnings and errors
        tracing_subscriber::fmt::layer()
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .with_filter(EnvFilter::from_default_env().add_directive(Level::WARN.into()))
    } else {
        // Development: verbose console output
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_thread_ids(true)
            .with_file(true)
            .with_line_number(true)
            .compact()
            .with_filter(EnvFilter::from_default_env().add_directive(level.into()))
    };

    // Create structured JSON file layer (machine-readable for production)
    let json_layer = tracing_subscriber::fmt::layer()
        .event_format(StructuredJsonFormatter::new(context.clone()))
        .with_writer(file_appender)
        .with_ansi(false)
        .with_filter(EnvFilter::new(&config.logging.level));

    // Initialize subscriber
    tracing_subscriber::registry()
        .with(console_layer)
        .with(json_layer)
        .try_init()
        .map_err(|e| {
            VoiceCliError::Config(format!("Failed to initialize structured logging: {}", e))
        })?;

    tracing::info!(
        node_id = %context.node_id,
        service_type = %context.service_type,
        instance_id = %context.instance_id,
        log_level = %config.logging.level,
        log_dir = ?log_dir,
        production_mode = is_production,
        "Structured logging initialized"
    );

    Ok(())
}

/// Parse log level string to tracing Level
fn parse_log_level(level_str: &str) -> crate::Result<Level> {
    match level_str.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" | "warning" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        _ => Err(VoiceCliError::Config(format!(
            "Invalid log level: {}. Valid levels: trace, debug, info, warn, error",
            level_str
        ))),
    }
}

/// Logging macros with consistent cluster context
#[macro_export]
macro_rules! log_cluster_event {
    ($level:ident, $node_id:expr, $service:expr, $operation:expr, $message:expr) => {
        tracing::$level!(
            node_id = $node_id,
            service_type = $service,
            operation = $operation,
            message = $message
        );
    };
    ($level:ident, $node_id:expr, $service:expr, $operation:expr, $message:expr, $($field:tt)*) => {
        tracing::$level!(
            node_id = $node_id,
            service_type = $service,
            operation = $operation,
            message = $message,
            $($field)*
        );
    };
}

/// Performance metrics logging
#[macro_export]
macro_rules! log_performance_metric {
    ($operation:expr, $duration_ms:expr, $node_id:expr, $service:expr) => {
        tracing::info!(
            node_id = $node_id,
            service_type = $service,
            operation = $operation,
            duration_ms = $duration_ms,
            metric_type = "performance",
            "Operation completed"
        );
    };
    ($operation:expr, $duration_ms:expr, $node_id:expr, $service:expr, $($field:tt)*) => {
        tracing::info!(
            node_id = $node_id,
            service_type = $service,
            operation = $operation,
            duration_ms = $duration_ms,
            metric_type = "performance",
            $($field)*
        );
    };
}

/// Health check logging
#[macro_export]
macro_rules! log_health_event {
    ($status:expr, $node_id:expr, $service:expr, $check_type:expr) => {
        tracing::info!(
            node_id = $node_id,
            service_type = $service,
            health_status = $status,
            check_type = $check_type,
            metric_type = "health",
            "Health check completed"
        );
    };
    ($status:expr, $node_id:expr, $service:expr, $check_type:expr, $($field:tt)*) => {
        tracing::info!(
            node_id = $node_id,
            service_type = $service,
            health_status = $status,
            check_type = $check_type,
            metric_type = "health",
            $($field)*
        );
    };
}

/// Cluster state change logging
#[macro_export]
macro_rules! log_cluster_state_change {
    ($event:expr, $node_id:expr, $service:expr, $old_state:expr, $new_state:expr) => {
        tracing::info!(
            node_id = $node_id,
            service_type = $service,
            event_type = $event,
            old_state = $old_state,
            new_state = $new_state,
            metric_type = "cluster_state",
            "Cluster state changed"
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_logging_context() {
        let context =
            ClusterLoggingContext::new("node-1".to_string(), "task_scheduler".to_string());

        assert_eq!(context.node_id, "node-1");
        assert_eq!(context.service_type, "task_scheduler");
        assert!(context.cluster_id.is_none());
        assert!(!context.instance_id.is_empty());
    }

    #[test]
    fn test_cluster_logging_context_with_cluster_id() {
        let context =
            ClusterLoggingContext::new("node-1".to_string(), "task_scheduler".to_string())
                .with_cluster_id("cluster-123".to_string());

        assert_eq!(context.cluster_id, Some("cluster-123".to_string()));
    }

    #[test]
    fn test_parse_log_level() {
        assert!(matches!(parse_log_level("info"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("INFO"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("debug"), Ok(Level::DEBUG)));
        assert!(matches!(parse_log_level("error"), Ok(Level::ERROR)));
        assert!(parse_log_level("invalid").is_err());
    }
}
