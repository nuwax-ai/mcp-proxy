//! `deploy-installer voice-cli` — macOS setup + OSS Whisper models + Linux CUDA OSS bundle.

use crate::{
    DropIn, InstallOptions, ServiceIdentity, ServiceSpec, WhisperModelsPack, bundled_binary_path,
    bundled_templates_dir, copy_if_exists, default_voice_cli_install_dir, deploy_asset_version,
    install, optional_voice_cli_cuda_url, optional_whisper_download_url,
    voice_cli_cuda_archive_filename, voice_cli_cuda_download_url_from_base,
    whisper_download_url_from_base,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::common::{
    CONFIG_FILENAME, WHISPER_DEFAULT_MODEL, build_cuda_sherpa_drop_in, canonicalize_install_dir,
    default_cuda_lib_dir, detect_cudnn_lib_dir, dispatch_service_action,
    download_and_extract_tarball, ensure_bundled_binary, ensure_whisper_pack_models,
    handle_launchd_install_result, patch_whisper_default_model, print_install_success,
    read_server_port, resolve_user_group, upgrade_bundled_binary, voice_cli_cuda_bundle_present,
    whisper_large_v3_present, whisper_pack_satisfied,
};
use crate::cli::{ServiceAction, ServiceDirArgs, VoiceCliAction, VoiceCliSetupArgs};

const SERVICE_NAME: &str = "voice-cli";
const DEFAULT_PORT: u16 = 8077;

pub fn run(action: VoiceCliAction) -> Result<()> {
    match action {
        VoiceCliAction::Setup(args) => {
            setup(&args, false, false)?;
            Ok(())
        }
        VoiceCliAction::Install(args) => install_full(&args),
        VoiceCliAction::Upgrade { install_dir } => {
            let dir = install_dir.unwrap_or_else(default_voice_cli_install_dir);
            upgrade_voice_cli(&dir, None)
        }
        VoiceCliAction::Service { action } => run_service(action),
    }
}

fn resolve_install_dir(args: &VoiceCliSetupArgs) -> PathBuf {
    args.install_dir
        .clone()
        .unwrap_or_else(default_voice_cli_install_dir)
}

fn effective_use_prebuilt_models(args: &VoiceCliSetupArgs) -> bool {
    if args.skip_models {
        return false;
    }
    if args.use_prebuilt_models {
        return true;
    }
    cfg!(target_os = "macos")
}

fn effective_use_oss_cuda(args: &VoiceCliSetupArgs) -> bool {
    if args.skip_oss_cuda {
        return false;
    }
    if args.use_oss_cuda {
        return true;
    }
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

fn resolve_models_pack(args: &VoiceCliSetupArgs) -> Result<WhisperModelsPack> {
    WhisperModelsPack::parse(&args.models).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid --models {} (expected large-v3 or all)",
            args.models
        )
    })
}

fn resolve_cuda_lib_dir(args: &VoiceCliSetupArgs) -> Option<PathBuf> {
    args.cuda_lib_dir.clone().or_else(default_cuda_lib_dir)
}

fn resolve_cudnn_lib_dir(args: &VoiceCliSetupArgs) -> Option<PathBuf> {
    args.cudnn_lib_dir.clone().or_else(detect_cudnn_lib_dir)
}

fn resolve_cuda_lib_dir_from_service(args: &ServiceDirArgs) -> Option<PathBuf> {
    args.cuda_lib_dir.clone().or_else(default_cuda_lib_dir)
}

fn resolve_cudnn_lib_dir_from_service(args: &ServiceDirArgs) -> Option<PathBuf> {
    args.cudnn_lib_dir.clone().or_else(detect_cudnn_lib_dir)
}

fn ensure_voice_cli_binary(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    if effective_use_oss_cuda(args) {
        download_oss_cuda_bundle(args, install_dir, quiet)?;
        return Ok(());
    }
    if cfg!(windows)
        && !bundled_binary_path(SERVICE_NAME).exists()
        && !install_dir.join(crate::binary_name(SERVICE_NAME)).exists()
    {
        bail!(
            "voice-cli is not bundled in this Windows release — only document-parser is \
             supported here; run `deploy-installer document-parser install`"
        );
    }
    ensure_bundled_binary(SERVICE_NAME, install_dir, quiet)?;
    Ok(())
}

