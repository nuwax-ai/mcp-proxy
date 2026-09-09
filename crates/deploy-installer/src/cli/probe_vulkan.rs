//! 隐藏子命令 `__probe-vulkan`：Vulkan GPU 探针（三档检测用，不出现在帮助里）。
//!
//! 探测走 Vulkan 标准能力查询（ash 绑定）：零扩展建 instance（无头安全，不碰
//! X11/Wayland surface）→ 枚举物理设备 → deviceType 由**驱动自报**（llvmpipe 等
//! 软件渲染如实上报 CPU 型，无需启发式）。父进程以子进程方式运行本探针并设
//! 超时（见 assets::spawn_vulkan_probe），探针异常一律按"无 Vulkan"降级——
//! 崩溃域被隔离在探针进程内，安装器本体不受坏驱动影响。

use anyhow::Result;

/// 探针退出码语义（父侧解析见 [`crate::cli::assets::map_probe_exit`]）。
#[cfg(target_os = "linux")]
const EXIT_GPU_FOUND: i32 = 0;
#[cfg(target_os = "linux")]
const EXIT_NO_HARDWARE_GPU: i32 = 3;
const EXIT_NO_LOADER: i32 = 4;

/// 运行探针并以退出码汇报（不返回 Ok——本子命令只服务于父进程探测）。
// clippy::exit：探针协议就是退出码，非库内正常控制流
#[allow(clippy::exit)]
pub fn run() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        probe_linux();
    }
    #[cfg(not(target_os = "linux"))]
    {
        // 非 Linux：探针不适用（vulkan 档仅 linux-x64），按 loader 缺失语义退出
        std::process::exit(EXIT_NO_LOADER);
    }
}

#[cfg(target_os = "linux")]
fn probe_linux() -> ! {
    use ash::vk;

    // SAFETY: Entry::load 初始化 Vulkan 函数指针表（加载 libvulkan.so.1）——
    // ash 标准绑定（wgpu 同款）初始化入口；失败走 loader 缺失退出码
    let Ok(entry) = (unsafe { ash::Entry::load() }) else {
        // libvulkan.so.1 不可加载（发行版 loader 未安装）
        std::process::exit(EXIT_NO_LOADER);
    };
    let app_info = vk::ApplicationInfo::default().application_name(c"nuwax-deploy-installer");
    let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);
    // SAFETY: 零扩展 instance 创建；失败（无可用 ICD 等）→ 仅软件/无驱动语义
    let Ok(instance) = (unsafe { entry.create_instance(&create_info, None) }) else {
        std::process::exit(EXIT_NO_HARDWARE_GPU);
    };
    // SAFETY: 只读枚举物理设备
    let devices = unsafe { instance.enumerate_physical_devices() }.unwrap_or_default();
    let has_hw_gpu = devices.iter().any(|&dev| {
        // SAFETY: 只读设备属性查询
        let props = unsafe { instance.get_physical_device_properties(dev) };
        // 软件渲染（llvmpipe/SwiftShader）驱动自报 CPU 型；真 GPU 为
        // INTEGRATED/DISCRETE/VIRTUAL（virtio-gpu 属 VIRTUAL，也是真设备）
        !matches!(props.device_type, vk::PhysicalDeviceType::CPU)
    });
    // SAFETY: 探测完毕销毁 instance
    unsafe { instance.destroy_instance(None) };
    std::process::exit(if has_hw_gpu {
        EXIT_GPU_FOUND
    } else {
        EXIT_NO_HARDWARE_GPU
    });
}
