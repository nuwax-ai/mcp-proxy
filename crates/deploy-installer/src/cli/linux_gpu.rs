//! voice-cli Linux GPU 三档决策（CUDA / Vulkan / CPU）：
//! 档位梯子（纯函数）、CUDA 与 Vulkan 运行时预检、Vulkan GPU 探针子进程、
//! CUDA systemd drop-in 构建。探测值由 [`crate::cli::voice_cli`] 的
//! `linux_tier_probe` 统一采集后经梯子解析——本模块不直接触发安装动作。

use crate::DropIn;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// voice-cli Linux 二进制档位（vendor CPU / CUDA OSS bundle / Vulkan OSS bundle）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceCliLinuxTier {
    Cuda,
    Vulkan,
    Cpu,
}

/// 三档解析入参（纯函数，任意平台可单测；探测值由调用方采集）。
#[derive(Debug, Clone, Copy)]
pub struct LinuxTierInputs {
    /// 显式 --use-oss-cuda
    pub use_cuda: bool,
    /// --skip-oss-cuda（跳过 cuda 档，仍可自动 vulkan）
    pub skip_cuda: bool,
    /// 显式 --use-oss-vulkan
    pub use_vulkan: bool,
    /// --skip-oss-vulkan（与 skip_cuda 同给 = 强制 CPU）
    pub skip_vulkan: bool,
    /// CUDA 预检通过（nvidia-smi + libcublas）
    pub cuda_ok: bool,
    /// Vulkan 预检通过（loader + 硬件 GPU 探针）
    pub vulkan_ok: bool,
    /// 已装 CUDA bundle（voice_cli_cuda_bundle_present）
    pub cuda_installed: bool,
    /// 已装 Vulkan bundle（voice_cli_vulkan_bundle_present）
    pub vulkan_installed: bool,
}

/// 三档解析梯子（顺序即优先级）：
/// 1. 显式 use_* 直取（clap 已互斥）；2. 双 skip = 强制 CPU——**先于 installed**，
///    已装 GPU 档的机器也要有强制回 CPU 的逃生门；3. 已装档位幂等保留（重装/
///    升级不漂移）；4. 预检自动档，cuda 先于 vulkan（NVIDIA 机器即使同时有
///    vulkan ICD 也走 CUDA——sherpa/TTS 的 CUDA 加速只有 CUDA 档有）。
pub fn resolve_linux_tier(i: &LinuxTierInputs) -> VoiceCliLinuxTier {
    if let Some(tier) = early_linux_tier(i) {
        return tier;
    }
    if !i.skip_cuda && i.cuda_ok {
        return VoiceCliLinuxTier::Cuda;
    }
    if !i.skip_vulkan && i.vulkan_ok {
        return VoiceCliLinuxTier::Vulkan;
    }
    VoiceCliLinuxTier::Cpu
}

/// 梯子的前三级（显式旗标 / 双 skip / 已装档位）——**与运行时探测无关**。
///
/// 返回 `None` 表示需要真探测（预检自动档）。拆出来的目的：让调用方在梯子
/// 前三级已定时**跳过探测**（nvidia-smi/ldconfig/探针子进程），尤其双 skip
/// 强制 CPU 的用户不该被坏驱动的 10s 探针超时卡住。`resolve_linux_tier` 与
/// 懒探测调用方（voice-cli 侧 `linux_tier_probe`）共用本函数，两级不会漂移。
pub fn early_linux_tier(i: &LinuxTierInputs) -> Option<VoiceCliLinuxTier> {
    if i.use_cuda {
        return Some(VoiceCliLinuxTier::Cuda);
    }
    if i.use_vulkan {
        return Some(VoiceCliLinuxTier::Vulkan);
    }
    if i.skip_cuda && i.skip_vulkan {
        return Some(VoiceCliLinuxTier::Cpu);
    }
    if i.cuda_installed {
        return Some(VoiceCliLinuxTier::Cuda);
    }
    if i.vulkan_installed {
        return Some(VoiceCliLinuxTier::Vulkan);
    }
    None
}

