pub mod server;
pub mod model;
pub mod cluster;
pub mod lb;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "voice-cli")]
#[command(about = "Speech-to-text HTTP service with CLI interface")]
#[command(version = "0.1.0")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Configuration file path
    #[arg(short, long, default_value = "config.yml")]
    pub config: String,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Server management commands
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// Model management commands
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Cluster management commands
    Cluster {
        #[command(subcommand)]
        action: ClusterAction,
    },
    /// Load balancer management commands
    Lb {
        #[command(subcommand)]
        action: LoadBalancerAction,
    },
    /// Internal daemon command (used by daemon service)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Subcommand)]
pub enum ServerAction {
    /// Run server in foreground mode
    Run,
    /// Start server in background mode (daemon)
    Start,
    /// Stop background server
    Stop,
    /// Restart background server
    Restart,
    /// Check server status
    Status,
}

#[derive(Subcommand)]
pub enum ModelAction {
    /// Download a specific model
    Download {
        /// Model name to download (e.g., base, small, large)
        model_name: String,
    },
    /// List available and downloaded models
    List,
    /// Validate downloaded models
    Validate,
    /// Remove a downloaded model
    Remove {
        /// Model name to remove
        model_name: String,
    },
}

#[derive(Subcommand)]
pub enum ClusterAction {
    /// Run cluster node in foreground mode
    Run {
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Whether this node can process tasks
        #[arg(long, default_value = "true")]
        can_process_tasks: bool,
    },
    /// Start cluster node in background mode
    Start {
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Whether this node can process tasks
        #[arg(long, default_value = "true")]
        can_process_tasks: bool,
    },
    /// Stop cluster node
    Stop,
    /// Restart cluster node
    Restart {
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Whether this node can process tasks
        #[arg(long, default_value = "true")]
        can_process_tasks: bool,
    },
    /// Initialize a new cluster
    Init {
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Whether this node can process tasks (leader configuration)
        #[arg(long, default_value = "true")]
        leader_can_process_tasks: bool,
    },
    /// Join an existing cluster
    Join {
        /// Address of a node in the target cluster
        #[arg(long)]
        peer_address: String,
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Cluster token for authentication (optional)
        #[arg(long)]
        token: Option<String>,
    },
    /// Get cluster status
    Status {
        /// Show detailed node information
        #[arg(long)]
        detailed: bool,
    },
    /// Generate cluster configuration
    GenerateConfig {
        /// Output file path (optional, defaults to current directory)
        #[arg(long, short)]
        output: Option<String>,
        /// Configuration template type
        #[arg(long, default_value = "default")]
        template: String,
    },
    /// Install systemd service for cluster node
    InstallService {
        /// Service name (defaults to voice-cli-cluster)
        #[arg(long, default_value = "voice-cli-cluster")]
        service_name: String,
        /// Node ID for this cluster node
        #[arg(long)]
        node_id: Option<String>,
        /// HTTP port for this node
        #[arg(long, default_value = "8080")]
        http_port: u16,
        /// gRPC port for this node
        #[arg(long, default_value = "50051")]
        grpc_port: u16,
        /// Whether this node can process tasks
        #[arg(long, default_value = "true")]
        can_process_tasks: bool,
        /// Memory limit for the service (e.g., 1G, 512M)
        #[arg(long)]
        memory_limit: Option<String>,
        /// CPU limit for the service (e.g., 2, 0.5)
        #[arg(long)]
        cpu_limit: Option<String>,
        /// User to run the service as (defaults to current user)
        #[arg(long)]
        user: Option<String>,
        /// Group to run the service as (defaults to current user's group)
        #[arg(long)]
        group: Option<String>,
    },
    /// Uninstall systemd service
    UninstallService {
        /// Service name to uninstall
        #[arg(long, default_value = "voice-cli-cluster")]
        service_name: String,
    },
    /// Check systemd service status
    ServiceStatus {
        /// Service name to check
        #[arg(long, default_value = "voice-cli-cluster")]
        service_name: String,
    },
}

#[derive(Subcommand)]
pub enum LoadBalancerAction {
    /// Run load balancer in foreground mode
    Run {
        /// Load balancer port
        #[arg(long, default_value = "8090")]
        port: u16,
        /// Health check interval in seconds
        #[arg(long, default_value = "10")]
        health_check_interval: u64,
    },
    /// Start load balancer in background mode
    Start {
        /// Load balancer port
        #[arg(long, default_value = "8090")]
        port: u16,
    },
    /// Stop load balancer
    Stop,
    /// Restart load balancer
    Restart {
        /// Load balancer port
        #[arg(long, default_value = "8090")]
        port: u16,
    },
    /// Check load balancer status
    Status,
}

#[derive(Subcommand)]
pub enum DaemonAction {
    /// Serve HTTP requests (internal command used by daemon)
    Serve,
}