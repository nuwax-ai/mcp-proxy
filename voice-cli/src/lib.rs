pub mod cli;
pub mod config;
pub mod config_rs_integration;
pub mod error;
pub mod models;
pub mod openapi;
pub mod server;
pub mod services;
pub mod utils;

// Re-export commonly used types
pub use error::{Result, VoiceCliError};
pub use models::*;

// Re-export services
pub use services::{AudioProcessor, ModelService, transcription_engine};

// Tests module
#[cfg(test)]
mod tests;
