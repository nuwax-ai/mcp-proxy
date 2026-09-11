//! `deploy-installer voice-cli` — macOS setup + OSS Whisper models + Linux 三档
//! （CUDA OSS bundle / Vulkan OSS bundle / vendor CPU，见 `VoiceCliLinuxTier`）。

use crate::{
    DropIn, InstallOptions, ServiceIdentity, ServiceSpec, WhisperModelsPack, bundled_binary_path,
    bundled_templates_dir, copy_if_exists, default_voice_cli_install_dir, deploy_asset_version,
    install, optional_voice_cli_cuda_url, optional_voice_cli_vulkan_url,
    optional_whisper_download_url, voice_cli_cuda_archive_filename,
    voice_cli_cuda_download_url_from_base, voice_cli_vulkan_archive_filename,
    voice_cli_vulkan_download_url_from_base, whisper_download_url_from_base,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::assets::{
    VOICE_CLI_CUDA_BUNDLE_FILES, VOICE_CLI_CUDA_BUNDLE_MARKER, VOICE_CLI_VULKAN_BUNDLE_FILES,
    VOICE_CLI_VULKAN_BUNDLE_MARKER, WHISPER_DEFAULT_MODEL, ensure_whisper_pack_models,
    patch_whisper_default_model, voice_cli_bundle_marker_version, voice_cli_cuda_bundle_present,
    voice_cli_vulkan_bundle_present, whisper_large_v3_present, whisper_pack_satisfied,
};
use crate::cli::common::{
    CONFIG_FILENAME, canonicalize_install_dir, dispatch_service_action, ensure_bundled_binary,
    handle_launchd_install_result, print_install_success, read_server_port, resolve_user_group,
    upgrade_bundled_binary,
};
use crate::cli::linux_gpu::{
    VoiceCliLinuxTier, build_cuda_sherpa_drop_in, cuda_preflight_report, default_cuda_lib_dir,
    detect_cudnn_lib_dir, vulkan_preflight_report,
};
use crate::cli::tarball::{download_and_extract_bundle_atomic, download_and_extract_tarball};
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
        VoiceCliAction::Verify { install_dir } => crate::cli::verify::verify_voice_cli(install_dir),
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
    // ggml 模型平台无关（whisper tar 三平台同 URL）——默认全平台下载，
    // 装完即用（2026-09-10 前仅 macOS，Windows/Linux 装完不能转写）
    true
}

/// 档位探测结果：tier + 原始探测值（回退提示需要区分"缺哪一项"）。
struct LinuxTierProbe {
    tier: VoiceCliLinuxTier,
    /// (nvidia-smi, libcublas)
    cuda: (bool, bool),
    /// (vulkan loader, 硬件 GPU)
    vulkan: (bool, bool),
    /// 探测是否真的跑过（false = 梯子前三级已定：显式旗标/双 skip/已装档位，
    /// 此时 cuda/vulkan 探测值是 false 占位，**不可用于打印"预检未通过"**）
    probed: bool,
}

impl LinuxTierProbe {
    /// 非 Linux x86_64 平台的占位档位（探测值全 false）。
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    fn cpu_default() -> Self {
        Self {
            tier: VoiceCliLinuxTier::Cpu,
            cuda: (false, false),
            vulkan: (false, false),
            probed: false,
        }
    }
}

