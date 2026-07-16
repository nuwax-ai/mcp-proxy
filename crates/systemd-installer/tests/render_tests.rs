use std::path::PathBuf;
use systemd_installer::{
    DropIn, InstallerError, ServiceIdentity, ServiceSpec, render_unit, sanitize_path, shell_join,
};

fn voice_cli_spec() -> ServiceSpec {
    ServiceSpec {
        name: "voice-cli".into(),
        description: "voice-cli speech-to-text service".into(),
        identity: ServiceIdentity {
            user: "swufe".into(),
            group: "swufe".into(),
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
        required_paths: vec![PathBuf::from("/opt/voice-cli/config.yml")],
        listen_port: Some(8077),
    }
}

fn document_parser_spec() -> ServiceSpec {
    ServiceSpec {
        name: "document-parser".into(),
        description: "Document Parser Service (MCP document-parser)".into(),
        identity: ServiceIdentity {
            user: "swufe".into(),
            group: "swufe".into(),
        },
        install_dir: PathBuf::from("/opt/document-parser"),
        exec_start: vec![
            "/opt/document-parser/document-parser".into(),
            "server".into(),
        ],
        env_file: Some(PathBuf::from("/opt/document-parser/.document-parser.env")),
        extra_env: vec![],
        kill_signal: Some("SIGINT".into()),
        timeout_stop_sec: Some(60),
        syslog_identifier: Some("document-parser".into()),
        drop_ins: vec![],
        supplementary_groups: vec![],
        required_paths: vec![PathBuf::from("/opt/document-parser/config.yml")],
        listen_port: Some(8087),
    }
}

#[test]
fn render_voice_cli_snapshot() {
    let unit = render_unit(&voice_cli_spec()).unwrap();
    assert!(unit.contains("[Unit]\n"));
    assert!(unit.contains("Description=voice-cli speech-to-text service\n"));
    assert!(unit.contains("User=swufe\n"));
    assert!(unit.contains("Group=swufe\n"));
    assert!(unit.contains("WorkingDirectory=/opt/voice-cli\n"));
    assert!(unit.contains(
        "ExecStart=/opt/voice-cli/voice-cli server run --config /opt/voice-cli/config.yml\n"
    ));
    assert!(unit.contains("Environment=RUST_LOG=info\n"));
    assert!(unit.contains("Restart=on-failure\n"));
    assert!(unit.contains("WantedBy=multi-user.target\n"));
    assert!(!unit.contains("EnvironmentFile="));
    assert!(!unit.contains("KillSignal="));
    insta_like_eq(
        &unit,
        "\
[Unit]
Description=voice-cli speech-to-text service
After=network.target

[Service]
Type=simple
User=swufe
Group=swufe
WorkingDirectory=/opt/voice-cli
ExecStart=/opt/voice-cli/voice-cli server run --config /opt/voice-cli/config.yml
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
",
    );
}

#[test]
fn render_document_parser_snapshot() {
    let unit = render_unit(&document_parser_spec()).unwrap();
    insta_like_eq(
        &unit,
        "\
[Unit]
Description=Document Parser Service (MCP document-parser)
After=network.target

[Service]
Type=simple
User=swufe
Group=swufe
WorkingDirectory=/opt/document-parser
EnvironmentFile=/opt/document-parser/.document-parser.env
ExecStart=/opt/document-parser/document-parser server
Restart=on-failure
RestartSec=5
KillSignal=SIGINT
TimeoutStopSec=60
SyslogIdentifier=document-parser
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
",
    );
}

#[test]
fn reject_path_traversal_unit_name() {
    let mut spec = voice_cli_spec();
    spec.name = "../evil".into();
    assert!(render_unit(&spec).is_err());
}

#[test]
fn render_with_drop_in() {
    let mut spec = voice_cli_spec();
    spec.drop_ins.push(DropIn {
        name: "cuda-sherpa".into(),
        content: "[Service]\nEnvironment=LD_LIBRARY_PATH=/opt/voice-cli:/usr/local/cuda/lib64\n"
            .into(),
    });
    let unit = render_unit(&spec).unwrap();
    // drop-ins are separate files; unit itself unchanged for LD_LIBRARY_PATH
    assert!(!unit.contains("LD_LIBRARY_PATH"));
    assert_eq!(spec.drop_ins.len(), 1);
}

#[test]
fn sanitize_rejects_injection() {
    assert!(sanitize_path(std::path::Path::new("/tmp/a\nb")).is_err());
    assert!(sanitize_path(std::path::Path::new("/tmp/%n")).is_err());
    assert!(matches!(
        shell_join(&[]),
        Err(InstallerError::EmptyExecStart)
    ));
}

fn insta_like_eq(actual: &str, expected: &str) {
    let a = actual.trim_end();
    let e = expected.trim_end();
    assert_eq!(a, e, "\n=== actual ===\n{a}\n=== expected ===\n{e}\n");
}
