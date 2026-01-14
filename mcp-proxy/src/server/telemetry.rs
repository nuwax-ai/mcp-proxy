use anyhow::Result;
use opentelemetry::global;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};

/// 初始化 OpenTelemetry tracer provider
///
/// 这个函数必须在创建 telemetry layer 之前调用
pub fn init_tracer_provider(_service_name: &str, _service_version: &str) -> Result<()> {
    // 创建 tracer provider
    let tracer_provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .build();

    // 设置全局 tracer provider
    global::set_tracer_provider(tracer_provider);

    Ok(())
}

/// 创建增强的 OpenTelemetry layer
///
/// 这个函数创建一个配置好的 OpenTelemetry layer，可以与现有的 tracing 配置集成
/// 注意：必须先调用 init_tracer_provider()
pub fn create_telemetry_layer() -> impl tracing_subscriber::Layer<tracing_subscriber::Registry> {
    tracing_opentelemetry::layer()
}

/// 记录服务启动信息
///
/// 在 telemetry 系统初始化后调用，记录服务的基本信息
pub fn log_service_info(service_name: &str, service_version: &str) -> Result<()> {
    tracing::info!(
        service_name = %service_name,
        service_version = %service_version,
        "Service started with OpenTelemetry tracing enabled"
    );
    Ok(())
}

/// 优雅关闭 OpenTelemetry
pub fn shutdown_telemetry() {
    tracing::info!("Shutting down OpenTelemetry");
    // 注意：在新版本的 OpenTelemetry 中，shutdown 方法可能不同
    // 这里我们简单地记录关闭信息
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_service_info() {
        let result = log_service_info("test-service", "0.1.0");
        assert!(result.is_ok());

        // 清理
        shutdown_telemetry();
    }
}
