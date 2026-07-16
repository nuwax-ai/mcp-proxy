use crate::checks::{self, PrecheckOptions, PrecheckReport};
use crate::error::{InstallerError, Result};
use crate::render::{self, validate_unit_name};
use crate::spec::{DropIn, ServiceSpec};
use crate::systemd;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Options for [`install`].
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// Call `systemctl enable` (default true from callers).
    pub enable: bool,
    /// Call `systemctl restart` after enable (default true; `--no-start` → false).
    pub start: bool,
    /// Only render and print unit text; skip writes and systemctl.
    pub dry_run: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            enable: true,
            start: true,
            dry_run: false,
        }
    }
}

fn write_unit_via_sudo_install(unit_path: &Path, unit_text: &str) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new().map_err(InstallerError::Io)?;
    tmp.write_all(unit_text.as_bytes())?;
    tmp.flush()?;
    systemd::install_file(tmp.path(), unit_path)?;
    Ok(())
}

fn write_dropin_via_sudo(spec: &ServiceSpec, drop_in: &DropIn) -> Result<()> {
    validate_unit_name(&drop_in.name)?;
    let dir = spec.drop_in_dir();
    systemd::mkdir_p(&dir)?;
    let path = dir.join(format!("{}.conf", drop_in.name));
    let mut tmp = tempfile::NamedTempFile::new().map_err(InstallerError::Io)?;
    tmp.write_all(drop_in.content.as_bytes())?;
    tmp.flush()?;
    systemd::install_file(tmp.path(), &path)?;
    Ok(())
}

/// Install (or update) a systemd unit from `spec`.
pub fn install(spec: &ServiceSpec, opts: &InstallOptions) -> Result<PrecheckReport> {
    validate_unit_name(&spec.name)?;

    if opts.start && !opts.enable {
        return Err(InstallerError::Other(
            "invalid InstallOptions: start=true requires enable=true".into(),
        ));
    }

    // Render first so field/name injection fails before any precheck summary noise.
    let unit_text = render::render_unit(spec)?;

    let report = checks::precheck(
        spec,
        &PrecheckOptions {
            require_sudo: !opts.dry_run,
            check_port: !opts.dry_run,
        },
    )?;
    report.print_summary();

    let unit_path = spec.unit_path();

    if opts.dry_run {
        println!("--- {} ---\n{}", unit_path.display(), unit_text);
        for d in &spec.drop_ins {
            println!(
                "--- drop-in {}.service.d/{}.conf ---\n{}",
                spec.name, d.name, d.content
            );
        }
        println!("(dry-run: no files written, systemctl not invoked)");
        return Ok(report);
    }

    write_unit_via_sudo_install(&unit_path, &unit_text)?;
    for d in &spec.drop_ins {
        write_dropin_via_sudo(spec, d)?;
    }

    systemd::daemon_reload()?;
    if opts.enable {
        systemd::enable(&spec.name)?;
    }
    if opts.start {
        systemd::restart(&spec.name)?;
    }

    println!("Installed {}.service → {}", spec.name, unit_path.display());
    if opts.enable {
        println!("  enabled (WantedBy=multi-user.target)");
    }
    if opts.start {
        println!("  restarted");
    }
    Ok(report)
}

/// Stop, disable, remove unit + drop-in directory, daemon-reload.
pub fn uninstall(name: &str) -> Result<()> {
    validate_unit_name(name)?;

    // Best-effort stop/disable (unit may already be absent).
    let stop_err = systemd::stop(name).err();
    let disable_err = systemd::disable(name).err();

    let unit_path = PathBuf::from(format!("/etc/systemd/system/{name}.service"));
    let drop_in_dir = PathBuf::from(format!("/etc/systemd/system/{name}.service.d"));

    systemd::remove_file(&unit_path)?;
    // Drop-in dir may not exist; ignore "No such file" style failures from rm -rf.
    if let Err(e) = systemd::remove_dir_all(&drop_in_dir) {
        let detail = e.to_string();
        if !detail.contains("No such file") && !detail.contains("cannot remove") {
            // Still reload; surface non-missing errors after reload attempt.
            let _ = systemd::daemon_reload();
            return Err(e);
        }
    }
    systemd::daemon_reload()?;

    if let Some(e) = stop_err {
        println!("  note: stop: {e}");
    }
    if let Some(e) = disable_err {
        println!("  note: disable: {e}");
    }
    println!("Uninstalled {name}.service");
    Ok(())
}

/// Print enable/active state, unit contents, and recent journal lines.
pub fn status(name: &str) -> Result<()> {
    validate_unit_name(name)?;
    let enabled = systemd::is_enabled(name).unwrap_or_else(|e| format!("(error: {e})"));
    let active = systemd::is_active(name).unwrap_or_else(|e| format!("(error: {e})"));
    println!("Service: {name}");
    println!("  is-enabled: {enabled}");
    println!("  is-active:  {active}");
    println!();
    match systemd::cat_unit(name) {
        Ok(text) => {
            println!("--- unit ---");
            println!("{text}");
        }
        Err(e) => println!("(could not cat unit: {e})"),
    }
    println!("--- recent logs ---");
    match systemd::recent_logs(name, 30) {
        Ok(logs) => println!("{logs}"),
        Err(e) => println!("(could not read journal: {e})"),
    }
    Ok(())
}

/// Restart an already-installed service.
pub fn restart(name: &str) -> Result<()> {
    validate_unit_name(name)?;
    systemd::restart(name)?;
    println!("Restarted {name}");
    Ok(())
}

/// Helper used by callers that only need to know if a path exists.
pub fn path_exists(path: &std::path::Path) -> bool {
    path.exists()
}

/// Write UTF-8 content to a path (no sudo; for config / .env bootstrap in install_dir).
pub fn write_user_file(path: &std::path::Path, content: &str, mode: Option<u32>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(m);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}