/// Linux CUDA 运行时预检（voice-cli CUDA bundle 的启动前置）。
///
/// bundle 不含 libcublas——它依赖系统 CUDA 工具包（/usr/local/cuda/lib64）或
/// ldconfig 可达的 CUDA 库；nvidia-smi 只证明驱动。两者任一缺失时 CUDA bundle
/// 装上也无法启动（131 无 GPU 机实测：libcublas.so.12 not found 崩溃循环）。
pub fn linux_cuda_runtime_available() -> (bool, bool) {
    // nvidia-smi 可执行且退出成功
    let has_smi = Command::new("nvidia-smi")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    // libcublas：CUDA 工具包目录存在，或 ldconfig 缓存可解析
    let cublas_in_toolkit = Path::new("/usr/local/cuda/lib64/libcublas.so.12").exists();
    let cublas_in_ldconfig = Command::new("ldconfig")
        .arg("-p")
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("libcublas.so.12"));
    (has_smi, cublas_in_toolkit || cublas_in_ldconfig)
}

/// 纯函数：CUDA 预检结果 → 提示文案（四分支单测；空串 = 预检通过无需提示）。
pub fn cuda_preflight_report(has_smi: bool, cublas_ok: bool) -> String {
    match (has_smi, cublas_ok) {
        (true, true) => String::new(),
        (true, false) => "检测到 NVIDIA 驱动（nvidia-smi）但缺 CUDA 工具包库（libcublas.so.12）——\n  安装: sudo apt install cuda-toolkit-12-6（或 nvidia-cuda-toolkit）".into(),
        (false, true) => "未检测到 nvidia-smi（无 NVIDIA 驱动/GPU）——CUDA bundle 无法使用 GPU\n  如需 GPU 加速: 安装 NVIDIA 驱动后重装".into(),
        (false, false) => "未检测到 NVIDIA GPU 与 CUDA 运行时（nvidia-smi、libcublas 均缺失）——\n  CUDA bundle 在本机无法启动；GPU 加速需: NVIDIA 驱动 + cuda-toolkit".into(),
    }
}

/// Vulkan GPU 探针超时：坏驱动常见死等而非返回错误（GPU 卡死态），必须可超时。
const VULKAN_PROBE_TIMEOUT_SECS: u64 = 10;

/// 纯函数：探针退出码 → (loader_ok, gpu_ok)。
///
/// `None` = 超时（探针仍在运行被 kill）或无法启动/信号崩溃——`code()` 对信号
/// 退出也返回 None，两者同按"无 Vulkan"处理（宁保守回 CPU，不挡安装）。
/// 0=有真 GPU；3=loader 在但只枚举到 CPU 型设备（软件渲染）；4=loader 缺失。
pub fn map_probe_exit(code: Option<i32>) -> (bool, bool) {
    match code {
        Some(0) => (true, true),
        Some(3) => (true, false),
        // 4 与其他未知码（含信号崩溃的 Some(kill 信号语义差异)）一律无 Vulkan
        _ => (false, false),
    }
}

