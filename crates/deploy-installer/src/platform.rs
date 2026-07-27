/// Active service manager backend for the current OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceBackend {
    Systemd,
    Launchd,
}

/// Return the service manager backend for this build target.
pub fn current_backend() -> ServiceBackend {
    if cfg!(target_os = "macos") {
        ServiceBackend::Launchd
    } else {
        ServiceBackend::Systemd
    }
}
