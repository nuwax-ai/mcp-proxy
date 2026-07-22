use crate::{
    InstallOptions, ServiceIdentity, ServiceSpec, bundled_templates_dir, copy_if_exists,
    default_document_parser_install_dir, deploy_asset_version, install, optional_venv_download_url,
    write_user_file,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::common::{
    CONFIG_FILENAME, apply_oss_keys_from_env, canonicalize_install_dir, dispatch_service_action,
    download_and_extract_tarball, ensure_bundled_binary, oss_keys_configured,
    print_install_success, read_server_port, resolve_user_group, upgrade_bundled_binary,
};
use crate::cli::{DocumentParserAction, ServiceAction, ServiceDirArgs, SetupArgs};

const SERVICE_NAME: &str = "document-parser";
const ENV_FILENAME: &str = ".document-parser.env";
const DEFAULT_PORT: u16 = 8087;

pub fn run(action: DocumentParserAction) -> Result<()> {
    match action {
        DocumentParserAction::Setup(args) => {
            setup(&args, false, false)?;
            Ok(())
        }
        DocumentParserAction::Install(args) => install_full(&args),
        DocumentParserAction::Upgrade { install_dir } => {
            let dir = install_dir.unwrap_or_else(default_document_parser_install_dir);
            upgrade_bundled_binary(SERVICE_NAME, &dir)
        }
        DocumentParserAction::Service { action } => run_service(action),
    }
}

fn resolve_install_dir(args: &SetupArgs) -> PathBuf {
    args.install_dir
        .clone()
        .unwrap_or_else(default_document_parser_install_dir)
}

fn effective_use_prebuilt_venv(args: &SetupArgs) -> bool {
    if args.no_prebuilt_venv {
        return false;
    }
    if args.use_prebuilt_venv {
        return true;
    }
    cfg!(target_os = "macos")
}

fn venv_present(install_dir: &Path) -> bool {
    install_dir.join("venv").join("bin").join("python").exists()
}

fn setup(args: &SetupArgs, quiet: bool, installing: bool) -> Result<PathBuf> {
    let install_dir = resolve_install_dir(args);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("create install dir {}", install_dir.display()))?;

    if !quiet {
        println!("==> document-parser setup → {}", install_dir.display());
    }

    let dst_bin = ensure_bundled_binary(SERVICE_NAME, &install_dir, quiet)?;
    copy_templates(&install_dir, quiet)?;

    if effective_use_prebuilt_venv(args) {
        if venv_present(&install_dir) {
            if !quiet {
                println!("  venv: already exists, skipping download");
            }
        } else {
            download_prebuilt_venv(args, &install_dir, quiet)?;
        }
    } else if !venv_present(&install_dir) {
        run_uv_init(&dst_bin, &install_dir, quiet)?;
    } else if !quiet {
        println!("  venv: already exists, skipping uv-init");
    }

    patch_config_for_macos(&install_dir.join(CONFIG_FILENAME))?;

    if !quiet {
        println!("\n✅ setup complete: {}", install_dir.display());
        if !installing {
            let env_path = install_dir.join(ENV_FILENAME);
            if oss_keys_configured(&env_path) {
                println!("   OSS keys: configured in {}", env_path.display());
            } else {
                println!(
                    "   Next: set OSS keys, then run: deploy-installer document-parser install"
                );
            }
        }
    }
    Ok(install_dir)
}

fn install_full(args: &SetupArgs) -> Result<()> {
    let install_dir = setup(args, false, true)?;
    let env_path = install_dir.join(ENV_FILENAME);

    if !oss_keys_configured(&env_path) {
        let _ = apply_oss_keys_from_env(&env_path)?;
    }

    if !oss_keys_configured(&env_path) {
        println!("\n⚠️  OSS keys required for document-parser upload features.");
        println!("   Setup finished (venv/binary ready). Configure keys, then re-run install:");
        println!("   Option A — environment variables:");
        println!("     export OSS_ACCESS_KEY_ID=your_key");
        println!("     export OSS_ACCESS_KEY_SECRET=your_secret");
        println!("     deploy-installer document-parser install");
        println!(
            "   Option B — edit {} (no export prefix):",
            env_path.display()
        );
        println!("     OSS_ACCESS_KEY_ID=...");
        println!("     OSS_ACCESS_KEY_SECRET=...");
        println!("     deploy-installer document-parser install");
        bail!(
            "OSS keys not configured in {} — export OSS_ACCESS_KEY_ID / OSS_ACCESS_KEY_SECRET \
             or edit the file above",
            env_path.display()
        );
    }

    run_service(ServiceAction::Install(ServiceDirArgs {
        install_dir: install_dir.clone(),
        user: None,
        no_start: false,
        dry_run: false,
    }))?;
    let config_path = install_dir.join(CONFIG_FILENAME);
    let port = read_server_port(&config_path).unwrap_or(DEFAULT_PORT);
    print_install_success(SERVICE_NAME, &install_dir, port);
    Ok(())
}

fn run_service(action: ServiceAction) -> Result<()> {
    dispatch_service_action(SERVICE_NAME, action, service_install)
}

fn service_install(args: &ServiceDirArgs) -> Result<()> {
    let install_dir = canonicalize_install_dir(&args.install_dir);

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

    let (user, group) = resolve_user_group(args.user.clone())?;
    let port = read_server_port(&config_path).unwrap_or(DEFAULT_PORT);

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
            if !quiet {
                println!("  template: {}", dst_name);
            }
        } else if dst_name == CONFIG_FILENAME && !dst.exists() {
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
    let version = deploy_asset_version();
    let url = if let Some(base) = args.oss_base.as_deref() {
        format!(
            "{}/venv-macos-arm64-{version}.tar.gz",
            base.trim_end_matches('/')
        )
    } else if let Some(url) = optional_venv_download_url() {
        url
    } else {
        bail!("prebuilt venv requires --oss-base or a venv URL in vendor/templates/manifest.json");
    };
    download_and_extract_tarball(&url, install_dir, "venv-prebuilt.tar.gz", quiet, "venv")
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
