//! systemd service 子命令分发（Linux）。

use super::ServiceAction;
use anyhow::Result;

pub async fn handle_service_command(action: ServiceAction) -> Result<()> {
    use document_parser::service_cli::{self, InstallParams};

    match action {
        ServiceAction::Install {
            install_dir,
            user,
            no_start,
            dry_run,
        } => service_cli::handle_service_install(InstallParams {
            install_dir,
            user,
            no_start,
            dry_run,
        }),
        ServiceAction::Uninstall => service_cli::handle_service_uninstall(),
        ServiceAction::Status => service_cli::handle_service_status(),
        ServiceAction::Restart => service_cli::handle_service_restart(),
    }
}
