//! OpenTelemetry 追踪模块
//!
//! 提供统一的分布式追踪初始化接口，支持 OTLP (Jaeger) exporter。
//!
//! # Feature Flags
//! - `telemetry`: 基础 OpenTelemetry 支持
//! - `otlp`: OTLP exporter 支持（用于 Jaeger）
//!
//! # 使用示例
//!
//! ```ignore
//! use mcp_common::{TracingConfig, init_tracing};
//!
//! let config = TracingConfig::new("my-service")
//!     .with_otlp("http://localhost:4317")
//!     .with_version("1.0.0");
//!
//! let _guard = init_tracing(&config)?;
//! // guard 保持存活期间，追踪数据会被发送到 OTLP endpoint
//! ```

use anyhow::Result;

/// 追踪配置
#[derive(Debug, Clone, Default)]
pub struct TracingConfig {
    /// 服务名称
    pub service_name: String,
    /// 服务版本
    pub service_version: Option<String>,
    /// OTLP 端点 (如 http://localhost:4317)
    pub otlp_endpoint: Option<String>,
    /// 采样率 (0.0 - 1.0)，默认为 1.0（全部采样）
    pub sample_ratio: Option<f64>,
}

impl TracingConfig {
    /// 创建新的追踪配置
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            ..Default::default()
        }
    }

    /// 设置 OTLP 端点
    pub fn with_otlp(mut self, endpoint: impl Into<String>) -> Self {
        self.otlp_endpoint = Some(endpoint.into());
        self
    }

    /// 设置服务版本
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// 设置采样率
    pub fn with_sample_ratio(mut self, ratio: f64) -> Self {
        self.sample_ratio = Some(ratio.clamp(0.0, 1.0));
        self
    }
}

/// 初始化追踪系统（启用 OTLP feature 时）
#[cfg(feature = "otlp")]
pub fn init_tracing(config: &TracingConfig) -> Result<TracingGuard> {
    use opentelemetry::KeyValue;
    use opentelemetry::global;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};

    let mut provider_builder = SdkTracerProvider::builder();

    // 配置采样率
    if let Some(ratio) = config.sample_ratio {
        provider_builder = provider_builder.with_sampler(Sampler::TraceIdRatioBased(ratio));
    }

    // 配置 OTLP exporter
    if let Some(endpoint) = &config.otlp_endpoint {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()?;

        provider_builder = provider_builder.with_batch_exporter(exporter);
        tracing::info!(endpoint = %endpoint, "OTLP exporter configured");
    }

    // 配置资源属性
    let mut attributes = vec![KeyValue::new("service.name", config.service_name.clone())];

    if let Some(version) = &config.service_version {
        attributes.push(KeyValue::new("service.version", version.clone()));
    }

    let resource = Resource::builder().with_attributes(attributes).build();
    provider_builder = provider_builder.with_resource(resource);

    let provider = provider_builder.build();
    global::set_tracer_provider(provider.clone());

    tracing::info!(
        service_name = %config.service_name,
        "OpenTelemetry tracer provider initialized"
    );

    Ok(TracingGuard {
        provider: Some(provider),
    })
}

/// 无 OTLP 时的空实现
#[cfg(not(feature = "otlp"))]
pub fn init_tracing(_config: &TracingConfig) -> Result<TracingGuard> {
    tracing::debug!("OTLP feature not enabled, skipping tracer initialization");
    Ok(TracingGuard { provider: None })
}

/// 创建 tracing-opentelemetry layer
///
/// 此 layer 可以添加到 tracing_subscriber 中，将 tracing 的 span 和事件
/// 转发到 OpenTelemetry。
#[cfg(feature = "telemetry")]
pub fn create_otel_layer() -> impl tracing_subscriber::Layer<tracing_subscriber::Registry> {
    tracing_opentelemetry::layer()
}

/// 追踪守卫 - Drop 时自动关闭 tracer provider
///
/// 必须保持此守卫存活，否则追踪数据可能不会被正确发送。
pub struct TracingGuard {
    #[cfg(feature = "otlp")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(not(feature = "otlp"))]
    #[allow(dead_code)]
    provider: Option<()>,
}

impl TracingGuard {
    /// 检查追踪是否已初始化
    pub fn is_initialized(&self) -> bool {
        self.provider.is_some()
    }
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        #[cfg(feature = "otlp")]
        if let Some(provider) = self.provider.take() {
            tracing::info!("Shutting down OpenTelemetry tracer provider");
            if let Err(e) = provider.shutdown() {
                tracing::warn!("Failed to shutdown tracer provider: {}", e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_config_builder() {
        let config = TracingConfig::new("test-service")
            .with_otlp("http://localhost:4317")
            .with_version("1.0.0")
            .with_sample_ratio(0.5);

        assert_eq!(config.service_name, "test-service");
        assert_eq!(
            config.otlp_endpoint,
            Some("http://localhost:4317".to_string())
        );
        assert_eq!(config.service_version, Some("1.0.0".to_string()));
        assert_eq!(config.sample_ratio, Some(0.5));
    }

    #[test]
    fn test_sample_ratio_clamping() {
        let config = TracingConfig::new("test").with_sample_ratio(1.5);
        assert_eq!(config.sample_ratio, Some(1.0));

        let config = TracingConfig::new("test").with_sample_ratio(-0.5);
        assert_eq!(config.sample_ratio, Some(0.0));
    }
}
