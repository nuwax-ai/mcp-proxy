use crate::{
    InstallOptions, ServiceIdentity, ServiceSpec, bundled_binary_path, bundled_templates_dir,
    copy_if_exists, default_document_parser_install_dir, deploy_asset_version, group_for_user,
    install, make_executable, optional_venv_download_url, resolve_service_user, restart_in_dir,
    status_in_dir, uninstall_in_dir, write_user_file,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::{DocumentParserAction, ServiceAction, ServiceDirArgs, SetupArgs};

const SERVICE_NAME: &str = "document-parser";
const ENV_FILENAME: &str = ".document-parser.env";
const CONFIG_FILENAME: &str = "config.yml";

pub fn run(action: DocumentParserAction) -> Result<()> {
    match action {
        DocumentParserAction::Setup(args) => {
            setup(&args, false)?;
            Ok(())
        }
        DocumentParserAction::Install(args) => install_full(&args),
        DocumentParserAction::Upgrade { install_dir } => upgrade(&install_dir),
        DocumentParserAction::Service { action } => run_service(action),
    }
}

fn resolve_install_dir(args: &SetupArgs) -> PathBuf {
    args.install_dir
        .clone()
        .unwrap_or_else(default_document_parser_install_dir)
}

fn setup(args: &SetupArgs, quiet: bool) -> Result<PathBuf> {
    let install_dir = resolve_install_dir(args);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("create install dir {}", install_dir.display()))?;

    if !quiet {
        println!("==> document-parser setup → {}", install_dir.display());
    }

    // 1. Copy bundled binary
    let bundled = bundled_binary_path(SERVICE_NAME);
    let dst_bin = install_dir.join(SERVICE_NAME);
    if bundled.exists() {
        fs::copy(&bundled, &dst_bin)
            .with_context(|| format!("copy {} → {}", bundled.display(), dst_bin.display()))?;
        make_executable(&dst_bin)?;
        if !quiet {
            println!("  copied binary from {}", bundled.display());
        }
    } else if !dst_bin.exists() {
        bail!(
            "bundled binary not found at {} and no existing binary in {}",
            bundled.display(),
            dst_bin.display()
        );
    }

    // 2. Copy templates
    copy_templates(&install_dir, quiet)?;

    // 3. Python environment
    if args.use_prebuilt_venv {
        download_prebuilt_venv(args, &install_dir, quiet)?;
    } else {
        run_uv_init(&dst_bin, &install_dir, quiet)?;
    }

    // 4. Patch config for macOS MPS
    patch_config_for_macos(&install_dir.join(CONFIG_FILENAME))?;

    if !quiet {
        println!("\n✅ setup complete: {}", install_dir.display());
        println!("   Next: edit {}/{}", install_dir.display(), ENV_FILENAME);
        println!(
            "   Then: deploy-installer document-parser service install --install-dir {}",
            install_dir.display()
        );
    }
    Ok(install_dir)
}

fn install_full(args: &SetupArgs) -> Result<()> {
    let install_dir = setup(args, true)?;
    let env_path = install_dir.join(ENV_FILENAME);

    if !oss_keys_configured(&env_path) {
        println!("\n⚠️  OSS keys not configured in {}", env_path.display());
        println!("   Fill OSS_ACCESS_KEY_ID and OSS_ACCESS_KEY_SECRET, then run:");
        println!(
            "   deploy-installer document-parser service install --install-dir {}",
            install_dir.display()
        );
        return Ok(());
    }

    run_service(ServiceAction::Install(ServiceDirArgs {
        install_dir,
        user: None,
        no_start: false,
        dry_run: false,
    }))
}

fn upgrade(install_dir: &Path) -> Result<()> {
    let bundled = bundled_binary_path(SERVICE_NAME);
    let dst = install_dir.join(SERVICE_NAME);
    if !bundled.exists() {
        bail!("bundled binary not found at {}", bundled.display());
    }
    fs::copy(&bundled, &dst)?;
    make_executable(&dst)?;
    println!("✅ upgraded {} → {}", bundled.display(), dst.display());
    println!(
        "   Run: deploy-installer document-parser service restart --install-dir {}",
        install_dir.display()
    );
    Ok(())
}

fn run_service(action: ServiceAction) -> Result<()> {
    match action {
        ServiceAction::Install(args) => service_install(&args),
        ServiceAction::Uninstall(args) => {
            uninstall_in_dir(SERVICE_NAME, Some(args.install_dir)).context("uninstall failed")
        }
        ServiceAction::Status(args) => {
            status_in_dir(SERVICE_NAME, Some(args.install_dir)).context("status failed")
        }
        ServiceAction::Restart(args) => {
            restart_in_dir(SERVICE_NAME, Some(args.install_dir)).context("restart failed")
        }
    }
}

