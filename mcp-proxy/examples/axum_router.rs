use rmcp::transport::sse_server::SseServer;
use tracing_subscriber::{
    layer::SubscriberExt,
    util::SubscriberInitExt,
    {self},
};

const BIND_ADDRESS: &str = "127.0.0.1:8000";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "debug".to_string().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 使用简化方法启动 SSE 服务器
    let ct = SseServer::serve(BIND_ADDRESS.parse()?)
        .await?
        .with_service_directly(());

    tokio::signal::ctrl_c().await?;
    ct.cancel();
    Ok(())
}
