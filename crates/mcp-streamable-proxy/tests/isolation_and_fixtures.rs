//! Unit tests for isolation helpers and MCP JSON fixtures.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use mcp_streamable_proxy::{
    BackendConnector, BackendIsolation, BackendRunningService, ProxyHandler, ToolFilter,
    UpstreamPeerRegistry, UpstreamPeerSlot, UrlBackendConnector,
};
use rmcp::model::{CallToolRequestParams, ProgressToken};

struct CountingConnector {
    session_connects: AtomicU64,
}

#[async_trait]
impl BackendConnector for CountingConnector {
    async fn connect_shared(
        &self,
        _registry: Arc<UpstreamPeerRegistry>,
    ) -> anyhow::Result<BackendRunningService> {
        anyhow::bail!("CountingConnector::connect_shared not implemented")
    }

    async fn connect_session(
        &self,
        _slot: UpstreamPeerSlot,
    ) -> anyhow::Result<BackendRunningService> {
        self.session_connects.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("CountingConnector deliberately fails after count")
    }
}

#[test]
fn isolation_defaults() {
    assert_eq!(BackendIsolation::for_url(), BackendIsolation::PerSession);
    assert_eq!(BackendIsolation::for_stdio(), BackendIsolation::Shared);
}

#[test]
fn per_session_handler_starts_disconnected() {
    let connector: Arc<dyn BackendConnector> =
        Arc::new(UrlBackendConnector::new("http://127.0.0.1:9/mcp", None));
    let handler = ProxyHandler::new_per_session(connector, "test".into(), ToolFilter::default());
    assert_eq!(handler.isolation(), BackendIsolation::PerSession);
    assert!(!handler.is_backend_available());
    assert_eq!(handler.get_backend_version(), 0);
}

#[test]
fn two_per_session_handlers_are_independent() {
    let connector: Arc<dyn BackendConnector> = Arc::new(CountingConnector {
        session_connects: AtomicU64::new(0),
    });
    let a = ProxyHandler::new_per_session(connector.clone(), "a".into(), ToolFilter::default());
    let b = ProxyHandler::new_per_session(connector, "b".into(), ToolFilter::default());
    a.swap_backend(None);
    assert!(a.get_backend_version() >= 1);
    assert_eq!(b.get_backend_version(), 0);
}

#[tokio::test]
async fn counting_connector_session_calls() {
    let connector = CountingConnector {
        session_connects: AtomicU64::new(0),
    };
    let slot = UpstreamPeerSlot::new();
    let _ = connector.connect_session(slot.clone()).await;
    let _ = connector.connect_session(slot).await;
    assert_eq!(connector.session_connects.load(Ordering::SeqCst), 2);
}

#[test]
fn mcp_json_fixtures_parse() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/mcp");

    let init: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("initialize_request.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(init["method"], "initialize");

    let call: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tools_call_with_progress.json")).unwrap(),
    )
    .unwrap();
    let params: CallToolRequestParams = serde_json::from_value(call["params"].clone()).unwrap();
    assert_eq!(params.name.as_ref(), "echo");
    let token = params
        .meta
        .as_ref()
        .and_then(|m| m.get_progress_token())
        .expect("progressToken");
    assert_eq!(
        token,
        ProgressToken(rmcp::model::NumberOrString::Number(42))
    );

    let status: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("task_status_notification.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(status["method"], "notifications/tasks/status");
    assert_eq!(status["params"]["taskId"], "task-abc");
}
