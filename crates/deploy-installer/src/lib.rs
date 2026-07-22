//! Shared library to render and install service units (systemd on Linux, launchd on macOS)
//! via the [`service-manager`](https://github.com/chipsenkbeil/service-manager-rs) crate.
//!
//! Callers (voice-cli / document-parser / deploy-installer CLI) build a [`ServiceSpec`] and call
//! [`install`] / [`uninstall`] / [`status`] / [`restart`].

mod bundles;
mod checks;
mod error;
mod exec_argv;
mod installer;
mod platform;
mod render;
mod render_plist;
mod service_mgr;
mod spec;
mod systemd;

pub mod cli;

pub use bundles::{
    WhisperModelsPack, bundled_binary_path, bundled_templates_dir, copy_if_exists,
    default_document_parser_install_dir, default_voice_cli_install_dir, deploy_asset_version,
    deploy_root, deploy_version, make_executable, optional_venv_download_url,
    optional_whisper_download_url, platform_vendor_key, whisper_download_url_from_base,
};
pub use checks::{
    CheckItem, CheckSeverity, PrecheckOptions, PrecheckReport, current_user, group_for_user,
    precheck, resolve_service_user,
};
pub use error::{InstallerError, Result};
pub use exec_argv::{default_exec_argv, program_and_args};
pub use installer::{
    InstallOptions, install, path_exists, restart, restart_in_dir, status, status_in_dir,
    uninstall, uninstall_in_dir, write_user_file,
};
pub use platform::{ServiceBackend, current_backend};
pub use render::{render_unit, sanitize_path, sanitize_unit_value, shell_join, validate_unit_name};
pub use render_plist::render_launchd_plist;
pub use spec::{DropIn, ServiceIdentity, ServiceSpec};
