//! Background Service Implementations
//! 
//! This module contains all the concrete implementations of the BackgroundService trait
//! for different voice-cli services.

pub mod http_server_service;
pub mod cluster_node_service;
pub mod load_balancer_service;

// Re-export service implementations for convenience
pub use http_server_service::{HttpServerService, HttpServerServiceBuilder};
pub use cluster_node_service::{ClusterNodeService, ClusterNodeServiceBuilder};
pub use load_balancer_service::{LoadBalancerService, LoadBalancerServiceBuilder};

// Re-export common types
pub use crate::daemon::background_service::{
    BackgroundService, ServiceHealth, ServiceStatus, ServiceManager, 
    ClonableService, DefaultServiceManager, ServiceError
};