// New unified background service abstraction (recommended)
pub mod background_service;
pub mod service_logging;
pub mod services;

// Re-export new unified background service components (recommended)
pub use background_service::{
    BackgroundService, ServiceHealth, ServiceStatus, ServiceManager,
    ClonableService, DefaultServiceManager, ServiceError
};

// Cross-platform daemon functionality (removed - only foreground mode supported)
pub use service_logging::{
    init_service_logging, validate_logging_config, get_logs_directory,
    setup_log_rotation, apply_logging_env_overrides, ServiceLogContext
};
pub use services::{
    HttpServerService, HttpServerServiceBuilder,
    ClusterNodeService, ClusterNodeServiceBuilder,
    LoadBalancerService, LoadBalancerServiceBuilder
};

