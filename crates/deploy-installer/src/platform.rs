/// Active service manager backend for the current OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceBackend {
    Systemd,
    Launchd,
    /// Windows 任务计划程序（schtasks）：外部管理控制台 exe，与 launchd 的
    /// 用户级代理、systemd 的系统级单元同层级的服务生命周期后端。
    TaskScheduler,
}

/// Return the service manager backend for this build target.
pub fn current_backend() -> ServiceBackend {
    if cfg!(target_os = "macos") {
        ServiceBackend::Launchd
    } else if cfg!(target_os = "windows") {
        ServiceBackend::TaskScheduler
    } else {
        ServiceBackend::Systemd
    }
}
