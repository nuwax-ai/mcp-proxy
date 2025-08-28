//! Background Service Implementations
//! 
//! This module contains all the concrete implementations of the BackgroundService trait
//! for different voice-cli services.

pub mod http_server_service;

// Re-export service implementations for convenience
pub use http_server_service::{HttpServerService, HttpServerServiceBuilder};

// Re-export common types
pub use crate::daemon::background_service::{
    BackgroundService, ServiceHealth, ServiceStatus, ServiceManager, 
    ClonableService, DefaultServiceManager, ServiceError
};