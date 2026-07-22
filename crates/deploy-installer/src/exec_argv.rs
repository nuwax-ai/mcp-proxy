//! Shared helpers for deriving LaunchAgent / service-manager argv when `exec_start` is empty.
//!
//! Callers should always set a full `exec_start`; these defaults exist only as a last resort.

use crate::spec::ServiceSpec;
use std::ffi::OsString;
use std::path::PathBuf;

/// Default `ProgramArguments` (including argv0) when `spec.exec_start` is empty.
pub fn default_exec_argv(spec: &ServiceSpec) -> Vec<String> {
    let bin = spec.install_dir.join(&spec.name).display().to_string();
    let cfg = spec.install_dir.join("config.yml").display().to_string();
    match spec.name.as_str() {
        "voice-cli" => vec![bin, "server".into(), "run".into(), "--config".into(), cfg],
        // document-parser and any future service that uses `… --config … server`
        _ => vec![bin, "--config".into(), cfg, "server".into()],
    }
}

pub fn program_and_args(spec: &ServiceSpec) -> (PathBuf, Vec<OsString>) {
    let argv = if spec.exec_start.is_empty() {
        default_exec_argv(spec)
    } else {
        spec.exec_start.clone()
    };
    let program = PathBuf::from(
        argv.first()
            .cloned()
            .unwrap_or_else(|| spec.install_dir.join(&spec.name).display().to_string()),
    );
    let args = argv.into_iter().skip(1).map(OsString::from).collect();
    (program, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ServiceIdentity;
    use std::path::PathBuf;

    fn spec(name: &str) -> ServiceSpec {
        ServiceSpec {
            name: name.into(),
            description: "t".into(),
            identity: ServiceIdentity {
                user: "u".into(),
                group: "g".into(),
            },
            install_dir: PathBuf::from(format!("/opt/{name}")),
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
        }
    }

    #[test]
    fn voice_cli_default_argv() {
        let a = default_exec_argv(&spec("voice-cli"));
        assert_eq!(
            a,
            vec![
                "/opt/voice-cli/voice-cli",
                "server",
                "run",
                "--config",
                "/opt/voice-cli/config.yml",
            ]
        );
    }

    #[test]
    fn document_parser_default_argv() {
        let a = default_exec_argv(&spec("document-parser"));
        assert_eq!(
            a,
            vec![
                "/opt/document-parser/document-parser",
                "--config",
                "/opt/document-parser/config.yml",
                "server",
            ]
        );
    }
}