fn service_install(args: &ServiceDirArgs) -> Result<()> {
    let install_dir = args
        .install_dir
        .canonicalize()
        .unwrap_or(args.install_dir.clone());

    let config_path = install_dir.join(CONFIG_FILENAME);
    if !config_path.exists() {
        bail!("missing {} — run setup first", config_path.display());
    }

    let env_path = install_dir.join(ENV_FILENAME);
    if !env_path.exists() && !args.dry_run {
        bail!("missing {} — run setup first", env_path.display());
    }

    let bin = install_dir.join(SERVICE_NAME);
    if !bin.exists() && !args.dry_run {
        bail!("missing binary {} — run setup first", bin.display());
    }

    let user = resolve_service_user(args.user.clone()).context("resolve service user")?;
    let group = group_for_user(&user).context("resolve service group")?;

    let port = read_server_port(&config_path).unwrap_or(8087);

    let spec = ServiceSpec {
        name: SERVICE_NAME.into(),
        description: "Document Parser Service (MCP document-parser)".into(),
        identity: ServiceIdentity { user, group },
        install_dir: install_dir.clone(),
        exec_start: vec![
            bin.display().to_string(),
            "--config".into(),
            config_path.display().to_string(),
            "server".into(),
        ],
        env_file: Some(env_path.clone()),
        extra_env: vec![],
        kill_signal: Some("SIGINT".into()),
        timeout_stop_sec: Some(60),
        syslog_identifier: Some(SERVICE_NAME.into()),
        drop_ins: vec![],
        supplementary_groups: vec![],
        required_paths: if args.dry_run {
            vec![config_path]
        } else {
            vec![config_path, bin, env_path]
        },
        listen_port: Some(port),
    };

    let opts = InstallOptions {
        enable: true,
        start: !args.no_start,
        dry_run: args.dry_run,
    };

    install(&spec, &opts).context("service install failed")?;
    Ok(())
}

fn copy_templates(install_dir: &Path, quiet: bool) -> Result<()> {
    let templates = bundled_templates_dir(SERVICE_NAME);
    let mappings = [
        ("config.example.yml", CONFIG_FILENAME),
        (".document-parser.env.example", ENV_FILENAME),
        ("run-server.sh", "run-server.sh"),
        (
            "com.nuwax.document-parser.plist",
            "com.nuwax.document-parser.plist",
        ),
    ];

    for (src_name, dst_name) in mappings {
        let src = templates.join(src_name);
        let dst = install_dir.join(dst_name);
        if dst.exists() {
            continue;
        }
        if copy_if_exists(&src, &dst)? {
            if dst_name == ENV_FILENAME {
                write_user_file(&dst, &fs::read_to_string(&dst)?, Some(0o600))?;
            }
            if dst_name == "run-server.sh" {
                make_executable(&dst)?;
            }
            if !quiet {
                println!("  template: {}", dst_name);
            }
        } else if dst_name == CONFIG_FILENAME && !dst.exists() {
            // Fallback: minimal config for macOS
            let content = include_str!("../../../document-parser/deploy/config/config.example.yml");
            fs::write(&dst, content)?;
            patch_config_for_macos(&dst)?;
            if !quiet {
                println!("  template: {} (embedded fallback)", dst_name);
            }
        } else if dst_name == ENV_FILENAME && !dst.exists() {
            let content = include_str!(
                "../../../document-parser/deploy/systemd/.document-parser.env.example"
            );
            write_user_file(&dst, content, Some(0o600))?;
            if !quiet {
                println!("  template: {} (embedded fallback)", dst_name);
            }
        }
    }
    Ok(())
}

fn run_uv_init(bin: &Path, install_dir: &Path, quiet: bool) -> Result<()> {
    if install_dir.join("venv").join("bin").join("python").exists() {
        if !quiet {
            println!("  venv: already exists, skipping uv-init");
        }
        return Ok(());
    }
    if !quiet {
        println!("  venv: running document-parser uv-init (may take several minutes)...");
    }
    let status = Command::new(bin)
        .arg("uv-init")
        .current_dir(install_dir)
        .status()
        .with_context(|| format!("run {} uv-init", bin.display()))?;
    if !status.success() {
        bail!("document-parser uv-init failed with status {status}");
    }
    Ok(())
}

fn download_prebuilt_venv(args: &SetupArgs, install_dir: &Path, quiet: bool) -> Result<()> {
    let url = if let Some(base) = args.oss_base.as_deref() {
        let version = deploy_asset_version();
        format!("{base}/venv-macos-arm64-{version}.tar.gz")
    } else if let Some(url) = optional_venv_download_url() {
        url
    } else {
        bail!(
            "--use-prebuilt-venv requires --oss-base or a venv URL in vendor/templates/manifest.json"
        );
    };
    let archive = install_dir.join("venv-prebuilt.tar.gz");
    if !quiet {
        println!("  venv: downloading {url}");
    }
    let status = Command::new("curl")
        .args(["-fL", &url, "-o", &archive.display().to_string()])
        .status()
        .context("curl download venv")?;
    if !status.success() {
        bail!("failed to download prebuilt venv from {url}");
    }
    let status = Command::new("tar")
        .args([
            "-xzf",
            &archive.display().to_string(),
            "-C",
            &install_dir.display().to_string(),
        ])
        .status()
        .context("extract venv")?;
    if !status.success() {
        bail!("failed to extract venv archive");
    }
    let _ = fs::remove_file(&archive);
    Ok(())
}

fn patch_config_for_macos(config_path: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") || !config_path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(config_path)?;
    if content.contains("device: \"mps\"") || content.contains("device: 'mps'") {
        return Ok(());
    }
    let patched = content.replace("device: \"cpu\"", "device: \"mps\"");
    fs::write(config_path, patched)?;
    Ok(())
}

fn read_server_port(config_path: &Path) -> Option<u16> {
    let content = fs::read_to_string(config_path).ok()?;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("port:") {
            return t.split(':').nth(1)?.trim().parse().ok();
        }
    }
    None
}

fn oss_keys_configured(env_path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(env_path) else {
        return false;
    };
    let mut id_ok = false;
    let mut secret_ok = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if k.trim() == "OSS_ACCESS_KEY_ID" && !v.is_empty() {
                id_ok = true;
            }
            if k.trim() == "OSS_ACCESS_KEY_SECRET" && !v.is_empty() {
                secret_ok = true;
            }
        }
    }
    id_ok && secret_ok
}
