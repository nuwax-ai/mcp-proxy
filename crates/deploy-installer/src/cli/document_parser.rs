use crate::{
    InstallOptions, ServiceIdentity, ServiceSpec, bundled_templates_dir, copy_if_exists,
    default_document_parser_install_dir, deploy_asset_version, install, optional_mineru_models_url,
    optional_venv_download_url, write_user_file,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::common::{
    CONFIG_FILENAME, canonicalize_install_dir, dispatch_service_action, ensure_bundled_binary,
    handle_launchd_install_result, print_install_success, read_server_port, resolve_user_group,
    upgrade_bundled_binary,
};
use crate::cli::env_config::{
    OSS_BUCKET_PLACEHOLDERS, apply_bucket_overrides_from_env, apply_upload_config_from_env,
    custom_upload_configured, oss_keys_configured, parse_env_file_value, upload_backend_configured,
};
use crate::cli::tarball::{download_and_extract_tarball, extract_tarball_at};
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
        DocumentParserAction::Verify { install_dir } => {
            crate::cli::verify::verify_document_parser(install_dir)
        }
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
    venv_python_path(install_dir).exists()
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

    // MinerU pipeline 模型缓存（~/.cache/modelscope，用户级、跨安装目录）：
    // 缺模型时首跑 PDF 解析会从 ModelScope 下载——部分网络下死循环到
    // 3600s 超时（Win53 实测）。默认安装期从自家 OSS 供给（~1GB，幂等跳过）。
    ensure_mineru_models(args, quiet);

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
        println!("     export ALIYUN_OSS_PUBLIC_BUCKET=your_bucket");
        println!("     export ALIYUN_OSS_PRIVATE_BUCKET=your_bucket");
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

    // OSS 后端防呆：密钥已配而 bucket 仍是模板占位符时，异步上传会在运行期
    // 才炸 E010（2026-09-10 三机深测实测）——安装期 fail-fast 并给出解法。
    // custom 后端部署不走 OSS，跳过（131/53 既有模式回归）。
    if oss_keys_configured(&env_path) && !custom_upload_configured(&env_path) {
        // 先落盘 bucket 覆盖键（export 了才写）：防呆报错的修复指引就是
        // "export ALIYUN_OSS_*_BUCKET 后重跑 install"，不落盘的话重跑时
        // 凭证已在 .env、上传配置整体 skip，bucket 永远进不去（死循环）
        apply_bucket_overrides_from_env(&env_path)?;
        verify_oss_bucket_resolved(&env_path, &install_dir.join(CONFIG_FILENAME))?;
    }

    let service_result = run_service(ServiceAction::Install(ServiceDirArgs {
        install_dir: install_dir.clone(),
        user: None,
        no_start: false,
        dry_run: false,
        cuda_lib_dir: None,
        cudnn_lib_dir: None,
    }));
    let manual_cmd = format!(
        "{}/{} --config {}/config.yml server",
        install_dir.display(),
        crate::binary_name("document-parser"),
        install_dir.display()
    );
    let started = handle_launchd_install_result(service_result, SERVICE_NAME, &manual_cmd)?;
    let config_path = install_dir.join(CONFIG_FILENAME);
    let port = read_server_port(&config_path).unwrap_or(DEFAULT_PORT);
    if started {
        print_install_success(SERVICE_NAME, &install_dir, port)?;
    } else {
        // SSH-only 降级：有意未启动（桌面登录后自启），非失败——退出码 0
        println!("\n✅ {SERVICE_NAME} 已安装（服务未启动：等待桌面登录自启，或手动运行上方命令）");
        println!("   Dir: {}", install_dir.display());
    }
    Ok(())
}

fn run_service(action: ServiceAction) -> Result<()> {
    dispatch_service_action(SERVICE_NAME, action, service_install)
}

