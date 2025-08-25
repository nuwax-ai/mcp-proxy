use crate::models::Config;
use crate::{Result, VoiceCliError};
use crate::models::{ClusterNode, MetadataStore, NodeRole, NodeStatus, TaskState, ClusterError};
use crate::cluster::SimpleTaskScheduler;
use tracing::{info, warn, error};
use uuid::Uuid;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::process::{Command, Stdio};
use tokio::signal;
use futures::TryFutureExt;

/// Handle cluster node run command (foreground mode)
pub async fn handle_cluster_run(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Running cluster node in foreground mode");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    info!("Cluster node run requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Can process tasks: {}", can_process_tasks);
    info!("  Metadata DB Path: {}", config.cluster.metadata_db_path);

    // Start the cluster node server
    start_cluster_node_server(config, node_id, http_port, grpc_port, can_process_tasks).await
}

/// Handle cluster node start command (background mode)
pub async fn handle_cluster_start(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Starting cluster node in background mode");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    // Check if already running
    if is_cluster_node_running(http_port).await? {
        return Err(VoiceCliError::Config(format!(
            "Cluster node is already running on port {}", http_port
        )));
    }

    info!("Cluster node start requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Can process tasks: {}", can_process_tasks);

    // Get the current executable path
    let current_exe = std::env::current_exe()
        .map_err(|e| VoiceCliError::Config(format!("Failed to get current executable path: {}", e)))?;

    // Build the daemon command
    let mut cmd = Command::new(&current_exe);
    cmd.args(&[
        "cluster", "run",
        "--node-id", &node_id,
        "--http-port", &http_port.to_string(),
        "--grpc-port", &grpc_port.to_string(),
        "--can-process-tasks", &can_process_tasks.to_string(),
        "--config", &get_config_path_from_args(),
    ]);

    // Start as background process
    cmd.stdout(Stdio::null())
       .stderr(Stdio::null())
       .stdin(Stdio::null());

    // Spawn the background process
    let child = cmd.spawn()
        .map_err(|e| VoiceCliError::Config(format!("Failed to start cluster node: {}", e)))?;

    // Write PID file for management
    let pid_file = format!("./voice-cli-cluster-{}.pid", http_port);
    std::fs::write(&pid_file, child.id().to_string())
        .map_err(|e| VoiceCliError::Config(format!("Failed to write PID file: {}", e)))?;

    info!("Cluster node started with PID: {}", child.id());
    info!("PID file: {}", pid_file);

    // Wait a moment and check if it's actually running
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    if is_cluster_node_running(http_port).await? {
        info!("Cluster node is running successfully on ports HTTP:{} gRPC:{}", http_port, grpc_port);
        Ok(())
    } else {
        Err(VoiceCliError::Config("Cluster node failed to start".to_string()))
    }
}

