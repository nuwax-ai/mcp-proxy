use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use super::retry::RetryState;
use super::*;
use crate::client::core::discovery_options::ExportTargets;
use mcp_proxy_args::{ImportSource, LoadedJson};

#[derive(Default)]
struct FakeState {
    connects: AtomicUsize,
    reconnects: AtomicUsize,
    installs: AtomicUsize,
    disconnects: AtomicUsize,
    builds: AtomicUsize,
    fallback_builds: AtomicUsize,
    exports: AtomicUsize,
    serves: AtomicUsize,
    initial_fails: AtomicBool,
    reconnect_failures: AtomicUsize,
    install_failures: AtomicUsize,
    panic_reconnect: AtomicBool,
    block_stdio: AtomicBool,
    stdio_finished: tokio::sync::Notify,
}

#[derive(Clone)]
struct FakeAdapter {
    state: Arc<FakeState>,
}

struct FakeHandler {
    available: AtomicBool,
}

impl HealthChecker for FakeHandler {
    fn is_backend_available(&self) -> bool {
        self.available.load(Ordering::Relaxed)
    }

    async fn is_terminated_async(&self) -> bool {
        !self.is_backend_available()
    }
}

#[async_trait]
impl RemoteProtocolAdapter for FakeAdapter {
    type Connection = ();
    type Handler = FakeHandler;
    type ImportedFallback = ();

    fn protocol_name(&self) -> &'static str {
        "Fake"
    }

    fn parse_import(&self, _json: LoadedFallbackJson) -> Result<Self::ImportedFallback> {
        Ok(())
    }

    async fn connect_initial(&self, _config: McpClientConfig) -> Result<Self::Connection> {
        self.state.connects.fetch_add(1, Ordering::Relaxed);
        if self.state.initial_fails.load(Ordering::Relaxed) {
            anyhow::bail!("fake initial connection failure");
        }
        Ok(())
    }

    async fn connect_reconnect(
        &self,
        _config: McpClientConfig,
        _handler: &Arc<Self::Handler>,
    ) -> Result<Self::Connection> {
        self.state.reconnects.fetch_add(1, Ordering::Relaxed);
        assert!(
            !self.state.panic_reconnect.load(Ordering::Relaxed),
            "fake watchdog panic"
        );
        if consume_failure(&self.state.reconnect_failures) {
            anyhow::bail!("fake reconnect failure");
        }
        Ok(())
    }

    async fn export_discovery(&self, _connection: &Self::Connection) -> Result<DiscoveryExport> {
        self.state.exports.fetch_add(1, Ordering::Relaxed);
        Ok(DiscoveryExport {
            initialize_json: "{}".to_string(),
            tools_json: r#"{"tools":[]}"#.to_string(),
        })
    }

    async fn preview_tools(&self, _connection: &Self::Connection) -> Result<Vec<ToolPreview>> {
        Ok(Vec::new())
    }

    async fn build_handler(
        &self,
        connection: Option<Self::Connection>,
        fallback: Option<Self::ImportedFallback>,
        _filter: ToolFilter,
    ) -> Result<PreparedHandler<Self::Handler>> {
        self.state.builds.fetch_add(1, Ordering::Relaxed);
        if fallback.is_some() {
            self.state.fallback_builds.fetch_add(1, Ordering::Relaxed);
        }
        Ok(PreparedHandler {
            handler: Arc::new(FakeHandler {
                available: AtomicBool::new(connection.is_some()),
            }),
            initially_connected: connection.is_some(),
        })
    }

    async fn install_reconnected(
        &self,
        _connection: Self::Connection,
        handler: &Arc<Self::Handler>,
    ) -> Result<()> {
        self.state.installs.fetch_add(1, Ordering::Relaxed);
        if consume_failure(&self.state.install_failures) {
            anyhow::bail!("fake install failure");
        }
        handler.available.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn disconnect(&self, handler: &Self::Handler) {
        self.state.disconnects.fetch_add(1, Ordering::Relaxed);
        handler.available.store(false, Ordering::Relaxed);
    }

    async fn serve_stdio(&self, _handler: Arc<Self::Handler>) -> Result<()> {
        self.state.serves.fetch_add(1, Ordering::Relaxed);
        if self.state.block_stdio.load(Ordering::Relaxed) {
            self.state.stdio_finished.notified().await;
        }
        Ok(())
    }
}

fn consume_failure(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_sub(1)
        })
        .is_ok()
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        retries: 1,
        ping_interval: 0,
        ping_timeout: 1,
        diagnostic: false,
        verbose: false,
        quiet: true,
    }
}

fn imported_mode() -> DiscoveryMode {
    DiscoveryMode::Import(LoadedFallbackJson {
        initialize: LoadedJson {
            json: "{}".to_string(),
            source: ImportSource::Inline,
        },
        tools: LoadedJson {
            json: r#"{"tools":[]}"#.to_string(),
            source: ImportSource::Inline,
        },
    })
}

async fn advance_runtime() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
    }
}