/// OSS 后端的 bucket 解析校验：生效值 = `.env` 的 `ALIYUN_OSS_*_BUCKET`
/// 覆盖（运行时同名 env 生效）> config.yml 的 `public_bucket`/`private_bucket`。
/// 仍是模板占位符（或缺配置）即报错——给出 env / 手改 config 两种解法。
fn verify_oss_bucket_resolved(env_path: &Path, config_path: &Path) -> Result<()> {
    let config_buckets = read_config_yaml_buckets(config_path);
    let mut offending: Vec<&'static str> = Vec::new();

    for (field, env_key, config_key) in [
        ("public_bucket", "ALIYUN_OSS_PUBLIC_BUCKET", "public_bucket"),
        (
            "private_bucket",
            "ALIYUN_OSS_PRIVATE_BUCKET",
            "private_bucket",
        ),
    ] {
        let effective = parse_env_file_value(env_path, env_key)
            .filter(|v| !v.trim().is_empty())
            .or_else(|| config_buckets.get(config_key).cloned());
        let resolved = effective.is_some_and(|v| {
            let t = v.trim().trim_matches('"').trim_matches('\'');
            !t.is_empty() && !OSS_BUCKET_PLACEHOLDERS.contains(&t)
        });
        if !resolved {
            offending.push(field);
        }
    }

    if offending.is_empty() {
        return Ok(());
    }
    let list = offending.join(" & ");
    println!("\n⚠️  OSS keys are configured but {list} is still the template placeholder.");
    println!("   Async uploads would fail at runtime (E010) with this configuration.");
    println!("   Fix either way, then re-run install:");
    println!("     export ALIYUN_OSS_PUBLIC_BUCKET=your_bucket   (and ALIYUN_OSS_PRIVATE_BUCKET)");
    println!(
        "     — or edit {} and set storage.oss.{} to the real bucket",
        config_path.display(),
        offending[0]
    );
    bail!(
        "OSS backend selected but {} unresolved in {} — set ALIYUN_OSS_PUBLIC_BUCKET \
         / ALIYUN_OSS_PRIVATE_BUCKET or edit storage.oss in the config",
        list,
        config_path.display()
    );
}

/// 从 config.yml 提取 `public_bucket:` / `private_bucket:` 行的值（模板形状的
/// 简单行解析，去引号；缺失或找不到文件返回空 map，由调用方按未解析处理）。
/// 值含行内注释时先剥离——出厂模板每行都带 `# ← 改成你的` 尾注，不剥的话
/// `"your-public-bucket"   # ← …` 解析出的值不在占位符列表里，防呆全失效
fn read_config_yaml_buckets(config_path: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Ok(content) = fs::read_to_string(config_path) else {
        return map;
    };
    for line in content.lines() {
        let t = line.trim();
        for key in ["public_bucket", "private_bucket"] {
            if let Some(rest) = t.strip_prefix(&format!("{key}:")) {
                let v = strip_yaml_inline_comment(rest.trim());
                let v = v.trim_matches('"').trim_matches('\'');
                map.entry(key.to_string()).or_insert(v.to_string());
            }
        }
    }
    map
}

