//! Cross-platform daemon/service implementation
//! 
//! This module provides safe, cross-platform daemon functionality without unsafe code.
//! It uses the existing background service infrastructure for consistent behavior across platforms.

use crate::daemon::background_service::{BackgroundService, DefaultServiceManager, ServiceError, ClonableService};

/// Cross-platform daemon service manager
pub struct CrossPlatformDaemon<S: BackgroundService + Clone + ClonableService> {
    service: S,
    config: S::Config,
    foreground_mode: bool,
}

impl<S: BackgroundService + Clone + ClonableService> CrossPlatformDaemon<S> {
    /// Create a new cross-platform daemon
    pub fn new(service: S, config: S::Config, foreground_mode: bool) -> Self {
        Self {
            service,
            config,
            foreground_mode,
        }
    }

    /// Start the daemon/service
    pub async fn start(&mut self) -> Result<(), ServiceError> {
        // Use the service manager's built-in background mode handling
        let mut manager = DefaultServiceManager::new(self.service.clone(), self.config.clone(), self.foreground_mode);
        manager.start().await?;
        
        // In foreground mode, wait for completion
        // In background mode, we need to keep the main process alive
        // but not block (so the service can run in the background)
        if self.foreground_mode {
            manager.wait().await;
        } else {
            // In background mode, keep the process running indefinitely
            // by waiting for a signal or sleeping forever
            tokio::signal::ctrl_c().await.expect("Failed to wait for Ctrl+C");
        }
        
        Ok(())
    }

    // Platform-specific methods removed - using service manager's built-in background mode

    /// Stop the daemon/service
    pub async fn stop(&mut self) -> Result<(), ServiceError> {
        // Use the service manager for stopping
        let mut manager = DefaultServiceManager::new(self.service.clone(), self.config.clone(), false);
        manager.stop().await
    }
}

// Platform-specific utilities removed - using service manager's built-in functionality