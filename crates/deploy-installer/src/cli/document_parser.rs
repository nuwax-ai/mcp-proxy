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
    CONFIG_FILENAME, apply_upload_config_from_env, canonicalize_install_dir,
    custom_upload_configured, dispatch_service_action, download_and_extract_tarball,
    ensure_bundled_binary, extract_tarball_at, handle_launchd_install_result, oss_keys_configured,
    print_install_success, read_server_port, resolve_user_group, upgrade_bundled_binary,
    upload_backend_configured,
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

    // Linux headless 预检：MinerU/opencv 运行需要 X11/GL 基础库。放在所有 venv
    // 路径（uv-init / --venv-file / 预编译 venv）之前——venv 内的 Python 同样依赖
    // 这些系统库，缺失时应在下载/解压数 GB 依赖前就 fail-fast 并给出修复命令。
    ensure_linux_syslibs()?;

    // --venv-file：离线安装本地 venv 包（最高优先级；适合无法访问 OSS 下载源的内网环境）
    if let Some(venv_file) = args.venv_file.as_deref() {
        if !venv_file.is_file() {
            bail!("--venv-file not found: {}", venv_file.display());
        }
        if venv_present(&install_dir) {
            if !quiet {
                println!("  venv: already exists, skipping --venv-file extraction");
            }
        } else {
            extract_tarball_at(venv_file, &install_dir, quiet, "venv (local)")?;
            if !venv_present(&install_dir) {
                bail!(
                    "--venv-file archive has no top-level venv/ directory — \
                     pack it as: tar -czf venv.tgz -C <staging_dir> venv"
                );
            }
        }
    } else if effective_use_prebuilt_venv(args) {
        ensure_prebuilt_venv(args, &install_dir, &dst_bin, quiet)?;
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
                println!(
                    "   upload backend: OSS keys configured in {}",
                    env_path.display()
                );
            } else if custom_upload_configured(&env_path) {
                println!(
                    "   upload backend: custom upload configured in {}",
                    env_path.display()
                );
            } else {
                println!(
                    "   Next: configure an upload backend (OSS keys OR custom upload), then run:"
                );
                println!("     deploy-installer document-parser install");
            }
        }
    }
    Ok(install_dir)
}

fn install_full(args: &SetupArgs) -> Result<()> {
    let install_dir = setup(args, false, true)?;
    let env_path = install_dir.join(ENV_FILENAME);

    // 上传后端凭证落盘：仅在**尚未配置任何后端**时才从环境写入——
    // 已配置的 .env 不被 shell 残留的旧 export 覆盖（用户手改凭证后重跑
    // install 应保持文件值）
    if !upload_backend_configured(&env_path) {
        apply_upload_config_from_env(&env_path)?;
    }

    if !upload_backend_configured(&env_path) {
        println!("\n⚠️  An upload backend is required (OSS keys OR custom upload, pick one).");
        println!("   Setup finished (venv/binary ready). Configure one, then re-run install:");
        println!("   Option A — OSS (cloud deployment):");
        println!("     export OSS_ACCESS_KEY_ID=your_key");
        println!("     export OSS_ACCESS_KEY_SECRET=your_secret");
        println!("   Option B — custom upload backend (private deployment, nuwax-style REST):");
        println!(
            "     export DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://your-system.example.com"
        );
        println!("     export DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=your_api_key");
        println!(
            "   Option C — edit {} directly (no export prefix), then re-run install.",
            env_path.display()
        );
        bail!(
            "no upload backend configured in {} — export OSS_ACCESS_KEY_* \
             or DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL, or edit the file above",
            env_path.display()
        );
    }

    let service_result = run_service(ServiceAction::Install(ServiceDirArgs {
        install_dir: install_dir.clone(),
        user: None,
        no_start: false,
        dry_run: false,
        cuda_lib_dir: None,
        cudnn_lib_dir: None,
    }));
    handle_launchd_install_result(service_result, SERVICE_NAME, &install_dir)?;
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

/// venv 内 python 解释器路径（unix: `bin/python`；windows: `Scripts/python.exe`）。
fn venv_python_path(install_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        install_dir.join("venv").join("Scripts").join("python.exe")
    } else {
        install_dir.join("venv").join("bin").join("python")
    }
}

/// 验证 venv 的 python 可用（能执行且输出 `Python 3.x`）。
///
/// 预编译 venv 的 `bin/python` 是指向**打包机**绝对路径的 symlink，在非打包机上
/// 必然断链；服务运行时"后台自动安装"也可能留下空骨架 venv。两种坏 venv 都会
/// 通过 `venv_present` 检查却无法解析任何文档——必须以 python 实际可执行为准。
fn venv_python_usable(install_dir: &Path) -> bool {
    Command::new(venv_python_path(install_dir))
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).starts_with("Python 3")
        })
}