/// 剥离 YAML 值的行内注释：引号值取到闭合引号为止（保留引号让调用方统一剥壳），
/// 裸值在首个 `#` 处截断（`#` 前至少一个空白才是注释，`a#b` 这种伪注释不截）
fn strip_yaml_inline_comment(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'"') || bytes.first() == Some(&b'\'') {
        let quote = bytes[0];
        if let Some(end) = bytes[1..].iter().position(|&b| b == quote).map(|i| i + 1) {
            return &value[..end + 1];
        }
        // 无闭合引号：畸形行，退回裸值规则
    }
    match value.find(" #") {
        Some(idx) => value[..idx].trim_end(),
        None => value,
    }
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

    let bin = install_dir.join(crate::binary_name(SERVICE_NAME));
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
    // macOS LaunchAgent 模板仅在 macOS 复制（Windows 任务定义由
    // render_task_xml 渲染生成，无需模板文件）
    #[cfg(target_os = "macos")]
    let mappings = [
        ("config.example.yml", CONFIG_FILENAME),
        (".document-parser.env.example", ENV_FILENAME),
        (
            "com.nuwax.document-parser.plist",
            "com.nuwax.document-parser.plist",
        ),
    ];
    #[cfg(not(target_os = "macos"))]
    let mappings = [
        ("config.example.yml", CONFIG_FILENAME),
        (".document-parser.env.example", ENV_FILENAME),
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

/// MinerU pipeline 模型缓存目录特征（PDF-Extract-Kit-1.0 的 snapshot 根）——
/// 幂等判定与下载后验共用
fn mineru_models_cache_present() -> bool {
    crate::home_dir()
        .map(|h| {
            h.join(".cache/modelscope/models/OpenDataLab--PDF-Extract-Kit-1.0/snapshots")
                .exists()
        })
        .unwrap_or(false)
}

/// 供给 MinerU pipeline 模型到 `~/.cache/modelscope`（tar 顶层 `modelscope/` →
/// 解压到 `~/.cache` 即落位）。
///
/// 语义与 venv 不同：URL 缺失/网络失败**WARN 降级不 bail**——模型可由首跑
/// PDF 解析从 ModelScope 自下（只是部分网络会慢/死循环），不应阻断安装。
fn ensure_mineru_models(args: &SetupArgs, quiet: bool) {
    if args.skip_models {
        if !quiet {
            println!(
                "  mineru models: skipped (--skip-models); first PDF parse will download from ModelScope"
            );
        }
        return;
    }
    if mineru_models_cache_present() {
        if !quiet {
            println!("  mineru models: cache present, skipping download");
        }
        return;
    }
    let url = match args.oss_base.as_deref() {
        // --oss-base 显式给下载源（与 venv/whisper 的 from_base 语义对齐）
        Some(base) => crate::mineru_models_download_url_from_base(base),
        None => {
            let Some(url) = optional_mineru_models_url() else {
                if !quiet {
                    println!(
                        "  mineru models: WARN (no mineruModels URL in manifest.json — first PDF parse will download from ModelScope)"
                    );
                }
                return;
            };
            url
        }
    };
    let Some(home) = crate::home_dir() else {
        if !quiet {
            println!("  mineru models: WARN (cannot resolve home directory)");
        }
        return;
    };
    let cache_dir = home.join(".cache");
    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        println!(
            "  mineru models: WARN (create {}: {e})",
            cache_dir.display()
        );
        return;
    }
    let quiet_dl = quiet;
    if let Err(e) = download_and_extract_tarball(
        &url,
        &cache_dir,
        "mineru-models.tar.gz",
        quiet_dl,
        "mineru models",
    ) {
        println!(
            "  mineru models: WARN (download failed: {e:#}; first PDF parse will download from ModelScope)"
        );
        return;
    }
    if !mineru_models_cache_present() {
        println!(
            "  mineru models: WARN (archive extracted but expected cache layout not found under ~/.cache/modelscope)"
        );
    } else if !quiet {
        println!("  mineru models: ready (~/.cache/modelscope)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_pair(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let env = dir.join(ENV_FILENAME);
        std::fs::write(&env, "OSS_ACCESS_KEY_ID=ak\nOSS_ACCESS_KEY_SECRET=sk\n").unwrap();
        let cfg = dir.join(CONFIG_FILENAME);
        std::fs::write(
            &cfg,
            "storage:\n  oss:\n    public_bucket: \"your-public-bucket\"\n    private_bucket: \"your-private-bucket\"\n",
        )
        .unwrap();
        (env, cfg)
    }

    #[test]
    fn oss_placeholder_bucket_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (env, cfg) = write_pair(dir.path());
        let err = verify_oss_bucket_resolved(&env, &cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("public_bucket"), "报错应点名字段: {msg}");
        assert!(
            msg.contains("ALIYUN_OSS_PUBLIC_BUCKET"),
            "报错应给出解法: {msg}"
        );
    }

    #[test]
    fn bucket_env_override_passes() {
        let dir = tempfile::tempdir().unwrap();
        let (env, cfg) = write_pair(dir.path());
        std::fs::write(
            &env,
            "OSS_ACCESS_KEY_ID=ak\nOSS_ACCESS_KEY_SECRET=sk\n\
             ALIYUN_OSS_PUBLIC_BUCKET=nuwa-packages\nALIYUN_OSS_PRIVATE_BUCKET=nuwa-packages\n",
        )
        .unwrap();
        verify_oss_bucket_resolved(&env, &cfg)
            .expect("bucket 经 env 覆盖后应通过（运行时同名 env 生效）");
    }

    #[test]
    fn real_bucket_in_config_passes() {
        let dir = tempfile::tempdir().unwrap();
        let (env, cfg) = write_pair(dir.path());
        std::fs::write(
            &cfg,
            "storage:\n  oss:\n    public_bucket: 'nuwa-packages'\n    private_bucket: nuwa-packages\n",
        )
        .unwrap();
        verify_oss_bucket_resolved(&env, &cfg)
            .expect("config.yml 填了真实 bucket 应通过（单引号与裸值都要能解析）");
    }

    #[test]
    fn config_bucket_line_parser() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.yml");
        std::fs::write(
            &cfg,
            "# 注释\npublic_bucket: \"a-bucket\"\n  private_bucket: b-bucket\nendpoint: e\n",
        )
        .unwrap();
        let m = read_config_yaml_buckets(&cfg);
        assert_eq!(m.get("public_bucket").map(String::as_str), Some("a-bucket"));
        assert_eq!(
            m.get("private_bucket").map(String::as_str),
            Some("b-bucket")
        );
    }

    /// 出厂模板形状：每行带 `# ← 改成你的` 尾注——不剥离注释的话解析值带着
    /// 注释尾巴、不在占位符列表里，防呆在最常见的全新安装场景一次都不触发
    #[test]
    fn config_bucket_line_parser_strips_inline_comments() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.yml");
        std::fs::write(
            &cfg,
            "storage:\n  oss:\n    endpoint: \"oss-rg-china-mainland.aliyuncs.com\"   # ← 改成你的 OSS endpoint\n    public_bucket: \"your-public-bucket\"               # ← 改成你的公共 bucket\n    private_bucket: \"your-private-bucket\"             # ← 改成你的私有 bucket\n",
        )
        .unwrap();
        let m = read_config_yaml_buckets(&cfg);
        assert_eq!(
            m.get("public_bucket").map(String::as_str),
            Some("your-public-bucket"),
            "引号值 + 行尾注释应剥出干净占位符（防呆输入）: {:?}",
            m.get("public_bucket")
        );
        assert_eq!(
            m.get("private_bucket").map(String::as_str),
            Some("your-private-bucket")
        );

        // 真实值 + 行尾注释：剥注释后是干净 bucket（不误伤）
        std::fs::write(
            &cfg,
            "    public_bucket: nuwa-packages   # 我的生产桶\n    private_bucket: nuwa-packages #note\n",
        )
        .unwrap();
        let m = read_config_yaml_buckets(&cfg);
        assert_eq!(
            m.get("public_bucket").map(String::as_str),
            Some("nuwa-packages")
        );
        assert_eq!(
            m.get("private_bucket").map(String::as_str),
            Some("nuwa-packages")
        );

        // 单引号值 + 尾注
        std::fs::write(&cfg, "public_bucket: 'your-public-bucket'  # x\n").unwrap();
        let m = read_config_yaml_buckets(&cfg);
        assert_eq!(
            m.get("public_bucket").map(String::as_str),
            Some("your-public-bucket")
        );
    }

    /// 全新安装防呆回归：出厂模板原样（带尾注）+ OSS 密钥 → 必须报 bucket 未解析
    #[test]
    fn factory_template_shape_triggers_bucket_guard() {
        let dir = tempfile::tempdir().unwrap();
        let (env, cfg) = write_pair(dir.path());
        // 模拟 copy_templates 落盘的出厂 config.example.yml 原样行
        std::fs::write(
            &cfg,
            "storage:\n  oss:\n    public_bucket: \"your-public-bucket\"               # ← 改成你的公共 bucket\n    private_bucket: \"your-private-bucket\"             # ← 改成你的私有 bucket\n",
        )
        .unwrap();
        let err = verify_oss_bucket_resolved(&env, &cfg).unwrap_err();
        assert!(
            format!("{err:#}").contains("public_bucket"),
            "出厂模板形状必须触发防呆: {err:#}"
        );
    }
}
