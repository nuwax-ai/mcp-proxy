use std::sync::Arc;
use std::time::{Duration, Instant};

use super::retry::{ReconnectAttemptError, RetryState};
use super::{RemoteProtocolAdapter, RuntimeOptions};
use crate::client::core::common::{connect_with_timeout, monitor_connection_health};
use crate::client::support::{classify_error, print_diagnostic_report, summarize_error};
use crate::proxy::McpClientConfig;

pub(super) async fn run_watchdog<A>(
    adapter: A,
    handler: Arc<A::Handler>,
    config: McpClientConfig,
    options: RuntimeOptions,
    initially_connected: bool,
) where
    A: RemoteProtocolAdapter,
{
    const EVENT_DISCONNECTED: &str = "EVENT_DISCONNECTED";
    const EVENT_RECONNECTED: &str = "EVENT_RECONNECTED";

    let protocol = adapter.protocol_name();
    let mut retry = RetryState::new(options.retries);

    if initially_connected {
        let started = Instant::now();
        let reason = monitor_connection_health(
            handler.as_ref(),
            options.ping_interval,
            options.ping_timeout,
            options.quiet,
            protocol,
        )
        .await;
        adapter.disconnect(handler.as_ref());
        report_disconnect(
            protocol,
            &config.url,
            started.elapsed(),
            &reason,
            options.diagnostic,
            options.quiet,
            EVENT_DISCONNECTED,
        );
    } else {
        tracing::info!(protocol, "No initial backend; entering reconnect loop");
    }

    loop {
        let attempt = retry.failures.saturating_add(1);
        if !options.quiet {
            eprintln!("🔗 Reconnecting (attempt #{attempt})...");
        }

        let connect_started = Instant::now();
        match reconnect_once(&adapter, &config, &handler).await {
            Ok(()) => {
                tracing::info!(
                    protocol,
                    elapsed = ?connect_started.elapsed(),
                    "Backend reconnected"
                );
                retry.reset();
                if !options.quiet {
                    eprintln!("✅ [{EVENT_RECONNECTED}] Reconnected, proxy service resumed");
                }

                let started = Instant::now();
                let reason = monitor_connection_health(
                    handler.as_ref(),
                    options.ping_interval,
                    options.ping_timeout,
                    options.quiet,
                    protocol,
                )
                .await;
                adapter.disconnect(handler.as_ref());
                report_disconnect(
                    protocol,
                    &config.url,
                    started.elapsed(),
                    &reason,
                    options.diagnostic,
                    options.quiet,
                    EVENT_DISCONNECTED,
                );
            }
            Err(attempt_error) => {
                adapter.disconnect(handler.as_ref());
                let error = attempt_error.into_anyhow();
                let error_type = classify_error(&error);
                tracing::error!(
                    protocol,
                    %error_type,
                    error = %summarize_error(&error),
                    elapsed = ?connect_started.elapsed(),
                    "Backend reconnect failed"
                );

                if retry.record_failure() {
                    report_retry_exhausted(protocol, &config.url, &options, &error_type, &error);
                    break;
                }

                report_retry_backoff(&options, &retry, &error_type, &error);
            }
        }

        tokio::time::sleep(retry.current_delay()).await;
        retry.advance_delay();
    }

    tracing::info!(protocol, "Remote watchdog exited");
}

async fn reconnect_once<A>(
    adapter: &A,
    config: &McpClientConfig,
    handler: &Arc<A::Handler>,
) -> std::result::Result<(), ReconnectAttemptError>
where
    A: RemoteProtocolAdapter,
{
    let connection = connect_with_timeout(adapter.connect_reconnect(config.clone(), handler))
        .await
        .map_err(ReconnectAttemptError::Connect)?;
    adapter
        .install_reconnected(connection, handler)
        .await
        .map_err(ReconnectAttemptError::Install)
}

fn report_retry_exhausted(
    protocol: &str,
    url: &str,
    options: &RuntimeOptions,
    error_type: &str,
    error: &anyhow::Error,
) {
    if !options.quiet {
        eprintln!(
            "❌ Connection failed, max retries reached ({})",
            options.retries
        );
        eprintln!("   Error type: {error_type}");
        eprintln!("   Error detail: {error}");
    }
    print_diagnostic_report(
        protocol,
        url,
        0,
        "Connection failed: max retries reached",
        Some(error_type),
        options.diagnostic,
    );
}

fn report_retry_backoff(
    options: &RuntimeOptions,
    retry: &RetryState,
    error_type: &str,
    error: &anyhow::Error,
) {
    const EVENT_RETRY_BACKOFF: &str = "EVENT_RETRY_BACKOFF";

    if options.quiet {
        return;
    }

    let retry_position = if retry.max_failures == 0 {
        format!("attempt #{}", retry.failures)
    } else {
        format!("{}/{}", retry.failures, retry.max_failures)
    };
    eprintln!(
        "⚠️ [{EVENT_RETRY_BACKOFF}] Connection failed [{error_type}]: {}; retrying in {}s ({retry_position})...",
        summarize_error(error),
        retry.current_delay().as_secs()
    );
    if options.verbose {
        eprintln!("   Full error: {error}");
    }
}

fn report_disconnect(
    protocol: &str,
    url: &str,
    alive: Duration,
    reason: &str,
    diagnostic: bool,
    quiet: bool,
    event: &str,
) {
    tracing::warn!(protocol, reason, "Backend connection disconnected");
    if !quiet {
        eprintln!("⚠️ [{event}] Connection disconnected: {reason}");
    }
    print_diagnostic_report(protocol, url, alive.as_secs(), reason, None, diagnostic);
}
