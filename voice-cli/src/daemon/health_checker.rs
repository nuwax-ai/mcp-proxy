use crate::log_health_event;
use crate::models::{Config, HealthResponse};
use crate::VoiceCliError;
use std::time::Duration;
use tracing::debug;

/// Handles health checking for the daemon service
pub struct HealthChecker {
    health_url: String,
    client: reqwest::Client,
}

impl HealthChecker {
    pub fn new(config: &Config) -> Self {
        let health_url = format!(
            "http://{}:{}/health",
            config.server.host, config.server.port
        );

        // Create HTTP client with reasonable timeouts
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { health_url, client }
    }

    /// Perform a health check
    pub async fn check_health(&self) -> crate::Result<HealthResponse> {
        let start_time = std::time::Instant::now();
        debug!("Performing health check at: {}", self.health_url);

        let response = self
            .client
            .get(&self.health_url)
            .send()
            .await
            .map_err(|e| {
                log_health_event!(
                    "failed",
                    "daemon",
                    "health_checker",
                    "http_request",
                    error = %e,
                    url = %self.health_url
                );
                VoiceCliError::Daemon(format!("Health check request failed: {}", e))
            })?;

        if !response.status().is_success() {
            log_health_event!(
                "failed",
                "daemon",
                "health_checker",
                "http_response",
                status_code = %response.status(),
                url = %self.health_url
            );
            return Err(VoiceCliError::Daemon(format!(
                "Health check returned status: {}",
                response.status()
            )));
        }

        let health: HealthResponse = response.json().await.map_err(|e| {
            log_health_event!(
                "failed",
                "daemon",
                "health_checker",
                "json_parse",
                error = %e,
                url = %self.health_url
            );
            VoiceCliError::Daemon(format!("Health check response parse error: {}", e))
        })?;

        let duration = start_time.elapsed();
        log_health_event!(
            "healthy",
            "daemon",
            "health_checker",
            "complete",
            duration_ms = duration.as_millis() as u64,
            status = %health.status,
            url = %self.health_url
        );

        debug!("Health check successful: {:?}", health);
        Ok(health)
    }

    /// Check if the service is ready (more thorough than basic health check)
    pub async fn check_readiness(&self) -> crate::Result<()> {
        let health = self.check_health().await?;

        // Additional readiness checks could be added here
        if health.status != "healthy" {
            return Err(VoiceCliError::Daemon(format!(
                "Service not ready, status: {}",
                health.status
            )));
        }

        Ok(())
    }

    /// Wait for service to become ready with retries
    pub async fn wait_for_ready(&self, max_attempts: u32, interval: Duration) -> crate::Result<()> {
        for attempt in 1..=max_attempts {
            match self.check_readiness().await {
                Ok(_) => {
                    debug!("Service ready after {} attempts", attempt);
                    return Ok(());
                }
                Err(e) => {
                    if attempt == max_attempts {
                        return Err(e);
                    }
                    debug!(
                        "Health check attempt {}/{} failed: {}",
                        attempt, max_attempts, e
                    );
                    tokio::time::sleep(interval).await;
                }
            }
        }

        Err(VoiceCliError::Daemon(
            "Service did not become ready within the specified attempts".to_string(),
        ))
    }
}
