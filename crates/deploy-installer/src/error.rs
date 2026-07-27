use std::path::PathBuf;
use thiserror::Error;

/// Errors produced while rendering or installing systemd units.
#[derive(Debug, Error)]
pub enum InstallerError {
    #[error("invalid path `{path}`: {reason}")]
    InvalidPath { path: String, reason: String },

    #[error("invalid ExecStart argument `{arg}`: {reason}")]
    InvalidArg { arg: String, reason: String },

    #[error("invalid unit/drop-in name `{name}`: {reason}")]
    InvalidName { name: String, reason: String },

    #[error("invalid unit field `{field}`: {reason}")]
    InvalidField { field: String, reason: String },

    #[error("ExecStart is empty")]
    EmptyExecStart,

    #[error("precheck failed:\n{details}")]
    PrecheckFailed { details: String },

    #[error("binary not found or not executable: {path}")]
    BinaryNotExecutable { path: PathBuf },

    #[error("required path missing: {path}")]
    RequiredPathMissing { path: PathBuf },

    #[error("install directory not writable: {path}")]
    InstallDirNotWritable { path: PathBuf },

    #[error("port {port} is already in use{pid_hint}. Change config server.port and retry.")]
    PortInUse { port: u16, pid_hint: String },

    #[error("sudo is required but unavailable: {reason}")]
    SudoUnavailable { reason: String },

    #[error("command failed: {cmd}: {detail}")]
    CommandFailed { cmd: String, detail: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, InstallerError>;
