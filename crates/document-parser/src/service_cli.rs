//! systemd service install / uninstall / status / restart for document-parser.

use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use systemd_installer::{
    InstallOptions, ServiceIdentity, ServiceSpec, group_for_user, install, resolve_service_user,
    restart, status, uninstall, write_user_file,
};

const SERVICE_NAME: &str = "document-parser";
const ENV_FILENAME: &str = ".document-parser.env";
const ENV_EXAMPLE: &str = include_str!("../deploy/systemd/.document-parser.env.example");

/// Ensure `config.yml` exists, creating from `AppConfig::default()` if missing (`create=true`).
pub fn ensure_config_yml(install_dir: &Path, create: bool) -> Result<(PathBuf, AppConfig)> {
    let config_path = install_dir.join("config.yml");
    if !config_path.exists() {
        if create {
            AppConfig::write_default_config(&config_path).map_err(|e| {
                anyhow::anyhow!("failed to write config.yml from code defaults: {e}")
            })?;
            println!(
                "✅ Created {} from code defaults — edit for your environment",
                config_path.display()
            );
        } else {
            return Ok((config_path.clone(), AppConfig::default()));
        }
    }
    let cfg =
        AppConfig::load_base_config_with_path(Some(config_path.to_string_lossy().into_owned()))
            .map_err(|e| anyhow::anyhow!("failed to load {}: {e}", config_path.display()))?;
    Ok((config_path, cfg))
}

/// Ensure `.document-parser.env` exists from embedded example template (`create=true`).
pub fn ensure_env_file(install_dir: &Path, create: bool) -> Result<PathBuf> {
    let env_path = install_dir.join(ENV_FILENAME);
    if !env_path.exists() && create {
        write_user_file(&env_path, ENV_EXAMPLE, Some(0o600))
            .map_err(|e| anyhow::anyhow!("failed to create {ENV_FILENAME}: {e}"))?;
        println!(
            "✅ Created {} from template — fill OSS_ACCESS_KEY_ID / OSS_ACCESS_KEY_SECRET",
            env_path.display()
        );
        println!("⚠️  OSS secrets may still be empty; service install will warn but not block");
    }
    Ok(env_path)
}

fn resolve_binary(install_dir: &Path, dry_run: bool) -> Result<PathBuf> {
    let candidate = install_dir.join("document-parser");
    if candidate.exists() {
        return Ok(candidate);
    }
    if dry_run {
        return Ok(std::env::current_exe().unwrap_or(candidate));
    }
    bail!(
        "binary not found at {} — copy the release binary into install_dir before install",
        candidate.display()
    )
}

pub struct InstallParams {
    pub install_dir: PathBuf,
    pub user: Option<String>,
    pub no_start: bool,
    pub dry_run: bool,
}

pub fn handle_service_install(params: InstallParams) -> Result<()> {
    let install_dir = if params.install_dir.as_os_str().is_empty() {
        std::env::current_dir().context("current_dir")?
    } else {
        params
            .install_dir
            .canonicalize()
            .unwrap_or(params.install_dir)
    };

    let (config_path, cfg) = ensure_config_yml(&install_dir, !params.dry_run)?;
    let env_path = ensure_env_file(&install_dir, !params.dry_run)?;
    let bin = resolve_binary(&install_dir, params.dry_run)?;
    let user = resolve_service_user(params.user).context("resolve service user")?;
    let group = group_for_user(&user).context("id -gn")?;

    let mut required_paths = Vec::new();
    if !params.dry_run || config_path.exists() {
        required_paths.push(config_path.clone());
    }
    if !params.dry_run {
        required_paths.push(bin.clone());
        // EnvironmentFile= is mandatory for this unit; file must exist before install.
        required_paths.push(env_path.clone());
    }

    let spec = ServiceSpec {
        name: SERVICE_NAME.into(),
        description: "Document Parser Service (MCP document-parser)".into(),
        identity: ServiceIdentity { user, group },
        install_dir: install_dir.clone(),
        // Top-level --config then subcommand (clap global-style args on Cli).
        exec_start: vec![
            bin.display().to_string(),
            "--config".into(),
            config_path.display().to_string(),
            "server".into(),
        ],
        env_file: Some(env_path),
        extra_env: vec![],
        kill_signal: Some("SIGINT".into()),
        timeout_stop_sec: Some(60),
        syslog_identifier: Some("document-parser".into()),
        drop_ins: vec![],
        supplementary_groups: vec![],
        required_paths,
        listen_port: Some(cfg.server.port),
    };

    let opts = InstallOptions {
        enable: true,
        start: !params.no_start,
        dry_run: params.dry_run,
    };

    install(&spec, &opts).context("systemd install failed")?;
    Ok(())
}

pub fn handle_service_uninstall() -> Result<()> {
    uninstall(SERVICE_NAME).context("systemd uninstall failed")?;
    Ok(())
}

pub fn handle_service_status() -> Result<()> {
    status(SERVICE_NAME).context("systemd status failed")?;
    Ok(())
}

pub fn handle_service_restart() -> Result<()> {
    restart(SERVICE_NAME).context("systemd restart failed")?;
    Ok(())
}