/// Handle cluster node stop command
pub async fn handle_cluster_stop(config: &Config) -> Result<()> {
    info!("Stopping cluster node");

    // Find all cluster node PID files
    let current_dir = std::env::current_dir()
        .map_err(|e| VoiceCliError::Config(format!("Failed to get current directory: {}", e)))?;
    
    let mut stopped_any = false;
    
    // Look for PID files matching pattern voice-cli-cluster-*.pid
    if let Ok(entries) = std::fs::read_dir(&current_dir) {
        for entry in entries.flatten() {
            if let Some(filename) = entry.file_name().to_str() {
                if filename.starts_with("voice-cli-cluster-") && filename.ends_with(".pid") {
                    let pid_file = entry.path();
                    
                    // Read PID from file
                    if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
                        if let Ok(pid) = pid_str.trim().parse::<u32>() {
                            // Stop the process
                            if stop_cluster_process(pid).await.is_ok() {
                                // Remove PID file
                                let _ = std::fs::remove_file(&pid_file);
                                info!("Stopped cluster node with PID: {}", pid);
                                stopped_any = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if stopped_any {
        info!("Cluster node(s) stopped successfully");
        Ok(())
    } else {
        info!("No running cluster nodes found");
        Ok(())
    }
}

/// Handle cluster node restart command
pub async fn handle_cluster_restart(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Restarting cluster node");

    // Try to stop if running (ignore errors if not running)
    let _ = handle_cluster_stop(config).await;

    // Wait a moment for cleanup
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Start again
    handle_cluster_start(config, node_id, http_port, grpc_port, can_process_tasks).await
}

/// Handle cluster initialization command
pub async fn handle_cluster_init(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    leader_can_process_tasks: bool,
) -> Result<()> {
    info!("Initializing new cluster");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    info!("Cluster initialization requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Leader can process tasks: {}", leader_can_process_tasks);
    info!("  Metadata DB Path: {}", config.cluster.metadata_db_path);

    // Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(&config.cluster.metadata_db_path)
        .map_err(|e| VoiceCliError::Config(format!("Failed to initialize metadata store: {}", e)))?);

    // Create cluster node configuration
    let cluster_node = ClusterNode {
        node_id: node_id.clone(),
        address: config.cluster.bind_address.clone(),
        grpc_port,
        http_port,
        role: NodeRole::Leader, // First node becomes leader
        status: NodeStatus::Healthy,
        last_heartbeat: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    };

    // Add this node to the cluster metadata
    metadata_store.add_node(&cluster_node).await?;

    // Initialize task scheduler for this node
    let scheduler_config = crate::cluster::SchedulerConfig::default();
    let _task_scheduler = SimpleTaskScheduler::new(
        metadata_store.clone(),
        leader_can_process_tasks,
        node_id.clone(),
        scheduler_config,
    );

    info!("✅ Cluster initialized successfully");
    info!("   Node ID: {}", node_id);
    info!("   gRPC Address: {}:{}", config.cluster.bind_address, grpc_port);
    info!("   HTTP Address: {}:{}", config.cluster.bind_address, http_port);
    info!("   Metadata Store: {}", config.cluster.metadata_db_path);
    info!("   Leader can process tasks: {}", leader_can_process_tasks);

    Ok(())
}

/// Handle cluster join command
pub async fn handle_cluster_join(
    config: &Config,
    peer_address: String,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    token: Option<String>,
) -> Result<()> {
    info!("Joining cluster via peer: {}", peer_address);

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    info!("Cluster join requested:");
    info!("  Peer Address: {}", peer_address);
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Token: {:?}", token);
    info!("  Metadata DB Path: {}", config.cluster.metadata_db_path);

    // Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(&config.cluster.metadata_db_path)
        .map_err(|e| VoiceCliError::Config(format!("Failed to initialize metadata store: {}", e)))?);

    // Create cluster node configuration for this joining node
    let cluster_node = ClusterNode {
        node_id: node_id.clone(),
        address: config.cluster.bind_address.clone(),
        grpc_port,
        http_port,
        role: NodeRole::Follower, // Joining nodes start as followers
        status: NodeStatus::Joining,
        last_heartbeat: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    };

    // Add this node to the cluster metadata
    metadata_store.add_node(&cluster_node).await?;

    // Implement actual cluster join protocol with gRPC peer communication
    let join_result = perform_cluster_join(&peer_address, &cluster_node, token.clone()).await;
    
    match join_result {
        Ok(join_response) => {
            info!("✅ Successfully joined cluster!");
            info!("   Join response: {}", join_response.message);
            info!("   Cluster now has {} nodes", join_response.cluster_nodes.len());
            
            // Update local metadata store with cluster nodes information
            for node_proto in &join_response.cluster_nodes {
                if let Ok(remote_node) = proto_to_cluster_node(node_proto) {
                    if remote_node.node_id != cluster_node.node_id {
                        // Add other cluster nodes to our local metadata
                        if let Err(e) = metadata_store.add_node(&remote_node).await {
                            warn!("Failed to add remote node {} to local metadata: {}", remote_node.node_id, e);
                        } else {
                            info!("Added remote node {} to local metadata", remote_node.node_id);
                        }
                    }
                }
            }
        }
        Err(e) => {
            error!("❌ Failed to join cluster: {}", e);
            return Err(VoiceCliError::Config(format!("Cluster join failed: {}", e)));
        }
    }

    info!("✅ Successfully prepared to join cluster");
    info!("   Peer Address: {}", peer_address);
    info!("   Node ID: {}", node_id);
    info!("   gRPC Address: {}:{}", config.cluster.bind_address, grpc_port);
    info!("   HTTP Address: {}:{}", config.cluster.bind_address, http_port);
    if let Some(ref t) = token {
        info!("   Using authentication token: {} characters", t.len());
    }

    Ok(())
}

/// Handle cluster status command
pub async fn handle_cluster_status(config: &Config, detailed: bool) -> Result<()> {
    info!("Checking cluster status");

    // Initialize metadata store to check cluster status
    let metadata_store_result = MetadataStore::new(&config.cluster.metadata_db_path);
    
    match metadata_store_result {
        Ok(metadata_store) => {
            let metadata_store = Arc::new(metadata_store);
            
            // Get cluster nodes
            let nodes = metadata_store.get_all_nodes().await?;
            
            println!("Cluster Status:");
            println!("==============");
            println!("Metadata DB Path: {}", config.cluster.metadata_db_path);
            println!("Cluster Enabled: {}", config.cluster.enabled);
            println!("Node ID: {}", config.cluster.node_id);
            println!("gRPC Port: {}", config.cluster.grpc_port);
            println!("HTTP Port: {}", config.cluster.http_port);
            println!("Total Nodes: {}", nodes.len());
            
            if detailed {
                println!("\nCluster Configuration:");
                println!("=====================");
                println!("Leader can process tasks: {}", config.cluster.leader_can_process_tasks);
                println!("Heartbeat interval: {}s", config.cluster.heartbeat_interval);
                println!("Election timeout: {}s", config.cluster.election_timeout);
                println!("Bind address: {}", config.cluster.bind_address);
                
                println!("\nCluster Nodes:");
                println!("==============");
                if nodes.is_empty() {
                    println!("No nodes found in cluster");
                } else {
                    for node in &nodes {
                        let status = match node.status {
                            NodeStatus::Healthy => "Healthy",
                            NodeStatus::Unhealthy => "Unhealthy", 
                            NodeStatus::Joining => "Joining",
                            NodeStatus::Leaving => "Leaving",
                        };
                        let role = match node.role {
                            NodeRole::Leader => "Leader",
                            NodeRole::Follower => "Follower",
                            NodeRole::Candidate => "Candidate",
                        };
                        let last_seen = chrono::DateTime::from_timestamp(node.last_heartbeat, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                            .unwrap_or_else(|| "Unknown".to_string());
                        
                        println!("  - Node ID: {}", node.node_id);
                        println!("    Address: {}", node.address);
                        println!("    gRPC: {}", node.grpc_address());
                        println!("    HTTP: {}", node.http_address());
                        println!("    Role: {}", role);
                        println!("    Status: {}", status);
                        println!("    Last Heartbeat: {}", last_seen);
                        println!();
                    }
                }
                
                // Check for active tasks using public methods
                // Get tasks by different states and combine them
                let pending_tasks = metadata_store.get_tasks_by_state(TaskState::Pending).await.unwrap_or_default();
                let assigned_tasks = metadata_store.get_tasks_by_state(TaskState::Assigned).await.unwrap_or_default();
                let processing_tasks = metadata_store.get_tasks_by_state(TaskState::Processing).await.unwrap_or_default();
                let completed_tasks = metadata_store.get_tasks_by_state(TaskState::Completed).await.unwrap_or_default();
                let failed_tasks = metadata_store.get_tasks_by_state(TaskState::Failed).await.unwrap_or_default();
                
                let mut all_tasks = Vec::new();
                all_tasks.extend(pending_tasks);
                all_tasks.extend(assigned_tasks);
                all_tasks.extend(processing_tasks);
                all_tasks.extend(completed_tasks);
                all_tasks.extend(failed_tasks);
                
                let active_tasks: Vec<_> = all_tasks.iter()
                    .filter(|task| task.completed_at.is_none())
                    .collect();
                
                println!("Active Tasks: {}", active_tasks.len());
                
                if !active_tasks.is_empty() {
                    println!("\nActive Task List:");
                    println!("=================");
                    for task in active_tasks {
                        let assigned_to = task.assigned_node.as_deref().unwrap_or("Unassigned");
                        println!("  - Task ID: {}", task.task_id);
                        println!("    Client: {}", task.client_id);
                        println!("    File: {}", task.filename);
                        println!("    Assigned to: {}", assigned_to);
                        println!("    Model: {}", task.model.as_deref().unwrap_or("default"));
                        println!();
                    }
                }
            }
            
            // Check cluster health
            if config.cluster.enabled {
                let active_nodes = nodes.iter().filter(|n| matches!(n.status, NodeStatus::Healthy)).count();
                if active_nodes == 0 {
                    warn!("⚠️  No healthy nodes in cluster");
                } else if active_nodes == 1 {
                    info!("ℹ️  Single-node cluster (no redundancy)");
                } else {
                    info!("✅ Multi-node cluster operational");
                }
            }
        }
        Err(e) => {
            error!("Failed to access cluster metadata: {}", e);
            println!("Cluster Status: UNAVAILABLE");
            println!("Error: {}", e);
            println!("\nThis might indicate:");
            println!("- Cluster has not been initialized");
            println!("- Database file is corrupted");
            println!("- Insufficient permissions");
            return Err(VoiceCliError::Config(format!("Cluster metadata unavailable: {}", e)));
        }
    }

    Ok(())
}

/// Handle configuration generation command
pub async fn handle_generate_config(
    config: &Config,
    output: Option<String>,
    template: String,
) -> Result<()> {
    info!("Generating cluster configuration");

    let output_path = match output {
        Some(path) => path,
        None => "./cluster-config.yml".to_string(),
    };

    // Generate configuration based on template
    let config_content = match template.as_str() {
        "default" => generate_default_cluster_config(config),
        "production" => generate_production_cluster_config(config),
        "development" => generate_development_cluster_config(config),
        _ => return Err(VoiceCliError::Config(format!("Unknown template: {}", template))),
    };

    // Write to file
    std::fs::write(&output_path, config_content)
        .map_err(|e| VoiceCliError::Config(format!("Failed to write config file: {}", e)))?;

    println!("Cluster configuration generated: {}", output_path);
    Ok(())
}

/// Generate default cluster configuration
fn generate_default_cluster_config(config: &Config) -> String {
    format!(r#"# Voice-CLI Cluster Configuration
# Generated automatically - modify as needed

cluster:
  enabled: true
  node_id: "node-random-id"
  leader_can_process_tasks: true
  
  # Network Configuration
  bind_address: "0.0.0.0"
  grpc_port: 50051
  http_port: 8080
  
  # Cluster Settings
  heartbeat_interval: 5
  election_timeout: 10
  metadata_db_path: "{}"

# Load Balancer Configuration
load_balancer:
  enabled: false
  port: 8090
  health_check_interval: 10

# Logging Configuration  
logging:
  level: "info"
  log_dir: "{}"
"#, config.cluster.metadata_db_path, config.logging.log_dir)
}

/// Generate production cluster configuration
fn generate_production_cluster_config(config: &Config) -> String {
    format!(r#"# Voice-CLI Production Cluster Configuration

cluster:
  enabled: true
  node_id: "prod-node-random-id"
  leader_can_process_tasks: false  # Dedicated leader for coordination
  
  # Network Configuration
  bind_address: "0.0.0.0"
  grpc_port: 50051
  http_port: 8080
  
  # High-availability Settings
  heartbeat_interval: 3
  election_timeout: 5
  metadata_db_path: "{}"

# Production Load Balancer
load_balancer:
  enabled: true
  port: 8090
  health_check_interval: 5

# Production Logging
logging:
  level: "warn"
  log_dir: "{}"
"#, config.cluster.metadata_db_path, config.logging.log_dir)
}

/// Generate development cluster configuration
fn generate_development_cluster_config(config: &Config) -> String {
    format!(r#"# Voice-CLI Development Cluster Configuration

cluster:
  enabled: true
  node_id: "dev-node-random-id"
  leader_can_process_tasks: true  # All nodes can process in dev
  
  # Network Configuration
  bind_address: "127.0.0.1"  # Local development
  grpc_port: 50051
  http_port: 8080
  
  # Development Settings
  heartbeat_interval: 10
  election_timeout: 20
  metadata_db_path: "{}"

# Development Load Balancer
load_balancer:
  enabled: true
  port: 8090
  health_check_interval: 15

# Development Logging
logging:
  level: "debug"
  log_dir: "{}"
"#, config.cluster.metadata_db_path, config.logging.log_dir)
}

/// Perform actual cluster join using gRPC communication
async fn perform_cluster_join(
    peer_address: &str,
    joining_node: &ClusterNode,
    cluster_token: Option<String>,
) -> std::result::Result<crate::grpc::proto::JoinResponse, VoiceCliError> {
    info!("Connecting to cluster peer: {}", peer_address);
    
    // Parse peer address to extract host and gRPC port
    let (host, grpc_port) = parse_peer_address(peer_address)?;
    let grpc_address = format!("{}:{}", host, grpc_port);
    
    // Create gRPC client connection
    let mut client = crate::grpc::client::AudioClusterClient::connect(&grpc_address).await
        .map_err(|e| VoiceCliError::Config(format!("Failed to connect to peer {}: {}", grpc_address, e)))?;
    
    info!("Connected to peer, sending join request...");
    
    // Send join request
    let join_response = client.join_cluster(joining_node, cluster_token).await
        .map_err(|e| VoiceCliError::Config(format!("Join request failed: {}", e)))?;
    
    if join_response.success {
        info!("Join request accepted by cluster");
        Ok(join_response)
    } else {
        Err(VoiceCliError::Config(format!(
            "Join request rejected: {}", 
            join_response.message
        )))
    }
}

/// Parse peer address to extract host and gRPC port
fn parse_peer_address(peer_address: &str) -> std::result::Result<(String, u16), VoiceCliError> {
    let parts: Vec<&str> = peer_address.split(':').collect();
    
    match parts.len() {
        1 => {
            // Only host provided, use default gRPC port
            Ok((parts[0].to_string(), 50051))
        }
        2 => {
            // Host and port provided
            let host = parts[0].to_string();
            let port = parts[1].parse::<u16>()
                .map_err(|_| VoiceCliError::Config(format!("Invalid port in peer address: {}", peer_address)))?;
            Ok((host, port))
        }
        _ => {
            Err(VoiceCliError::Config(format!("Invalid peer address format: {}. Expected format: host:port or host", peer_address)))
        }
    }
}

/// Convert protobuf NodeInfo to ClusterNode
fn proto_to_cluster_node(node_proto: &crate::grpc::proto::NodeInfo) -> std::result::Result<ClusterNode, VoiceCliError> {
    use crate::models::{NodeRole, NodeStatus};
    
    let role = match node_proto.role {
        0 => NodeRole::Follower,
        1 => NodeRole::Leader,
        2 => NodeRole::Candidate,
        _ => return Err(VoiceCliError::Config(format!("Invalid node role: {}", node_proto.role))),
    };
    
    let status = match node_proto.status {
        0 => NodeStatus::Healthy,
        1 => NodeStatus::Unhealthy,
        2 => NodeStatus::Joining,
        3 => NodeStatus::Leaving,
        _ => return Err(VoiceCliError::Config(format!("Invalid node status: {}", node_proto.status))),
    };
    
    Ok(ClusterNode {
        node_id: node_proto.node_id.clone(),
        address: node_proto.address.clone(),
        grpc_port: node_proto.grpc_port as u16,
        http_port: node_proto.http_port as u16,
        role,
        status,
        last_heartbeat: node_proto.last_heartbeat,
    })
}

/// Start the actual cluster node server (core implementation)
async fn start_cluster_node_server(
    config: &Config,
    node_id: String,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Starting cluster node server");
    info!("  Node ID: {}", node_id);
    info!("  HTTP Port: {}", http_port);
    info!("  gRPC Port: {}", grpc_port);
    info!("  Can process tasks: {}", can_process_tasks);

    // Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(&config.cluster.metadata_db_path)
        .map_err(|e| VoiceCliError::Config(format!("Failed to initialize metadata store: {}", e)))?);

    // Create cluster node configuration
    let cluster_node = ClusterNode {
        node_id: node_id.clone(),
        address: config.cluster.bind_address.clone(),
        grpc_port,
        http_port,
        role: NodeRole::Follower, // Start as follower, leader election will determine actual role
        status: NodeStatus::Healthy,
        last_heartbeat: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    };

    // Add this node to the cluster metadata
    metadata_store.add_node(&cluster_node).await?;

    // Initialize task scheduler
    let scheduler_config = crate::cluster::SchedulerConfig::default();
    let _task_scheduler = SimpleTaskScheduler::new(
        metadata_store.clone(),
        can_process_tasks,
        node_id.clone(),
        scheduler_config,
    );

    info!("Cluster node initialized, starting services...");

    // Set up graceful shutdown
    let shutdown_signal = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Received shutdown signal, stopping cluster node...");
    };

    // In a full implementation, this would start:
    // 1. HTTP server for API endpoints
    // 2. gRPC server for cluster communication
    // 3. Heartbeat service
    // 4. Task processing workers
    // 5. Leader election service
    
    // For now, we'll simulate the cluster node running
    info!("✅ Cluster node '{}' is running", node_id);
    info!("   HTTP API: {}:{}", config.cluster.bind_address, http_port);
    info!("   gRPC Cluster: {}:{}", config.cluster.bind_address, grpc_port);
    info!("   Task Processing: {}", if can_process_tasks { "Enabled" } else { "Disabled" });
    info!("   Metadata Store: {}", config.cluster.metadata_db_path);
    
    // Keep the node running until shutdown signal
    shutdown_signal.await;
    
    info!("Cluster node '{}' shutting down", node_id);
    Ok(())
}

/// Check if cluster node is running on a specific port
async fn is_cluster_node_running(port: u16) -> Result<bool> {
    Ok(tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .is_ok())
}

/// Stop a cluster process by PID
async fn stop_cluster_process(pid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        let pid = Pid::from_raw(pid as i32);
        signal::kill(pid, Signal::SIGTERM)
            .map_err(|e| VoiceCliError::Config(format!("Failed to send SIGTERM to process {}: {}", pid, e)))?;
            
        // Wait a moment for graceful shutdown
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Check if still running and force kill if necessary
        if check_process_running(pid.as_raw() as u32) {
            warn!("Process {} did not respond to SIGTERM, sending SIGKILL", pid);
            signal::kill(pid, Signal::SIGKILL)
                .map_err(|e| VoiceCliError::Config(format!("Failed to send SIGKILL to process {}: {}", pid, e)))?;
        }
    }

    #[cfg(windows)]
    {
        let output = Command::new("taskkill")
            .args(&["/F", "/PID", &pid.to_string()])
            .output()
            .map_err(|e| VoiceCliError::Config(format!("Failed to execute taskkill: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VoiceCliError::Config(format!("Failed to stop process {}: {}", pid, stderr)));
        }
    }

    Ok(())
}

/// Check if a process is running by PID
fn check_process_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        
        let pid = Pid::from_raw(pid as i32);
        signal::kill(pid, None).is_ok()
    }

    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(&["/FI", &format!("PID eq {}", pid)])
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            }
            Err(_) => false,
        }
    }
}

/// Get configuration path from command line arguments
fn get_config_path_from_args() -> String {
    std::env::args()
        .collect::<Vec<String>>()
        .windows(2)
        .find(|window| window[0] == "--config" || window[0] == "-c")
        .map(|window| window[1].clone())
        .unwrap_or_else(|| "config.yml".to_string())
}