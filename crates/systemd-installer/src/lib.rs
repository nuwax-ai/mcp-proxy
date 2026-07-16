//! Shared library to render and install systemd unit files.
//!
//! Callers (voice-cli / document-parser) build a [`ServiceSpec`] and call
//! [`install`] / [`uninstall`] / [`status`] / [`restart`].

mod checks;
mod error;
mod installer;
mod render;
mod spec;
mod systemd;

pub use checks::{
    CheckItem, CheckSeverity, PrecheckOptions, PrecheckReport, current_user, group_for_user,
    precheck, resolve_service_user,
};
pub use error::{InstallerError, Result};
pub use installer::{
    InstallOptions, install, path_exists, restart, status, uninstall, write_user_file,
};
pub use render::{render_unit, sanitize_path, sanitize_unit_value, shell_join, validate_unit_name};
pub use spec::{DropIn, ServiceIdentity, ServiceSpec};