fn download_oss_cuda_bundle(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    if voice_cli_cuda_bundle_present(install_dir) {
        if !quiet {
            println!("  binary: CUDA bundle already present, skipping download");
        }
        return Ok(());
    }

    let version = deploy_asset_version();
    let archive = voice_cli_cuda_archive_filename(&version);
    let url = if let Some(base) = args.oss_base.as_deref() {
        voice_cli_cuda_download_url_from_base(base)
    } else if let Some(url) = optional_voice_cli_cuda_url() {
        url
    } else {
        bail!(
            "prebuilt voice-cli CUDA bundle requires --oss-base or voiceCliCuda URL in \
             vendor/templates/manifest.json (upload {archive} to OSS first)"
        );
    };

    download_and_extract_tarball(
        &url,
        install_dir,
        "voice-cli-cuda-prebuilt.tar.gz",
        quiet,
        "voice-cli CUDA",
    )?;

    if !voice_cli_cuda_bundle_present(install_dir) {
        bail!(
            "CUDA bundle incomplete under {} — expected voice-cli and sherpa/onnx .so files",
            install_dir.display()
        );
    }
    Ok(())
}

fn upgrade_voice_cli(install_dir: &Path, oss_base: Option<&str>) -> Result<()> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        let args = VoiceCliSetupArgs {
            install_dir: Some(install_dir.to_path_buf()),
            use_prebuilt_models: false,
            skip_models: true,
            models: "large-v3".into(),
            oss_base: oss_base.map(String::from),
            use_oss_cuda: true,
            skip_oss_cuda: false,
            cuda_lib_dir: None,
            cudnn_lib_dir: None,
        };
        download_oss_cuda_bundle(&args, install_dir, false)?;
        println!(
            "✅ upgraded voice-cli CUDA bundle in {}",
            install_dir.display()
        );
        println!("   Run: deploy-installer voice-cli service restart");
        return Ok(());
    }
    upgrade_bundled_binary(SERVICE_NAME, install_dir)
}

fn setup(args: &VoiceCliSetupArgs, quiet: bool, installing: bool) -> Result<PathBuf> {
    let install_dir = resolve_install_dir(args);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("create install dir {}", install_dir.display()))?;

    if !quiet {
        println!("==> voice-cli setup → {}", install_dir.display());
    }

    ensure_voice_cli_binary(args, &install_dir, quiet)?;
    copy_templates(&install_dir, quiet)?;
    fs::create_dir_all(install_dir.join("models"))
        .with_context(|| format!("create models dir under {}", install_dir.display()))?;
    fs::create_dir_all(install_dir.join("logs"))
        .with_context(|| format!("create logs dir under {}", install_dir.display()))?;

    let config_path = install_dir.join(CONFIG_FILENAME);
    patch_whisper_default_model(&config_path, WHISPER_DEFAULT_MODEL)?;

    if effective_use_prebuilt_models(args) {
        download_prebuilt_whisper(args, &install_dir, quiet)?;
    } else if !quiet && effective_use_oss_cuda(args) {
        println!("  models: place ggml-*.bin under models/ (Whisper OSS is macOS-only in phase 1)");
    } else if !quiet {
        println!("  models: skipped (use --use-prebuilt-models or omit --skip-models on macOS)");
    }

    if !quiet {
        println!("\n✅ setup complete: {}", install_dir.display());
        if whisper_large_v3_present(&install_dir) {
            println!("   Whisper model: {WHISPER_DEFAULT_MODEL} (ggml-large-v3.bin)");
        } else if effective_use_prebuilt_models(args) {
            println!("   Whisper default model: {WHISPER_DEFAULT_MODEL}");
        }
        if effective_use_oss_cuda(args) && voice_cli_cuda_bundle_present(&install_dir) {
            println!("   Binary: voice-cli CUDA bundle (OSS)");
        }
        if !installing {
            println!("   Next: deploy-installer voice-cli install");
            println!("   Default listen port: {DEFAULT_PORT} (document-parser uses 8087)");
        }
    }
    Ok(install_dir)
}

