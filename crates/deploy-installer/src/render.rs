use crate::error::{InstallerError, Result};
use crate::spec::ServiceSpec;
use std::path::Path;

/// Validate systemd unit / drop-in basename: single path segment, safe charset.
pub fn validate_unit_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(InstallerError::InvalidName {
            name: name.to_string(),
            reason: "name is empty".into(),
        });
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(InstallerError::InvalidName {
            name: name.to_string(),
            reason: "path separators or '..' are not allowed".into(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(InstallerError::InvalidName {
            name: name.to_string(),
            reason: "only [A-Za-z0-9._-] allowed".into(),
        });
    }
    Ok(())
}

/// Reject characters that can inject systemd unit directives into scalar fields.
pub fn sanitize_unit_value(field: &str, value: &str) -> Result<String> {
    if value.is_empty() {
        return Err(InstallerError::InvalidField {
            field: field.to_string(),
            reason: "value is empty".into(),
        });
    }
    if value.contains('\n') || value.contains('\r') || value.contains('%') || value.contains('\0') {
        return Err(InstallerError::InvalidField {
            field: field.to_string(),
            reason: "contains newline, %, or NUL".into(),
        });
    }
    Ok(value.to_string())
}

/// Reject path characters that can inject systemd unit directives.
pub fn sanitize_path(path: &Path) -> Result<String> {
    let s = path.to_str().ok_or_else(|| InstallerError::InvalidPath {
        path: path.display().to_string(),
        reason: "path is not valid UTF-8".into(),
    })?;
    if s.is_empty() {
        return Err(InstallerError::InvalidPath {
            path: s.to_string(),
            reason: "path is empty".into(),
        });
    }
    if s.contains('\n') || s.contains('\r') || s.contains('%') || s.contains('\0') {
        return Err(InstallerError::InvalidPath {
            path: s.to_string(),
            reason: "path contains newline, %, or NUL".into(),
        });
    }
    Ok(s.to_string())
}

fn sanitize_arg(arg: &str) -> Result<String> {
    if arg.is_empty() {
        return Err(InstallerError::InvalidArg {
            arg: arg.to_string(),
            reason: "argument is empty".into(),
        });
    }
    if arg.contains('\n') || arg.contains('\r') || arg.contains('%') || arg.contains('\0') {
        return Err(InstallerError::InvalidArg {
            arg: arg.to_string(),
            reason: "argument contains newline, %, or NUL".into(),
        });
    }
    // Quote if whitespace or shell metacharacters appear.
    if arg
        .chars()
        .any(|c| c.is_whitespace() || "\"'\\$`".contains(c))
    {
        let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
        Ok(format!("\"{escaped}\""))
    } else {
        Ok(arg.to_string())
    }
}

/// Join ExecStart argv into a single systemd ExecStart= value.
pub fn shell_join(args: &[String]) -> Result<String> {
    if args.is_empty() {
        return Err(InstallerError::EmptyExecStart);
    }
    let parts: Result<Vec<String>> = args.iter().map(|a| sanitize_arg(a)).collect();
    Ok(parts?.join(" "))
}

/// Render a complete systemd unit file from `ServiceSpec`.
pub fn render_unit(spec: &ServiceSpec) -> Result<String> {
    validate_unit_name(&spec.name)?;
    for d in &spec.drop_ins {
        validate_unit_name(&d.name)?;
    }

    let description = sanitize_unit_value("Description", &spec.description)?;
    let user = sanitize_unit_value("User", &spec.identity.user)?;
    let group = sanitize_unit_value("Group", &spec.identity.group)?;

    let mut s = String::new();
    s.push_str("[Unit]\n");
    s.push_str(&format!("Description={description}\n"));
    s.push_str("After=network.target\n\n");

    s.push_str("[Service]\n");
    s.push_str("Type=simple\n");
    s.push_str(&format!("User={user}\n"));
    s.push_str(&format!("Group={group}\n"));
    s.push_str(&format!(
        "WorkingDirectory={}\n",
        sanitize_path(&spec.install_dir)?
    ));
    if let Some(env_file) = &spec.env_file {
        s.push_str(&format!("EnvironmentFile={}\n", sanitize_path(env_file)?));
    }
    let exec_start = if spec.exec_start.is_empty() {
        crate::exec_argv::default_exec_argv(spec)
    } else {
        spec.exec_start.clone()
    };
    s.push_str(&format!("ExecStart={}\n", shell_join(&exec_start)?));
    for (k, v) in &spec.extra_env {
        let key = sanitize_unit_value("Environment.key", k)?;
        let value = sanitize_unit_value("Environment.value", v)?;
        let value = if value.chars().any(|c| c.is_whitespace()) {
            format!("\"{}\"", value.replace('"', "\\\""))
        } else {
            value
        };
        s.push_str(&format!("Environment={key}={value}\n"));
    }
    s.push_str("Restart=on-failure\n");
    s.push_str("RestartSec=5\n");
    if let Some(ks) = &spec.kill_signal {
        let ks = sanitize_unit_value("KillSignal", ks)?;
        s.push_str(&format!("KillSignal={ks}\n"));
    }
    if let Some(t) = spec.timeout_stop_sec {
        s.push_str(&format!("TimeoutStopSec={t}\n"));
    }
    if let Some(id) = &spec.syslog_identifier {
        let id = sanitize_unit_value("SyslogIdentifier", id)?;
        s.push_str(&format!("SyslogIdentifier={id}\n"));
    }
    if !spec.supplementary_groups.is_empty() {
        for g in &spec.supplementary_groups {
            sanitize_unit_value("SupplementaryGroups", g)?;
        }
        s.push_str(&format!(
            "SupplementaryGroups={}\n",
            spec.supplementary_groups.join(" ")
        ));
    }
    s.push_str("StandardOutput=journal\n");
    s.push_str("StandardError=journal\n\n");

    s.push_str("[Install]\n");
    s.push_str("WantedBy=multi-user.target\n");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sanitize_path_rejects_percent() {
        let err = sanitize_path(Path::new("/tmp/foo%n")).unwrap_err();
        assert!(err.to_string().contains('%') || err.to_string().contains("newline"));
    }

    #[test]
    fn shell_join_rejects_empty() {
        assert!(matches!(
            shell_join(&[]),
            Err(InstallerError::EmptyExecStart)
        ));
    }

    #[test]
    fn shell_join_quotes_spaces() {
        let joined = shell_join(&[
            "/opt/bin".into(),
            "run".into(),
            "--config".into(),
            "/opt/my dir/config.yml".into(),
        ])
        .unwrap();
        assert!(joined.contains("\"/opt/my dir/config.yml\""));
    }

    #[test]
    fn sanitize_path_ok() {
        assert_eq!(
            sanitize_path(&PathBuf::from("/opt/voice-cli")).unwrap(),
            "/opt/voice-cli"
        );
    }

    #[test]
    fn validate_unit_name_rejects_traversal() {
        assert!(validate_unit_name("../evil").is_err());
        assert!(validate_unit_name("a/b").is_err());
        assert!(validate_unit_name("voice-cli").is_ok());
        assert!(validate_unit_name("cuda-sherpa").is_ok());
    }

    #[test]
    fn sanitize_unit_value_rejects_newline() {
        assert!(sanitize_unit_value("User", "a\nExecStart=/bin/evil").is_err());
    }
}
