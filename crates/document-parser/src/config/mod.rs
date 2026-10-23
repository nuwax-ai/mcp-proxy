//! 配置模块：分层配置体系（默认值 → YAML 文件 → 环境变量 → 命令行）。
//!
//! 按领域拆分为多个子模块，本模块通过 re-export 保持对外统一的 API 面：
//! 所有 `crate::config::X` / `document_parser::config::X` 路径与拆分前完全一致。

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use std::env;
use std::fs::File;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};
use thiserror::Error;
use tracing::{info, warn};

/// 配置验证错误
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("配置文件读取失败: {0}")]
    FileRead(String),
    #[error("配置解析失败: {0}")]
    Parse(String),
    #[error("配置验证失败: {field} - {message}")]
    Validation { field: String, message: String },
    #[error("环境变量解析失败: {var} - {message}")]
    EnvVar { var: String, message: String },
    #[error("路径无效: {path} - {message}")]
    InvalidPath { path: String, message: String },
}

pub mod app_config;
pub mod builder;
pub mod engines;
pub mod file_size;
pub mod global;
pub mod server;
pub mod storage;

pub use app_config::*;
pub use builder::*;
pub use engines::*;
pub use file_size::*;
pub use global::*;
pub use server::*;
pub use storage::*;
