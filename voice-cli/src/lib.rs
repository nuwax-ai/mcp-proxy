// 初始化 i18n，使用 workspace 根目录的翻译文件
#[macro_use]
extern crate rust_i18n;

// 初始化翻译文件，指向 workspace 根目录的 locales
i18n!("../locales", fallback = "en");

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