/// 自 reexec 跑 `__probe-vulkan` 探针子进程（带超时）。
///
/// 为什么子进程：vkCreateInstance/枚举会把机器上每个 ICD 驱动 .so dlopen 进调用
/// 进程执行厂商代码——坏驱动（升级到一半版本不匹配 assert / ICD 残尸 / GPU 卡死
/// 态死等）可能 SIGSEGV 或挂起，Rust 接不住信号。隔离后爆炸半径从"安装中断"
/// 缩到"降档继续"。Chrome 把 GPU 工作放独立进程同因。
fn spawn_vulkan_probe() -> Option<std::process::ExitStatus> {
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(exe)
        .arg("__probe-vulkan")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(VULKAN_PROBE_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
}

/// Linux Vulkan 运行时预检（voice-cli Vulkan bundle 的启动前置）。
///
/// Vulkan 二进制硬链 libvulkan.so.1（加载期失败，服务内无运行时回退机会），
/// loader 缺失的机器装了 bundle 起不来。GPU 判定交给探针子进程：ash 标准 API
/// 建零扩展 instance → 枚举设备 → deviceType 由驱动自报（llvmpipe 如实报 CPU
/// 型，无需文件名黑名单猜软件渲染）。
pub fn linux_vulkan_runtime_available() -> (bool, bool) {
    match spawn_vulkan_probe() {
        Some(status) => map_probe_exit(status.code()),
        // 探针起不来（current_exe 异常等罕见环境）：保守按无 Vulkan
        None => (false, false),
    }
}

/// 纯函数：Vulkan 预检结果 → 提示文案（四分支单测；空串 = 预检通过无需提示）。
pub fn vulkan_preflight_report(loader_ok: bool, gpu_ok: bool) -> String {
    match (loader_ok, gpu_ok) {
        (true, true) => String::new(),
        (true, false) => "Vulkan loader 在但未枚举到硬件 GPU（仅软件渲染或 GPU 驱动缺失）——\n  \
             安装 GPU 驱动: sudo apt install -y mesa-vulkan-drivers（AMD/Intel）"
            .into(),
        // gpu_ok 蕴含 loader_ok，(false, true) 不可达；防御性按 loader 缺失处理
        _ => "未检测到 Vulkan 运行时（libvulkan.so.1 缺失或驱动探测失败）——\n  \
              安装: sudo apt install -y libvulkan1 mesa-vulkan-drivers"
            .into(),
    }
}

/// systemd drop-in for CUDA/cuDNN `LD_LIBRARY_PATH` (install_dir first for bundled .so).
pub fn build_cuda_sherpa_drop_in(
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
    let ld = parts.join(":");
    Some(DropIn {
        name: "cuda-sherpa".into(),
        content: format!("[Service]\nEnvironment=LD_LIBRARY_PATH={ld}\n"),
    })
}

/// Default NVIDIA CUDA toolkit lib dir when present on the host.
pub fn default_cuda_lib_dir() -> Option<PathBuf> {
    for candidate in ["/usr/local/cuda/lib64", "/usr/local/cuda/lib"] {
        let p = PathBuf::from(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Best-effort cuDNN lib discovery (explicit env, then document-parser venv).
pub fn detect_cudnn_lib_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CUDNN_LIB_DIR") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let venv_lib = PathBuf::from(home).join("document-parser/venv/lib");
        if let Ok(entries) = fs::read_dir(&venv_lib) {
            for entry in entries.flatten() {
                let cudnn = entry.path().join("site-packages/nvidia/cudnn/lib");
                if cudnn.is_dir() {
                    return Some(cudnn);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuda_preflight_report_all_branches() {
        assert_eq!(cuda_preflight_report(true, true), "");
        assert!(cuda_preflight_report(true, false).contains("cuda-toolkit"));
        assert!(cuda_preflight_report(false, true).contains("nvidia-smi"));
        assert!(cuda_preflight_report(false, false).contains("均缺失"));
    }

    #[test]
    fn vulkan_preflight_report_all_branches() {
        assert_eq!(vulkan_preflight_report(true, true), "");
        assert!(vulkan_preflight_report(true, false).contains("mesa-vulkan-drivers"));
        assert!(vulkan_preflight_report(false, false).contains("libvulkan1"));
        // (false, true) 不可达，防御分支按 loader 缺失处理
        assert!(vulkan_preflight_report(false, true).contains("libvulkan1"));
    }

    #[test]
    fn map_probe_exit_codes() {
        assert_eq!(map_probe_exit(Some(0)), (true, true));
        assert_eq!(map_probe_exit(Some(3)), (true, false));
        // 4=loader 缺失；其他未知码同"无 Vulkan"
        assert_eq!(map_probe_exit(Some(4)), (false, false));
        assert_eq!(map_probe_exit(Some(101)), (false, false));
        // None = 超时/信号崩溃（code() 对信号退出返回 None）→ 保守无 Vulkan
        assert_eq!(map_probe_exit(None), (false, false));
    }

    /// installed 压过单 skip：skip 只挡"自动选档"，不推翻已装事实（推翻用双 skip）。
    #[test]
    fn resolve_linux_tier_installed_beats_single_skip() {
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                cuda_installed: true,
                vulkan_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
    }

    /// 三档梯子真值表（纯函数，任意平台可测）。
    #[test]
    fn resolve_linux_tier_explicit_flags_win() {
        // 显式 use_cuda 优先（即使 vulkan 也 ok / 已装 vulkan）
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                use_cuda: true,
                vulkan_ok: true,
                vulkan_installed: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                use_vulkan: true,
                cuda_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Vulkan
        );
    }

    #[test]
    fn resolve_linux_tier_double_skip_forces_cpu_over_installed() {
        // 双 skip 先于 installed——已装 GPU 档的机器也能强制回 CPU
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                skip_vulkan: true,
                cuda_installed: true,
                vulkan_installed: true,
                cuda_ok: true,
                vulkan_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cpu
        );
    }

    #[test]
    fn resolve_linux_tier_installed_is_idempotent() {
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                cuda_installed: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                vulkan_installed: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Vulkan
        );
        // cuda_installed 优先于 vulkan_installed（同梯子第 4/5 级顺序）
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                cuda_installed: true,
                vulkan_installed: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
    }

    #[test]
    fn resolve_linux_tier_probe_prefers_cuda_then_vulkan() {
        // AMD 机：无 CUDA、vulkan 探针过 → Vulkan（131 场景）
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                vulkan_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Vulkan
        );
        // NVIDIA 机同时有 vulkan ICD → CUDA 优先
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                cuda_ok: true,
                vulkan_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
        // 全缺 → CPU
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs { ..all_false() }),
            VoiceCliLinuxTier::Cpu
        );
    }

    #[test]
    fn resolve_linux_tier_single_skip_skips_only_that_tier() {
        // --skip-oss-cuda：跳过 cuda 档仍可自动 vulkan（语义变化点，CHANGELOG 注明）
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                cuda_ok: true,
                vulkan_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Vulkan
        );
        // --skip-oss-vulkan：cuda 仍可用
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                skip_vulkan: true,
                cuda_ok: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cuda
        );
        // 单 skip + 无任何 GPU → CPU
        assert_eq!(
            resolve_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                ..all_false()
            }),
            VoiceCliLinuxTier::Cpu
        );
    }

    /// early 梯子（与探测无关的前三级）：命中返回 Some，需探测返回 None。
    #[test]
    fn early_linux_tier_skips_probe_when_decided() {
        // 显式旗标/双 skip/已装档位 → 无需探测
        assert_eq!(
            early_linux_tier(&LinuxTierInputs {
                use_cuda: true,
                ..all_false()
            }),
            Some(VoiceCliLinuxTier::Cuda)
        );
        assert_eq!(
            early_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                skip_vulkan: true,
                ..all_false()
            }),
            Some(VoiceCliLinuxTier::Cpu)
        );
        assert_eq!(
            early_linux_tier(&LinuxTierInputs {
                vulkan_installed: true,
                ..all_false()
            }),
            Some(VoiceCliLinuxTier::Vulkan)
        );
        // 无旗标、未装 → 需要探测（即使探测值全真，early 也不能替它决定）
        assert_eq!(early_linux_tier(&LinuxTierInputs { ..all_false() }), None);
        assert_eq!(
            early_linux_tier(&LinuxTierInputs {
                cuda_ok: true,
                vulkan_ok: true,
                ..all_false()
            }),
            None
        );
        // 单 skip 不构成 early 决定（仍需探测另一档）
        assert_eq!(
            early_linux_tier(&LinuxTierInputs {
                skip_cuda: true,
                ..all_false()
            }),
            None
        );
    }

    fn all_false() -> LinuxTierInputs {
        LinuxTierInputs {
            use_cuda: false,
            skip_cuda: false,
            use_vulkan: false,
            skip_vulkan: false,
            cuda_ok: false,
            vulkan_ok: false,
            cuda_installed: false,
            vulkan_installed: false,
        }
    }
}