/// setup/install 全程唯一的档位决策点（非 Linux x86_64 恒 CPU）。
/// 探测只在此跑一次，ensure/汇总/安装提示共用结果。
///
/// 懒探测：梯子前三级（显式旗标/双 skip/已装档位）已定时完全跳过探测——
/// 不白跑 nvidia-smi/探针子进程，坏驱动死等时也不会卡住强制 CPU 的用户。
fn linux_tier_probe(args: &VoiceCliSetupArgs, install_dir: &Path) -> LinuxTierProbe {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        use crate::cli::linux_gpu::{LinuxTierInputs, early_linux_tier};
        let inputs_without_probe = LinuxTierInputs {
            use_cuda: args.use_oss_cuda,
            skip_cuda: args.skip_oss_cuda,
            use_vulkan: args.use_oss_vulkan,
            skip_vulkan: args.skip_oss_vulkan,
            cuda_ok: false,
            vulkan_ok: false,
            cuda_installed: voice_cli_cuda_bundle_present(install_dir),
            vulkan_installed: voice_cli_vulkan_bundle_present(install_dir),
        };
        if let Some(tier) = early_linux_tier(&inputs_without_probe) {
            return LinuxTierProbe {
                tier,
                cuda: (false, false),
                vulkan: (false, false),
                probed: false,
            };
        }
        let cuda = crate::cli::linux_gpu::linux_cuda_runtime_available();
        let vulkan = crate::cli::linux_gpu::linux_vulkan_runtime_available();
        let tier = crate::cli::linux_gpu::resolve_linux_tier(&LinuxTierInputs {
            cuda_ok: cuda.0 && cuda.1,
            vulkan_ok: vulkan.1,
            ..inputs_without_probe
        });
        LinuxTierProbe {
            tier,
            cuda,
            vulkan,
            probed: true,
        }
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (args, install_dir);
        LinuxTierProbe::cpu_default()
    }
}

/// 档位回退 CPU 时的预检报告打印（提示不阻塞——用户定的原则）。
fn print_gpu_preflight_warnings(probe: &LinuxTierProbe) {
    let cuda_report = cuda_preflight_report(probe.cuda.0, probe.cuda.1);
    if !cuda_report.is_empty() {
        println!(
            "  ⚠️ CUDA 预检未通过（nvidia-smi={}, libcublas={}）",
            probe.cuda.0, probe.cuda.1
        );
        for line in cuda_report.lines() {
            println!("{line}");
        }
    }
    let vk_report = vulkan_preflight_report(probe.vulkan.0, probe.vulkan.1);
    if !vk_report.is_empty() {
        println!(
            "  ⚠️ Vulkan 预检未通过（loader={}, gpu={}）",
            probe.vulkan.0, probe.vulkan.1
        );
        for line in vk_report.lines() {
            println!("{line}");
        }
    }
}

/// 档位互斥清理：CPU/CUDA 档安装后删除 vulkan bundle marker——marker 与二进制
/// 档位不符会让下次 install/upgrade 误判档位（vulkan 二进制与 CPU 版按文件
/// 不可区分，marker 是唯一判据）。
fn remove_vulkan_marker(install_dir: &Path) {
    let _ = fs::remove_file(install_dir.join(VOICE_CLI_VULKAN_BUNDLE_MARKER));
}

/// 档位互斥清理（→CPU 方向）：删 CUDA bundle 专属 .so——providers_cuda 残留会让
/// `voice_cli_cuda_bundle_present()` 仍判 true，双 skip 强制降级后下次 auto
/// install/upgrade 会静默跳回 CUDA 档，撤销用户的强制选择。
fn remove_cuda_exclusive_files(install_dir: &Path) {
    for stale in [
        "libonnxruntime_providers_cuda.so",
        "libonnxruntime_providers_shared.so",
    ] {
        let _ = fs::remove_file(install_dir.join(stale));
    }
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
    probe: &LinuxTierProbe,
    quiet: bool,
) -> Result<()> {
    match probe.tier {
        VoiceCliLinuxTier::Cuda => {
            // 预检已折入档位解析（tier==Cuda 即显式强制/已装/预检通过三者之一），
            // 无需重复预检；无 GPU 机器走 Cpu 分支提示并回退
            download_oss_cuda_bundle(args, install_dir, quiet)?;
            Ok(())
        }
        VoiceCliLinuxTier::Vulkan => {
            if args.use_oss_vulkan {
                // 显式强制：与 --use-oss-cuda 同语义，资产缺失按错误上报
                download_oss_vulkan_bundle(args, install_dir, quiet)?;
                return Ok(());
            }
            // auto 档优雅降级：vulkan 档是安装器替用户做的决定（manifest 键缺失
            // 或下载失败），不硬失败——WARN 后回退 CPU；显式强制则保持 bail
            match download_oss_vulkan_bundle(args, install_dir, quiet) {
                Ok(()) => Ok(()),
                Err(e) => {
                    if !quiet {
                        println!("  ⚠️ Vulkan bundle 获取失败（{e}）");
                        println!("  → 自动回退安装 CPU 版本（vendor 内置二进制）");
                    }
                    ensure_bundled_binary(SERVICE_NAME, install_dir, quiet)?;
                    remove_vulkan_marker(install_dir);
                    Ok(())
                }
            }
        }
        VoiceCliLinuxTier::Cpu => {
            if cfg!(windows)
                && !bundled_binary_path(SERVICE_NAME).exists()
                && !install_dir.join(crate::binary_name(SERVICE_NAME)).exists()
            {
                bail!(
                    "voice-cli is not bundled in this Windows release — only document-parser is \
                     supported here; run `deploy-installer document-parser install`"
                );
            }
            if !quiet && cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                if probe.probed {
                    // auto 档落选：双预检报告 + 提示不阻塞（131 实测教训——白下
                    // 360MB 后崩溃循环；现在直接装 CPU 版并说明原因与升级路径）
                    print_gpu_preflight_warnings(probe);
                    println!("  → 安装 CPU 版本（vendor 内置二进制，不影响功能）");
                    println!(
                        "    如需 GPU 加速: 按上方提示修复环境后重装，或加 --use-oss-cuda/--use-oss-vulkan 强制"
                    );
                } else {
                    // 双 skip 强制 CPU：用户主动选择，未做探测——不能谎报"预检
                    // 未通过"，也不该引导"修复环境"（环境没问题）
                    println!("  → 强制安装 CPU 版本（--skip-oss-cuda --skip-oss-vulkan）");
                }
            }
            ensure_bundled_binary(SERVICE_NAME, install_dir, quiet)?;
            // 互斥清 →CPU 方向全套：marker + CUDA 专属 .so（vendor CPU 二进制已
            // 顶替 voice-cli，残留 .so 会让下次误判回 CUDA 档）
            remove_vulkan_marker(install_dir);
            remove_cuda_exclusive_files(install_dir);
            Ok(())
        }
    }
}

