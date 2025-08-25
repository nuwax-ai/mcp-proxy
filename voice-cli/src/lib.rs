pub mod cli;
pub mod config;
pub mod daemon;
pub mod error;
pub mod models;
pub mod openapi;
pub mod server;
pub mod services;
pub mod utils;

// Cluster functionality (temporarily simplified - raft disabled due to protobuf issues)
pub mod cluster;
pub mod load_balancer;
pub mod grpc;

// Re-export commonly used types
pub use error::{VoiceCliError, Result};
pub use models::*;
pub use config::ConfigManager;

// Re-export services
pub use services::{AudioProcessor, TranscriptionService, ModelService};