fn install_full(args: &VoiceCliSetupArgs) -> Result<()> {
    let install_dir = setup(args, false, true)?;
    if !effective_use_prebuilt_models(args) && !effective_use_oss_cuda(args) {
        println!(
            "\n⚠️  Whisper models not downloaded. Run without --skip-models or place \
             models/ggml-large-v3.bin manually."
        );
    }
    let cuda_lib = resolve_cuda_lib_dir(args);
    let cudnn_lib = resolve_cudnn_lib_dir(args);
    let service_result = run_service(ServiceAction::Install(ServiceDirArgs {
        install_dir: install_dir.clone(),
        user: None,
        no_start: false,
        dry_run: false,
        cuda_lib_dir: cuda_lib,
        cudnn_lib_dir: cudnn_lib,
    }));
    let manual_cmd = format!(
        "{}/{} server run --config {}/config.yml",
        install_dir.display(),
        crate::binary_name("voice-cli"),
        install_dir.display()
    );
    handle_launchd_install_result(service_result, SERVICE_NAME, &manual_cmd)?;
    let config_path = install_dir.join(CONFIG_FILENAME);
    let port = read_server_port(&config_path).unwrap_or(DEFAULT_PORT);
    print_install_success(SERVICE_NAME, &install_dir, port);
    Ok(())
}

fn download_prebuilt_whisper(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    let pack = resolve_models_pack(args)?;
    if whisper_pack_satisfied(install_dir, pack) {
        if !quiet {
            let label = match pack {
                WhisperModelsPack::LargeV3 => "ggml-large-v3.bin",
                WhisperModelsPack::All => "all models (tiny…large-v3)",
            };
            println!("  models: {label} already present, skipping download");
        }
        return Ok(());
    }

    let version = deploy_asset_version();
    let archive = pack.archive_filename(&version);
    let url = if let Some(base) = args.oss_base.as_deref() {
        whisper_download_url_from_base(base, pack)
    } else if let Some(url) = optional_whisper_download_url(pack) {
        url
    } else {
        bail!(
            "prebuilt Whisper models require --oss-base or whisper URLs in \
             vendor/templates/manifest.json (upload {archive} to OSS first)"
        );
    };
    download_and_extract_tarball(
        &url,
        install_dir,
        "whisper-prebuilt.tar.gz",
        quiet,
        "models",
    )?;
    ensure_whisper_pack_models(install_dir, pack)
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

    let bin = install_dir.join(crate::binary_name(SERVICE_NAME));
    if !bin.exists() && !args.dry_run {
        bail!("missing binary {} — run setup first", bin.display());
    }

    let (user, group) = resolve_user_group(args.user.clone())?;
    let port = read_server_port(&config_path).unwrap_or(DEFAULT_PORT);

    let mut drop_ins: Vec<DropIn> = Vec::new();
    if cfg!(target_os = "linux") && voice_cli_cuda_bundle_present(&install_dir) {
        let cuda = resolve_cuda_lib_dir_from_service(args);
        let cudnn = resolve_cudnn_lib_dir_from_service(args);
        if let Some(d) = build_cuda_sherpa_drop_in(&install_dir, cuda.as_deref(), cudnn.as_deref())
        {
            drop_ins.push(d);
        } else if !args.dry_run {
            println!(
                "  WARN: CUDA bundle present but no --cuda-lib-dir / CUDNN_LIB_DIR; \
                 systemd unit may fail to load libcudart unless LD_LIBRARY_PATH is set globally"
            );
        }
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
        kill_signal: Some("SIGINT".into()),
        timeout_stop_sec: Some(60),
        syslog_identifier: Some(SERVICE_NAME.into()),
        drop_ins,
        supplementary_groups: vec![],
        required_paths: if args.dry_run {
            vec![config_path]
        } else {
            vec![config_path, bin]
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
    let mappings = if cfg!(target_os = "macos") {
        vec![
            ("config.example.yml", CONFIG_FILENAME),
            ("com.nuwax.voice-cli.plist", "com.nuwax.voice-cli.plist"),
        ]
    } else {
        vec![("config.example.yml", CONFIG_FILENAME)]
    };

    for (src_name, dst_name) in mappings {
        let src = templates.join(src_name);
        let dst = install_dir.join(dst_name);
        if dst.exists() {
            continue;
        }
        if copy_if_exists(&src, &dst)? {
            if !quiet {
                println!("  template: {}", dst_name);
            }
        } else if dst_name == CONFIG_FILENAME && !dst.exists() {
            let content = include_str!("../../../voice-cli/deploy/config.example.yml");
            fs::write(&dst, content)?;
            if !quiet {
                println!("  template: {} (embedded fallback)", dst_name);
            }
        }
    }
    Ok(())
}