fn download_oss_cuda_bundle(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    // 档位互斥清入口（含下方 already-present 早退）：清 vulkan marker，防
    // "cuda bundle 完整 + marker 残留"的双档并存态（手工摆放/中断安装场景）
    remove_vulkan_marker(install_dir);
    let version = deploy_asset_version();
    // 已装且版本一致才跳过——marker 版本不同/未知（旧安装）都重下，否则已装
    // GPU 档的机器 upgrade 永远拿不到新二进制（present 只判文件齐全）
    if voice_cli_cuda_bundle_present(install_dir)
        && voice_cli_bundle_marker_version(install_dir, VOICE_CLI_CUDA_BUNDLE_MARKER).as_deref()
            == Some(version.as_str())
    {
        if !quiet {
            println!("  binary: CUDA bundle already present, skipping download");
        }
        return Ok(());
    }
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

    download_and_extract_bundle_atomic(
        &url,
        install_dir,
        VOICE_CLI_CUDA_BUNDLE_FILES,
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
    // 版本 marker（不在 tar 内，安装器写入；写失败不阻断——代价仅是下次升级多下一次）
    let _ = fs::write(
        install_dir.join(VOICE_CLI_CUDA_BUNDLE_MARKER),
        format!("cuda {version}"),
    );
    Ok(())
}

fn download_oss_vulkan_bundle(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    // 档位互斥清入口（含下方 already-present 早退）：清 CUDA bundle 专属 .so——
    // vulkan 档的 sherpa 是 CPU 版，providers_cuda 残留会让 cuda_present 误判
    // 为 true（手工摆放/中断安装的双档并存态同样要清）
    for stale in [
        "libonnxruntime_providers_cuda.so",
        "libonnxruntime_providers_shared.so",
    ] {
        let _ = fs::remove_file(install_dir.join(stale));
    }
    let version = deploy_asset_version();
    // 已装且 marker 版本一致才跳过（marker 由 pack 脚本随 tar 写入 "vulkan {VERSION}"）；
    // 版本不同/未知一律重下——否则 vulkan 档机器的 upgrade 永远空操作
    if voice_cli_vulkan_bundle_present(install_dir)
        && voice_cli_bundle_marker_version(install_dir, VOICE_CLI_VULKAN_BUNDLE_MARKER).as_deref()
            == Some(version.as_str())
    {
        if !quiet {
            println!("  binary: Vulkan bundle already present, skipping download");
        }
        return Ok(());
    }
    let archive = voice_cli_vulkan_archive_filename(&version);
    let url = if let Some(base) = args.oss_base.as_deref() {
        voice_cli_vulkan_download_url_from_base(base)
    } else if let Some(url) = optional_voice_cli_vulkan_url() {
        url
    } else {
        bail!(
            "prebuilt voice-cli Vulkan bundle requires --oss-base or voiceCliVulkan URL in \
             vendor/templates/manifest.json (upload {archive} to OSS first)"
        );
    };

    download_and_extract_bundle_atomic(
        &url,
        install_dir,
        VOICE_CLI_VULKAN_BUNDLE_FILES,
        "voice-cli-vulkan-prebuilt.tar.gz",
        quiet,
        "voice-cli Vulkan",
    )?;

    if !voice_cli_vulkan_bundle_present(install_dir) {
        bail!(
            "Vulkan bundle incomplete under {} — expected voice-cli, sherpa/onnx .so files \
             and {} marker",
            install_dir.display(),
            VOICE_CLI_VULKAN_BUNDLE_MARKER
        );
    }
    Ok(())
}

/// upgrade 路径构造 bundle 下载参数（跳过模型/伴生目录，仅驱动下载）。
fn upgrade_bundle_args(
    install_dir: &Path,
    oss_base: Option<&str>,
    tier: VoiceCliLinuxTier,
) -> VoiceCliSetupArgs {
    VoiceCliSetupArgs {
        install_dir: Some(install_dir.to_path_buf()),
        use_prebuilt_models: false,
        skip_models: true,
        models: "large-v3".into(),
        oss_base: oss_base.map(String::from),
        use_oss_cuda: tier == VoiceCliLinuxTier::Cuda,
        skip_oss_cuda: false,
        use_oss_vulkan: tier == VoiceCliLinuxTier::Vulkan,
        skip_oss_vulkan: false,
        cuda_lib_dir: None,
        cudnn_lib_dir: None,
    }
}

fn print_bundle_upgraded(label: &str, install_dir: &Path) {
    println!(
        "✅ upgraded voice-cli {label} bundle in {}",
        install_dir.display()
    );
    println!("   Run: deploy-installer voice-cli service restart");
}

fn upgrade_voice_cli(install_dir: &Path, oss_base: Option<&str>) -> Result<()> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return upgrade_bundled_binary(SERVICE_NAME, install_dir);
    }
    // installed-first：保留已装档位升级——顺带修旧问题（已装 CUDA 但驱动临时
    // 不可用/工具包缺失时，旧逻辑会用 vendor CPU 二进制覆盖掉 CUDA 安装）
    if voice_cli_cuda_bundle_present(install_dir) {
        let args = upgrade_bundle_args(install_dir, oss_base, VoiceCliLinuxTier::Cuda);
        download_oss_cuda_bundle(&args, install_dir, false)?;
        print_bundle_upgraded("CUDA", install_dir);
        return Ok(());
    }
    if voice_cli_vulkan_bundle_present(install_dir) {
        let args = upgrade_bundle_args(install_dir, oss_base, VoiceCliLinuxTier::Vulkan);
        download_oss_vulkan_bundle(&args, install_dir, false)?;
        print_bundle_upgraded("Vulkan", install_dir);
        return Ok(());
    }
    if oss_base.is_some() {
        // 显式 --oss-base：用户明确意图，保持历史语义走 CUDA 下载
        let args = upgrade_bundle_args(install_dir, oss_base, VoiceCliLinuxTier::Cuda);
        download_oss_cuda_bundle(&args, install_dir, false)?;
        print_bundle_upgraded("CUDA", install_dir);
        return Ok(());
    }
    // 未装任何 bundle：三档解析（与 install 同款预检；CPU 档回退升级 vendor
    // 二进制，upgrade_bundled_binary 含自动重启）
    let probe_args = upgrade_bundle_args(install_dir, None, VoiceCliLinuxTier::Cpu);
    let probe = linux_tier_probe(&probe_args, install_dir);
    match probe.tier {
        VoiceCliLinuxTier::Cuda => {
            let args = upgrade_bundle_args(install_dir, None, probe.tier);
            download_oss_cuda_bundle(&args, install_dir, false)?;
            print_bundle_upgraded("CUDA", install_dir);
            Ok(())
        }
        VoiceCliLinuxTier::Vulkan => {
            let args = upgrade_bundle_args(install_dir, None, probe.tier);
            // 与 install 的 auto 档同款优雅降级：vulkan 是替用户选的（此路径无
            // 已装 GPU 档可降——installed-first 已排除），资产缺失回退升 CPU
            // vendor 二进制而非整体报错
            match download_oss_vulkan_bundle(&args, install_dir, false) {
                Ok(()) => {
                    print_bundle_upgraded("Vulkan", install_dir);
                    Ok(())
                }
                Err(e) => {
                    println!("  ⚠️ Vulkan bundle 获取失败（{e}）");
                    println!("  → 回退升级 CPU 版本（vendor 二进制）");
                    upgrade_bundled_binary(SERVICE_NAME, install_dir)
                }
            }
        }
        VoiceCliLinuxTier::Cpu => {
            // 此路径 probe 必然真跑过（installed/显式旗标在前面上方已分流），
            // 守卫是防御性的：防止未来把 skip 旗标接进探测参数后用占位值谎报
            if probe.probed {
                print_gpu_preflight_warnings(&probe);
            }
            println!("  → 回退升级 CPU 版本（vendor 二进制）");
            upgrade_bundled_binary(SERVICE_NAME, install_dir)
        }
    }
}