#[tokio::test]
async fn normal_mode_builds_and_serves_handler() {
    let state = Arc::new(FakeState::default());
    run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        DiscoveryMode::Normal,
        ToolFilter::default(),
        options(),
    )
    .await
    .expect("normal runtime");

    assert_eq!(state.connects.load(Ordering::Relaxed), 1);
    assert_eq!(state.builds.load(Ordering::Relaxed), 1);
    assert_eq!(state.serves.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn export_mode_does_not_build_or_serve_stdio() {
    let directory = tempfile::tempdir().expect("tempdir");
    let tools = directory.path().join("tools.json");
    let state = Arc::new(FakeState::default());
    run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        DiscoveryMode::Export(ExportTargets {
            initialize: None,
            tools: Some(tools.clone()),
        }),
        ToolFilter::default(),
        options(),
    )
    .await
    .expect("export runtime");

    assert_eq!(state.exports.load(Ordering::Relaxed), 1);
    assert_eq!(state.builds.load(Ordering::Relaxed), 0);
    assert_eq!(state.serves.load(Ordering::Relaxed), 0);
    assert!(
        std::fs::read_to_string(tools)
            .expect("tools export")
            .ends_with('\n')
    );
}

#[test]
fn retry_state_caps_failures_and_backoff() {
    let mut retry = RetryState::with_delays(2, Duration::from_secs(2), Duration::from_secs(3));
    assert!(!retry.record_failure());
    assert_eq!(retry.current_delay(), Duration::from_secs(2));
    retry.advance_delay();
    assert_eq!(retry.current_delay(), Duration::from_secs(3));
    assert!(retry.record_failure());
    retry.reset();
    assert_eq!(retry.failures, 0);
    assert_eq!(retry.current_delay(), Duration::from_secs(2));
}

#[tokio::test(start_paused = true)]
async fn initial_failure_without_fallback_fails_before_stdio() {
    let state = Arc::new(FakeState::default());
    state.initial_fails.store(true, Ordering::Relaxed);

    let error = run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        DiscoveryMode::Normal,
        ToolFilter::default(),
        options(),
    )
    .await
    .expect_err("initial failures without fallback must fail");

    assert!(
        error
            .to_string()
            .contains("all initial Fake connection attempts")
    );
    assert_eq!(state.connects.load(Ordering::Relaxed), 3);
    assert_eq!(state.serves.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn finite_connect_failures_stop_watchdog_but_not_stdio() {
    let state = Arc::new(FakeState::default());
    state.initial_fails.store(true, Ordering::Relaxed);
    state.reconnect_failures.store(10, Ordering::Relaxed);
    state.block_stdio.store(true, Ordering::Relaxed);
    let mut runtime_options = options();
    runtime_options.retries = 2;
    let runtime = tokio::spawn(run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        imported_mode(),
        ToolFilter::default(),
        runtime_options,
    ));

    advance_runtime().await;
    assert_eq!(state.reconnects.load(Ordering::Relaxed), 2);
    assert!(!runtime.is_finished());
    assert_eq!(state.fallback_builds.load(Ordering::Relaxed), 1);

    state.stdio_finished.notify_waiters();
    runtime
        .await
        .expect("runtime task")
        .expect("runtime result");
}

#[tokio::test(start_paused = true)]
async fn finite_install_failures_share_the_same_retry_limit() {
    let state = Arc::new(FakeState::default());
    state.initial_fails.store(true, Ordering::Relaxed);
    state.install_failures.store(10, Ordering::Relaxed);
    state.block_stdio.store(true, Ordering::Relaxed);
    let mut runtime_options = options();
    runtime_options.retries = 2;
    let runtime = tokio::spawn(run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        imported_mode(),
        ToolFilter::default(),
        runtime_options,
    ));

    advance_runtime().await;
    assert_eq!(state.installs.load(Ordering::Relaxed), 2);
    assert_eq!(state.disconnects.load(Ordering::Relaxed), 2);
    assert!(!runtime.is_finished());

    state.stdio_finished.notify_waiters();
    runtime
        .await
        .expect("runtime task")
        .expect("runtime result");
}

#[tokio::test(start_paused = true)]
async fn unlimited_retries_recover_and_install_backend() {
    let state = Arc::new(FakeState::default());
    state.initial_fails.store(true, Ordering::Relaxed);
    state.reconnect_failures.store(1, Ordering::Relaxed);
    state.block_stdio.store(true, Ordering::Relaxed);
    let mut runtime_options = options();
    runtime_options.retries = 0;
    let runtime = tokio::spawn(run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        imported_mode(),
        ToolFilter::default(),
        runtime_options,
    ));

    advance_runtime().await;
    assert!(state.reconnects.load(Ordering::Relaxed) >= 2);
    assert_eq!(state.installs.load(Ordering::Relaxed), 1);
    assert!(!runtime.is_finished());

    state.stdio_finished.notify_waiters();
    runtime
        .await
        .expect("runtime task")
        .expect("runtime result");
}

#[tokio::test(start_paused = true)]
async fn watchdog_panic_is_contained_until_stdio_finishes() {
    let state = Arc::new(FakeState::default());
    state.initial_fails.store(true, Ordering::Relaxed);
    state.panic_reconnect.store(true, Ordering::Relaxed);
    state.block_stdio.store(true, Ordering::Relaxed);
    let runtime = tokio::spawn(run_remote_mode(
        FakeAdapter {
            state: state.clone(),
        },
        McpClientConfig::new("http://fake"),
        imported_mode(),
        ToolFilter::default(),
        options(),
    ));

    advance_runtime().await;
    assert_eq!(state.reconnects.load(Ordering::Relaxed), 1);
    assert!(!runtime.is_finished());

    state.stdio_finished.notify_waiters();
    runtime
        .await
        .expect("runtime task")
        .expect("runtime result");
}
