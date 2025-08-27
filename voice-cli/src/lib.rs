pub mod cli;
pub mod config;
pub mod config_rs_integration;
pub mod daemon;
pub mod error;
pub mod models;
pub mod openapi;
pub mod server;
pub mod services;
pub mod utils;

// Cluster functionality (temporarily simplified - raft disabled due to protobuf issues)
pub mod cluster;
pub mod grpc;
pub mod load_balancer;

// Re-export commonly used types
pub use error::{Result, VoiceCliError};
pub use models::*;

// Re-export services
pub use services::{AudioProcessor, ModelService, TranscriptionService};

// Tests module
#[cfg(test)]
mod tests;