/// 预编译 venv 主流程：已存在则验证（坏则删除重建），下载后验证（坏则修复
/// symlink，修不了则回退 uv-init）。任何出口都保证 venv 可用或报错。
fn ensure_prebuilt_venv(
    args: &SetupArgs,
    install_dir: &Path,
    dst_bin: &Path,
    quiet: bool,
) -> Result<()> {
    if venv_present(install_dir) {
        if venv_python_usable(install_dir) {
            if !quiet {
                println!("  venv: already exists, skipping download");
            }
            return Ok(());
        }
        println!(
            "  venv: existing venv has unusable python (broken symlink or empty skeleton), \
             removing and re-downloading"
        );
        fs::remove_dir_all(install_dir.join("venv"))
            .with_context(|| format!("remove broken venv under {}", install_dir.display()))?;
    }

    download_prebuilt_venv(args, install_dir, quiet)?;

    if venv_python_usable(install_dir) {
        return Ok(());
    }

    // venv/bin/python 断链（指向打包机的解释器路径）：尝试用本机同版本解释器重建
    if repair_prebuilt_venv_python(install_dir) && venv_python_usable(install_dir) {
        println!("  venv: python symlink repaired to a local interpreter");
        return Ok(());
    }

    // 修复无望（本机没有同版本解释器）：删除坏 venv 回退 uv-init，用本机 Python 重建
    println!(
        "  venv: prebuilt venv is not usable on this machine (no matching local Python), \
         falling back to uv-init"
    );
    fs::remove_dir_all(install_dir.join("venv")).with_context(|| {
        format!(
            "remove unusable prebuilt venv under {}",
            install_dir.display()
        )
    })?;
    run_uv_init(dst_bin, install_dir, quiet)
}

/// 用本机与打包版本一致的解释器重建 venv 的 python symlink。
///
/// 打包脚本（pack-document-parser-venv-macos-arm64.sh）以 Python 3.12 构建，
/// venv 内 wheel 均为 cp312 ABI——只有本机存在 3.12 解释器时才能修复，其它
/// 版本会 ABI 不兼容。目标版本从 `pyvenv.cfg` 的 `version_info` 读取（缺省 3.12），
/// 候选路径覆盖 Homebrew / 系统常见位置与 PATH（`which python3.<minor>`）。
#[cfg(target_os = "macos")]
fn repair_prebuilt_venv_python(install_dir: &Path) -> bool {
    use std::os::unix::fs::symlink;

    let venv_bin = install_dir.join("venv").join("bin");

    // 打包时的解释器版本（pyvenv.cfg: "version_info = 3.12.14"）
    let mut minor = 12u32;
    if let Ok(cfg) = fs::read_to_string(install_dir.join("venv").join("pyvenv.cfg")) {
        for line in cfg.lines() {
            let Some(rest) = line.trim().strip_prefix("version_info") else {
                continue;
            };
            let mut parts = rest.trim_start_matches(['=', ' ']).split('.');
            if parts.next() == Some("3")
                && let Some(Ok(n)) = parts.next().map(str::parse::<u32>)
            {
                minor = n;
            }
        }
    }
    let exe_name = format!("python3.{minor}");

    let home = std::env::var("HOME").unwrap_or_default();
    let mut candidates: Vec<PathBuf> = [
        format!("/opt/homebrew/bin/{exe_name}"),
        format!("/usr/local/bin/{exe_name}"),
        format!("{home}/.local/bin/{exe_name}"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    // PATH 上的同版本解释器（which）
    if let Ok(out) = Command::new("which").arg(&exe_name).output()
        && out.status.success()
    {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            candidates.insert(0, PathBuf::from(p));
        }
    }

    let Some(found) = candidates.iter().find(|p| p.is_file()) else {
        return false;
    };

    // 重建 bin 下的 python 系列链接（python / python3 / python3.<minor>）
    for name in ["python", "python3", exe_name.as_str()] {
        let link = venv_bin.join(name);
        let _ = fs::remove_file(&link);
        if symlink(found, &link).is_err() {
            return false;
        }
    }
    true
}

#[cfg(not(target_os = "macos"))]
fn repair_prebuilt_venv_python(_install_dir: &Path) -> bool {
    false
}

/// Linux：X11/GL 基础库预检（Fail Fast，缺库时在 venv 下载前给出修复命令）。
#[cfg(target_os = "linux")]
fn ensure_linux_syslibs() -> Result<()> {
    let status = crate::checks::check_required_linux_syslibs();
    if status.skipped {
        println!("  syslibs: WARN (ldconfig unavailable — skipped X11/GL lib check)");
        return Ok(());
    }
    if status.missing.is_empty() {
        return Ok(());
    }
    bail!(
        "missing Linux system libraries: {}\n{}",
        status.missing.join(", "),
        crate::checks::linux_syslibs_install_hint()
    );
}

/// 非 Linux 平台无此依赖，直接放行。
#[cfg(not(target_os = "linux"))]
fn ensure_linux_syslibs() -> Result<()> {
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
