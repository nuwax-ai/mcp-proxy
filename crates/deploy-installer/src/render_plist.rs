use crate::error::{InstallerError, Result};
use crate::spec::ServiceSpec;

/// Render a macOS LaunchAgent plist from `ServiceSpec`.
///
/// Uses `run-server.sh` in `install_dir` so secrets stay in `.document-parser.env`
/// instead of the plist.
pub fn render_launchd_plist(spec: &ServiceSpec, run_at_load: bool) -> Result<String> {
    let label = spec.launchd_label();
    let install_dir = crate::render::sanitize_path(&spec.install_dir)?;
    let run_script = crate::render::sanitize_path(&spec.run_server_script_path())?;
    let stdout = format!("{install_dir}/logs/launchd.stdout.log");
    let stderr = format!("{install_dir}/logs/launchd.stderr.log");
    let path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin";
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = crate::render::sanitize_unit_value("HOME", &home)?;
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let tmpdir = crate::render::sanitize_unit_value("TMPDIR", &tmpdir)?;
    let run_at_load_xml = if run_at_load {
        "<true/>"
    } else {
        "<false/>"
    };
    let keep_alive_block = if run_at_load {
        r#"    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
"#
    } else {
        ""
    };

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>WorkingDirectory</key>
    <string>{install_dir}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{run_script}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path}</string>
        <key>HOME</key>
        <string>{home}</string>
        <key>TMPDIR</key>
        <string>{tmpdir}</string>
    </dict>
    <key>RunAtLoad</key>
    {run_at_load_xml}
{keep_alive_block}    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#
    ))
}

/// Default `run-server.sh` content for document-parser.
pub fn default_run_server_script() -> &'static str {
    r#"#!/usr/bin/env bash
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin:${PATH:-}"
# launchd may omit HOME; Python/HuggingFace caches need it
if [[ -z "${HOME:-}" ]]; then
  HOME="$(cd ~ && pwd)"
  export HOME
fi
export TMPDIR="${TMPDIR:-/tmp}"
cd "$(dirname "$0")"
set -a
# shellcheck disable=SC1091
source .document-parser.env
set +a
exec ./document-parser --config ./config.yml server
"#
}

/// Write `run-server.sh` into `install_dir` if missing or overwrite when `force`.
pub fn ensure_run_server_script(spec: &ServiceSpec, force: bool) -> Result<()> {
    let path = spec.run_server_script_path();
    if path.exists() && !force {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, default_run_server_script())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms)?;
    }
    Ok(())
}

/// Ensure `logs/` exists under install_dir for launchd stdout/stderr.
pub fn ensure_log_dir(spec: &ServiceSpec) -> Result<()> {
    let logs = spec.install_dir.join("logs");
    std::fs::create_dir_all(&logs).map_err(InstallerError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ServiceIdentity, ServiceSpec};
    use std::path::PathBuf;

    #[test]
    fn render_plist_contains_label() {
        let spec = ServiceSpec {
            name: "document-parser".into(),
            description: "test".into(),
            identity: ServiceIdentity {
                user: "u".into(),
                group: "g".into(),
            },
            install_dir: PathBuf::from("/opt/document-parser"),
            exec_start: vec![],
            env_file: None,
            extra_env: vec![],
            kill_signal: None,
            timeout_stop_sec: None,
            syslog_identifier: None,
            drop_ins: vec![],
            supplementary_groups: vec![],
            required_paths: vec![],
            listen_port: None,
        };
        let plist = render_launchd_plist(&spec, true).unwrap();
        assert!(plist.contains("<string>com.nuwax.document-parser</string>"));
        assert!(plist.contains("/opt/document-parser/run-server.sh"));
    }
}
