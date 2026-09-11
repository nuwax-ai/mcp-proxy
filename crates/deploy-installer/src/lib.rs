//! Shared library to render and install service units (systemd on Linux, launchd on macOS,
//! Task Scheduler on Windows) via the [`service-manager`](https://github.com/chipsenkbeil/service-manager-rs)
//! crate (launchd/systemd) and the schtasks wrapper module (Windows).
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
mod render_task;
mod service_mgr;
mod service_task;
mod spec;
mod systemd;
mod task_scheduler;

pub mod cli;

pub(crate) use bundles::home_dir;
pub use bundles::{
    WhisperModelsPack, binary_name, bundled_binary_path, bundled_templates_dir, copy_if_exists,
    default_document_parser_install_dir, default_voice_cli_install_dir, deploy_asset_version,
    deploy_root, deploy_version, make_executable, mineru_models_download_url_from_base,
    optional_mineru_models_url, optional_venv_download_url, optional_voice_cli_cuda_url,
    optional_voice_cli_vulkan_url, optional_whisper_download_url, platform_vendor_key,
    vendor_key_for, voice_cli_cuda_archive_filename, voice_cli_cuda_download_url_from_base,
    voice_cli_vulkan_archive_filename, voice_cli_vulkan_download_url_from_base,
    whisper_download_url_from_base,
};
#[cfg(target_os = "macos")]
pub use checks::macos_gui_session_present;
pub use checks::{
    CheckItem, CheckSeverity, LinuxSyslibStatus, PrecheckOptions, PrecheckReport,
    REQUIRED_LINUX_SYSLIBS, current_user, group_for_user, libs_missing_from_ldconfig,
    linux_syslibs_install_hint, precheck, resolve_service_user,
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
