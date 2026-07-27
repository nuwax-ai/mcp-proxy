use crate::error::{InstallerError, Result};
use crate::spec::ServiceSpec;

/// Render a macOS LaunchAgent plist from `ServiceSpec`.
///
/// `ProgramArguments` exec the binary directly. Secrets stay in dotenv files loaded by the
/// service binary when needed (launchd has no `EnvironmentFile=`).
/// Always injects `PATH` / `HOME` / `TMPDIR`; also emits `spec.extra_env` (e.g. `RUST_LOG`).
pub fn render_launchd_plist(spec: &ServiceSpec, run_at_load: bool) -> Result<String> {
    let label = spec.launchd_label();
    let install_dir = crate::render::sanitize_path(&spec.install_dir)?;
    let program_args = launchd_program_arguments(spec)?;
    let stdout = format!("{install_dir}/logs/launchd.stdout.log");
    let stderr = format!("{install_dir}/logs/launchd.stderr.log");
    let path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Library/Frameworks/Python.framework/Versions/Current/bin";
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = crate::render::sanitize_unit_value("HOME", &home)?;
    let tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let tmpdir = crate::render::sanitize_unit_value("TMPDIR", &tmpdir)?;
    let extra_env_xml = launchd_extra_env_xml(spec)?;
    let run_at_load_xml = if run_at_load { "<true/>" } else { "<false/>" };
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
{program_args}    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>{path}</string>
        <key>HOME</key>
        <string>{home}</string>
        <key>TMPDIR</key>
        <string>{tmpdir}</string>
{extra_env_xml}    </dict>
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

fn launchd_extra_env_xml(spec: &ServiceSpec) -> Result<String> {
    let mut out = String::new();
    for (key, value) in &spec.extra_env {
        if matches!(key.as_str(), "PATH" | "HOME" | "TMPDIR") {
            continue;
        }
        let key = crate::render::sanitize_unit_value("EnvironmentVariables", key)?;
        let value = crate::render::sanitize_unit_value("EnvironmentVariables", value)?;
        out.push_str(&format!(
            "        <key>{key}</key>\n        <string>{value}</string>\n"
        ));
    }
    Ok(out)
}

fn launchd_program_arguments(spec: &ServiceSpec) -> Result<String> {
    let argv = if spec.exec_start.is_empty() {
        crate::exec_argv::default_exec_argv(spec)
    } else {
        spec.exec_start.clone()
    };
    let mut out = Vec::with_capacity(argv.len());
    for (i, arg) in argv.iter().enumerate() {
        if i == 0 || looks_like_path(arg) {
            out.push(crate::render::sanitize_path(std::path::Path::new(arg))?);
        } else {
            out.push(crate::render::sanitize_unit_value("ProgramArguments", arg)?);
        }
    }
    Ok(out
        .into_iter()
        .map(|a| format!("        <string>{a}</string>\n"))
        .collect())
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with('/') || s.ends_with(".yml") || s.ends_with(".yaml")
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
    fn render_plist_contains_label_and_binary() {
        let spec = ServiceSpec {
            name: "document-parser".into(),
            description: "test".into(),
            identity: ServiceIdentity {
                user: "u".into(),
                group: "g".into(),
            },
            install_dir: PathBuf::from("/opt/document-parser"),
            exec_start: vec![
                "/opt/document-parser/document-parser".into(),
                "--config".into(),
                "/opt/document-parser/config.yml".into(),
                "server".into(),
            ],
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
        assert!(plist.contains("<string>/opt/document-parser/document-parser</string>"));
        assert!(plist.contains("<string>--config</string>"));
        assert!(plist.contains("<string>/opt/document-parser/config.yml</string>"));
        assert!(plist.contains("<string>server</string>"));
        assert!(!plist.contains("run-server.sh"));
    }

    #[test]
    fn render_plist_includes_extra_env() {
        let spec = ServiceSpec {
            name: "voice-cli".into(),
            description: "test".into(),
            identity: ServiceIdentity {
                user: "u".into(),
                group: "g".into(),
            },
            install_dir: PathBuf::from("/opt/voice-cli"),
            exec_start: vec![
                "/opt/voice-cli/voice-cli".into(),
                "server".into(),
                "run".into(),
                "--config".into(),
                "/opt/voice-cli/config.yml".into(),
            ],
            env_file: None,
            extra_env: vec![("RUST_LOG".into(), "info".into())],
            kill_signal: None,
            timeout_stop_sec: None,
            syslog_identifier: None,
            drop_ins: vec![],
            supplementary_groups: vec![],
            required_paths: vec![],
            listen_port: None,
        };
        let plist = render_launchd_plist(&spec, true).unwrap();
        assert!(plist.contains("<key>RUST_LOG</key>"));
        assert!(plist.contains("<string>info</string>"));
        assert!(plist.contains("<string>server</string>"));
        assert!(plist.contains("<string>run</string>"));
    }
}
