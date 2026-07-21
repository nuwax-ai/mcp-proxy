//! systemd service install / uninstall / status / restart.

use crate::models::Config;
use anyhow::{Context, Result, bail};
use deploy_installer::{
    DropIn, InstallOptions, ServiceIdentity, ServiceSpec, group_for_user, install,
    resolve_service_user, restart, status, uninstall,
};
use std::path::{Path, PathBuf};

const SERVICE_NAME: &str = "voice-cli";

/// Ensure `config.yml` exists under `install_dir`, creating from `Config::default()` if missing.
pub fn ensure_config_yml(install_dir: &Path, create: bool) -> Result<(PathBuf, Config)> {
    let config_path = install_dir.join("config.yml");
    if !config_path.exists() {
        if create {
            Config::default()
                .save(&config_path)
                .context("failed to write config.yml from code defaults")?;
            println!(
                "✅ Created {} from code defaults — edit for your environment",
                config_path.display()
            );
        } else {
            return Ok((config_path.clone(), Config::default()));
        }
    }
    let cfg = Config::load(&config_path)
        .with_context(|| format!("failed to load {}", config_path.display()))?;
    Ok((config_path, cfg))
}

fn resolve_binary(install_dir: &Path, dry_run: bool) -> Result<PathBuf> {
    let candidate = install_dir.join("voice-cli");
    if candidate.exists() {
        return Ok(candidate);
    }
    if dry_run {
        // Mac / cargo run: allow preview with current executable.
        return Ok(std::env::current_exe().unwrap_or(candidate));
    }
    bail!(
        "binary not found at {} — copy the release binary into install_dir before install",
        candidate.display()
    )
}

fn build_cuda_drop_in(
    install_dir: &Path,
    cuda_lib_dir: Option<&Path>,
    cudnn_lib_dir: Option<&Path>,
) -> Option<DropIn> {
    if cuda_lib_dir.is_none() && cudnn_lib_dir.is_none() {
        return None;
    }
    let mut parts: Vec<String> = vec![install_dir.display().to_string()];
    if let Some(p) = cudnn_lib_dir {
        parts.push(p.display().to_string());
    }
    if let Some(p) = cuda_lib_dir {
        parts.push(p.display().to_string());
    }
    // Only add default CUDA path when cuda_lib_dir was explicitly provided as empty?
    // No: if user only passed cudnn, do not invent /usr/local/cuda/lib64.
    let ld = parts.join(":");
    Some(DropIn {
        name: "cuda-sherpa".into(),
        content: format!("[Service]\nEnvironment=LD_LIBRARY_PATH={ld}\n"),
    })
}

/// Parameters for `service install`.
pub struct InstallParams {
    pub install_dir: PathBuf,
    pub user: Option<String>,
    pub no_start: bool,
    pub dry_run: bool,
    pub cuda_lib_dir: Option<PathBuf>,
    pub cudnn_lib_dir: Option<PathBuf>,
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
    let bin = resolve_binary(&install_dir, params.dry_run)?;
    let user = resolve_service_user(params.user).context("resolve service user")?;
    let group = group_for_user(&user).context("id -gn")?;

    let mut drop_ins = Vec::new();
    if let Some(d) = build_cuda_drop_in(
        &install_dir,
        params.cuda_lib_dir.as_deref(),
        params.cudnn_lib_dir.as_deref(),
    ) {
        drop_ins.push(d);
    }

    let mut required_paths = Vec::new();
    if !params.dry_run || config_path.exists() {
        required_paths.push(config_path.clone());
    }
    if !params.dry_run {
        required_paths.push(bin.clone());
    }

    let spec = ServiceSpec {
        name: SERVICE_NAME.into(),
        description: "voice-cli speech-to-text service".into(),
        identity: ServiceIdentity { user, group },
        install_dir: install_dir.clone(),
        exec_start: vec![
            bin.display().to_string(),
            "server".into(),
            "run".into(),
            "--config".into(),
            config_path.display().to_string(),
        ],
        env_file: None,
        extra_env: vec![("RUST_LOG".into(), "info".into())],
        kill_signal: None,
        timeout_stop_sec: None,
        syslog_identifier: None,
        drop_ins,
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