fn setup(
    args: &VoiceCliSetupArgs,
    quiet: bool,
    installing: bool,
) -> Result<(PathBuf, VoiceCliLinuxTier)> {
    let install_dir = resolve_install_dir(args);
    fs::create_dir_all(&install_dir)
        .with_context(|| format!("create install dir {}", install_dir.display()))?;

    if !quiet {
        println!("==> voice-cli setup → {}", install_dir.display());
    }

    let probe = linux_tier_probe(args, &install_dir);
    ensure_voice_cli_binary(args, &install_dir, &probe, quiet)?;
    copy_templates(&install_dir, quiet)?;
    fs::create_dir_all(install_dir.join("models"))
        .with_context(|| format!("create models dir under {}", install_dir.display()))?;
    fs::create_dir_all(install_dir.join("logs"))
        .with_context(|| format!("create logs dir under {}", install_dir.display()))?;

    let config_path = install_dir.join(CONFIG_FILENAME);
    patch_whisper_default_model(&config_path, WHISPER_DEFAULT_MODEL)?;

    if effective_use_prebuilt_models(args) {
        download_prebuilt_whisper(args, &install_dir, quiet)?;
    } else if !quiet {
        println!("  models: skipped (--skip-models); place models/ggml-*.bin manually if needed");
    }

    if !quiet {
        println!("\n✅ setup complete: {}", install_dir.display());
        if whisper_large_v3_present(&install_dir) {
            println!("   Whisper model: {WHISPER_DEFAULT_MODEL} (ggml-large-v3.bin)");
        } else if effective_use_prebuilt_models(args) {
            println!("   Whisper default model: {WHISPER_DEFAULT_MODEL}");
        }
        if probe.tier == VoiceCliLinuxTier::Cuda && voice_cli_cuda_bundle_present(&install_dir) {
            println!("   Binary: voice-cli CUDA bundle (OSS)");
        }
        if probe.tier == VoiceCliLinuxTier::Vulkan && voice_cli_vulkan_bundle_present(&install_dir)
        {
            println!("   Binary: voice-cli Vulkan bundle (OSS)");
        }
        if !installing {
            println!("   Next: deploy-installer voice-cli install");
            println!("   Default listen port: {DEFAULT_PORT} (document-parser uses 8087)");
        }
    }
    Ok((install_dir, probe.tier))
}

