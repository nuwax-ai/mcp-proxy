use std::path::PathBuf;

/// Runtime user / group for the systemd unit.
#[derive(Debug, Clone)]
pub struct ServiceIdentity {
    pub user: String,
    pub group: String,
}

/// Optional drop-in override file under `<name>.service.d/`.
#[derive(Debug, Clone)]
pub struct DropIn {
    /// Drop-in basename without `.conf`, e.g. `cuda-sherpa`.
    pub name: String,
    /// Full drop-in file content (already rendered).
    pub content: String,
}

/// Service-agnostic description of a managed service unit to install.
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    /// Unit basename, e.g. `voice-cli` → `voice-cli.service`.
    pub name: String,
    /// `[Unit] Description=`
    pub description: String,
    pub identity: ServiceIdentity,
    /// WorkingDirectory and usual binary location.
    pub install_dir: PathBuf,
    /// Full ExecStart argv (binary absolute path preferred as first element).
    pub exec_start: Vec<String>,
    /// Optional `EnvironmentFile=`.
    pub env_file: Option<PathBuf>,
    /// Extra `Environment=KEY=VALUE` lines.
    pub extra_env: Vec<(String, String)>,
    /// Optional `KillSignal=`.
    pub kill_signal: Option<String>,
    /// Optional `TimeoutStopSec=`.
    pub timeout_stop_sec: Option<u64>,
    /// Optional `SyslogIdentifier=`.
    pub syslog_identifier: Option<String>,
    pub drop_ins: Vec<DropIn>,
    /// Optional `SupplementaryGroups=`.
    pub supplementary_groups: Vec<String>,
    /// Paths that must exist before install (e.g. config.yml). Caller ensures creation.
    pub required_paths: Vec<PathBuf>,
    /// Listen port for conflict detection. Caller parses YAML.
    pub listen_port: Option<u16>,
}

impl ServiceSpec {
    /// Absolute path of the unit file under `/etc/systemd/system/`.
    pub fn unit_path(&self) -> PathBuf {
        PathBuf::from(format!("/etc/systemd/system/{}.service", self.name))
    }

    /// Drop-in directory path (systemd).
    pub fn drop_in_dir(&self) -> PathBuf {
        PathBuf::from(format!("/etc/systemd/system/{}.service.d", self.name))
    }

    /// launchd label, e.g. `com.nuwax.document-parser`.
    pub fn launchd_label(&self) -> String {
        format!("com.nuwax.{}", self.name)
    }

    /// User LaunchAgent plist path on macOS.
    pub fn launchd_plist_path(&self) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", self.launchd_label()))
    }

    /// Wrapper script used by launchd to load `.env` before starting the service.
    pub fn run_server_script_path(&self) -> PathBuf {
        self.install_dir.join("run-server.sh")
    }
}