fn install_full(args: &VoiceCliSetupArgs) -> Result<()> {
    let (install_dir, tier) = setup(args, false, true)?;
    if !effective_use_prebuilt_models(args) && tier == VoiceCliLinuxTier::Cpu {
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

fn download_prebuilt_whisper(
    args: &VoiceCliSetupArgs,
    install_dir: &Path,
    quiet: bool,
) -> Result<()> {
    let requested = resolve_models_pack(args)?;
    if whisper_pack_satisfied(install_dir, requested) {
        if !quiet {
            let label = match requested {
                WhisperModelsPack::LargeV3 => "ggml-large-v3.bin",
                WhisperModelsPack::All => "all models (tiny…large-v3)",
            };
            println!("  models: {label} already present, skipping download");
        }
        return Ok(());
    }

    // 包解析：--oss-base 显式给 URL 时按请求包直下（资产在不在由下载结果说话）；
    // manifest 路径下 whisperAll 只在部分平台配键——All 无资产回退 large-v3
    //（ggml 模型平台无关、三平台都有；Linux/Windows 上 `--models all` 此前
    // 直接 bail，连默认模型都装不上）；连 large-v3 都无资产才是真错误
    let effective = match effective_pack(
        requested,
        args.oss_base.is_some(),
        optional_whisper_download_url(WhisperModelsPack::All),
        optional_whisper_download_url(WhisperModelsPack::LargeV3),
    ) {
        Some(pack) => pack,
        None => {
            let version = deploy_asset_version();
            let archive = requested.archive_filename(&version);
            bail!(
                "prebuilt Whisper models require --oss-base or whisper URLs in \
                 vendor/templates/manifest.json (upload {archive} to OSS first)"
            );
        }
    };
    if effective != requested {
        println!(
            "  models: WARN (--models all 在此平台无 whisperAll 资产，回退 large-v3；\
             全套模型需上传 whisper-ggml-all 资产后重跑，或手动放置 models/ggml-*.bin)"
        );
        if whisper_pack_satisfied(install_dir, effective) {
            if !quiet {
                println!("  models: ggml-large-v3.bin already present, skipping download");
            }
            return Ok(());
        }
    }

    let url = if let Some(base) = args.oss_base.as_deref() {
        whisper_download_url_from_base(base, effective)
    } else {
        match optional_whisper_download_url(effective) {
            Some(url) => url,
            None => bail!(
                "prebuilt Whisper models unavailable: no whisper URL for this platform in \
                 vendor/templates/manifest.json — pass --oss-base or upload the tarball first"
            ),
        }
    };
    download_and_extract_tarball(
        &url,
        install_dir,
        "whisper-prebuilt.tar.gz",
        quiet,
        "models",
    )?;
    ensure_whisper_pack_models(install_dir, effective)
}

/// 请求包 → 实际可下载包（纯函数，便于单测）：--oss-base 恒按请求包；manifest
/// 路径下 All 无本平台资产时回退 LargeV3；LargeV3 亦无资产返回 None（真错误）
fn effective_pack(
    requested: WhisperModelsPack,
    oss_base: bool,
    all_url: Option<String>,
    large_v3_url: Option<String>,
) -> Option<WhisperModelsPack> {
    use WhisperModelsPack::{All, LargeV3};
    if oss_base {
        return Some(requested);
    }
    match requested {
        LargeV3 => large_v3_url.map(|_| LargeV3),
        All => all_url
            .map(|_| All)
            .or_else(|| large_v3_url.map(|_| LargeV3)),
    }
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

#[cfg(test)]
mod tests {
    use super::effective_pack;
    use crate::WhisperModelsPack::{All, LargeV3};

    /// `--models all` 平台回归：manifest 无 whisperAll 键的平台（Linux/Windows）
    /// 回退 large-v3 而不是 bail；--oss-base 恒按请求包
    #[test]
    fn effective_pack_falls_back_to_large_v3_when_all_missing() {
        let url = || Some("https://oss/whisper.tar.gz".to_string());

        // --oss-base：按请求包直下
        assert_eq!(effective_pack(All, true, None, None), Some(All));
        assert_eq!(effective_pack(LargeV3, true, None, None), Some(LargeV3));

        // 两包都有资产（darwin-arm64 现状）：按请求包
        assert_eq!(effective_pack(All, false, url(), url()), Some(All));

        // All 无资产、LargeV3 有（linux/windows 现状）：回退 LargeV3
        assert_eq!(effective_pack(All, false, None, url()), Some(LargeV3));

        // 连 LargeV3 都无：None（真错误，调用方 bail）
        assert_eq!(effective_pack(All, false, None, None), None);
        assert_eq!(effective_pack(LargeV3, false, None, None), None);
        assert_eq!(effective_pack(LargeV3, false, None, url()), Some(LargeV3));
    }
}
