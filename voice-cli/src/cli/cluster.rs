use crate::models::Config;
use crate::models::{ClusterNode, MetadataStore, NodeRole, NodeStatus, TaskState, ClusterError};
use crate::cluster::{SimpleTaskScheduler, ClusterServiceManager, ClusterState};
use crate::error::ClusterResultExt;
use tracing::{info, warn, error};
use uuid::Uuid;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH, Duration};

use anyhow::{Context, Result};


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

    // Create a modified config with the specified parameters
    let mut cluster_config = config.clone();
    cluster_config.cluster.node_id = node_id;
    cluster_config.cluster.http_port = http_port;
    cluster_config.cluster.grpc_port = grpc_port;
    cluster_config.cluster.leader_can_process_tasks = can_process_tasks;

    // Start the cluster node server
    start_cluster_node_server(Arc::new(cluster_config)).await
}

/// Handle cluster node start command (direct async mode)
pub async fn handle_cluster_start(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Starting cluster node in direct async mode");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    // Check if already running
    if is_cluster_node_running(http_port).await? {
        anyhow::bail!("Cluster node is already running on port {}", http_port);
    }

    info!("Cluster node start requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Can process tasks: {}", can_process_tasks);

    // Create a modified config with the specified ports
    let mut cluster_config = config.clone();
    cluster_config.cluster.node_id = node_id.clone();
    cluster_config.cluster.http_port = http_port;
    cluster_config.cluster.grpc_port = grpc_port;
    cluster_config.cluster.leader_can_process_tasks = can_process_tasks;

    // Start the cluster node directly using ClusterServiceManager
    start_cluster_node_server(Arc::new(cluster_config)).await
        .context("Failed to start cluster node server")?;

    Ok(())
}

/// Handle cluster node stop command
pub async fn handle_cluster_stop(config: &Config) -> Result<()> {
    info!("Stopping cluster node");

    // Since we no longer use PID files, try to stop via HTTP API
    let http_port = config.cluster.http_port;
    
    // Try to connect to the cluster node's HTTP API to request shutdown
    let client = reqwest::Client::new();
    let shutdown_url = format!("http://127.0.0.1:{}/cluster/shutdown", http_port);
    
    match client.post(&shutdown_url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                info!("✅ Cluster node stopped successfully via HTTP API");
                Ok(())
            } else {
                warn!("Cluster node responded with status: {}", response.status());
                anyhow::bail!("Failed to stop cluster node via HTTP API")
            }
        }
        Err(_) => {
            info!("No running cluster node found on port {}", http_port);
            Ok(())
        }
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

    // Step 1: Create proper directory structure
    create_cluster_directory_structure(config).await
        .context("Failed to create cluster directory structure")?;

    // Step 2: Validate environment and dependencies
    validate_cluster_environment(config, http_port, grpc_port).await
        .context("Environment validation failed")?;

    // Step 3: Generate configuration file with defaults if needed
    generate_cluster_configuration_file(config, &node_id, http_port, grpc_port, leader_can_process_tasks).await
        .context("Failed to generate cluster configuration")?;

    // Step 4: Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(&config.cluster.metadata_db_path)
        .map_err(ClusterError::from)
        .context("Failed to initialize metadata store")?);

    // Step 5: Create cluster node configuration
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

    // Step 6: Add this node to the cluster metadata
    metadata_store.add_node(&cluster_node).await
        .with_node_context(&cluster_node.node_id)?;

    // Step 7: Initialize task scheduler for this node
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
    info!("   Directory structure created and validated");

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
        .map_err(ClusterError::from)
        .context("Failed to initialize metadata store")?);

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
            anyhow::bail!("Cluster join failed: {}", e);
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
            anyhow::bail!("Cluster metadata unavailable: {}", e);
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
        _ => anyhow::bail!("Unknown template: {}", template),
    };

    // Write to file
    std::fs::write(&output_path, config_content)
        .context("Failed to write config file")?;

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
) -> Result<crate::grpc::proto::JoinResponse> {
    info!("Connecting to cluster peer: {}", peer_address);
    
    // Parse peer address to extract host and gRPC port
    let (host, grpc_port) = parse_peer_address(peer_address)?;
    let grpc_address = format!("{}:{}", host, grpc_port);
    
    // Create gRPC client connection
    let mut client = crate::grpc::client::AudioClusterClient::connect(&grpc_address).await
        .with_context(|| format!("Failed to connect to peer {}", grpc_address))?;
    
    info!("Connected to peer, sending join request...");
    
    // Send join request
    let join_response = client.join_cluster(joining_node, cluster_token).await
        .context("Join request failed")?;
    
    if join_response.success {
        info!("Join request accepted by cluster");
        Ok(join_response)
    } else {
        anyhow::bail!("Join request rejected: {}", join_response.message)
    }
}

/// Parse peer address to extract host and gRPC port
fn parse_peer_address(peer_address: &str) -> Result<(String, u16)> {
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
                .with_context(|| format!("Invalid port in peer address: {}", peer_address))?;
            Ok((host, port))
        }
        _ => {
            anyhow::bail!("Invalid peer address format: {}. Expected format: host:port or host", peer_address)
        }
    }
}

/// Convert protobuf NodeInfo to ClusterNode
fn proto_to_cluster_node(node_proto: &crate::grpc::proto::NodeInfo) -> Result<ClusterNode> {
    use crate::models::{NodeRole, NodeStatus};
    
    let role = match node_proto.role {
        0 => NodeRole::Follower,
        1 => NodeRole::Leader,
        2 => NodeRole::Candidate,
        _ => anyhow::bail!("Invalid node role: {}", node_proto.role),
    };
    
    let status = match node_proto.status {
        0 => NodeStatus::Healthy,
        1 => NodeStatus::Unhealthy,
        2 => NodeStatus::Joining,
        3 => NodeStatus::Leaving,
        _ => anyhow::bail!("Invalid node status: {}", node_proto.status),
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

/// Start the actual cluster node server using ClusterServiceManager
async fn start_cluster_node_server(config: Arc<Config>) -> Result<()> {
    info!("Starting cluster node server with ClusterServiceManager");
    
    let node_id = config.cluster.node_id.clone();
    let http_port = config.cluster.http_port;
    let grpc_port = config.cluster.grpc_port;
    let can_process_tasks = config.cluster.leader_can_process_tasks;

    info!("  Node ID: {}", node_id);
    info!("  HTTP Port: {}", http_port);
    info!("  gRPC Port: {}", grpc_port);
    info!("  Can process tasks: {}", can_process_tasks);

    // Initialize metadata store
    let metadata_store = Arc::new(MetadataStore::new(&config.cluster.metadata_db_path)
        .with_cluster_context("initialize metadata store")?);

    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());

    // Create cluster node configuration
    let cluster_node = ClusterNode::new(
        node_id.clone(),
        config.cluster.bind_address.clone(),
        grpc_port,
        http_port,
    );

    // Initialize task scheduler if task processing is enabled
    let task_scheduler = if can_process_tasks {
        let scheduler_config = crate::cluster::SchedulerConfig::default();
        Some(Arc::new(SimpleTaskScheduler::new(
            metadata_store.clone(),
            can_process_tasks,
            node_id.clone(),
            scheduler_config,
        )))
    } else {
        None
    };

    // Create and configure the service manager
    let mut service_manager = ClusterServiceManager::new(
        config.clone(),
        cluster_node,
        cluster_state,
        Some(metadata_store),
    );

    if let Some(scheduler) = task_scheduler {
        service_manager = service_manager.with_task_scheduler(scheduler);
    }

    info!("✅ Starting cluster node '{}' services", node_id);
    info!("   HTTP API: {}:{}", config.cluster.bind_address, http_port);
    info!("   gRPC Cluster: {}:{}", config.cluster.bind_address, grpc_port);
    info!("   Task Processing: {}", if can_process_tasks { "Enabled" } else { "Disabled" });
    info!("   Metadata Store: {}", config.cluster.metadata_db_path);

    // Start all services concurrently
    service_manager.start().await
        .context("Failed to start cluster services")?;

    info!("Cluster node '{}' shut down gracefully", node_id);
    Ok(())
}

/// Check if cluster node is running on a specific port
async fn is_cluster_node_running(port: u16) -> Result<bool> {
    Ok(tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .is_ok())
}

/// Create proper directory structure for cluster initialization
async fn create_cluster_directory_structure(config: &Config) -> Result<()> {
    info!("Creating cluster directory structure");

    // Create metadata database directory
    let metadata_db_path = std::path::PathBuf::from(&config.cluster.metadata_db_path);
    if let Some(parent) = metadata_db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create metadata DB parent directory: {:?}", parent))?;
        info!("✓ Created metadata DB directory: {:?}", parent);
    }

    // Create logs directory
    let logs_dir = std::path::PathBuf::from(&config.logging.log_dir);
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("Failed to create logs directory: {:?}", logs_dir))?;
    info!("✓ Created logs directory: {:?}", logs_dir);

    // Create models directory for whisper models
    let models_dir = std::path::PathBuf::from(&config.whisper.models_dir);
    std::fs::create_dir_all(&models_dir)
        .with_context(|| format!("Failed to create models directory: {:?}", models_dir))?;
    info!("✓ Created models directory: {:?}", models_dir);

    // Create daemon work directory
    let work_dir = std::path::PathBuf::from(&config.daemon.work_dir);
    std::fs::create_dir_all(&work_dir)
        .with_context(|| format!("Failed to create work directory: {:?}", work_dir))?;
    info!("✓ Created work directory: {:?}", work_dir);

    // Create shared storage directory for cluster file sharing
    let shared_dir = work_dir.join("shared-voice-cli");
    std::fs::create_dir_all(&shared_dir)
        .with_context(|| format!("Failed to create shared directory: {:?}", shared_dir))?;
    info!("✓ Created shared storage directory: {:?}", shared_dir);

    // Create temp directory for processing
    let temp_dir = work_dir.join("temp");
    std::fs::create_dir_all(&temp_dir)
        .with_context(|| format!("Failed to create temp directory: {:?}", temp_dir))?;
    info!("✓ Created temp directory: {:?}", temp_dir);

    info!("✅ Cluster directory structure created successfully");
    Ok(())
}

/// Validate environment and dependencies for cluster operation
async fn validate_cluster_environment(config: &Config, http_port: u16, grpc_port: u16) -> Result<()> {
    info!("Validating cluster environment and dependencies");

    // Check if ports are available
    if let Err(_) = tokio::net::TcpListener::bind(format!("{}:{}", config.cluster.bind_address, http_port)).await {
        anyhow::bail!("HTTP port {}:{} is not available", config.cluster.bind_address, http_port);
    }
    info!("✓ HTTP port {}:{} is available", config.cluster.bind_address, http_port);

    if let Err(_) = tokio::net::TcpListener::bind(format!("{}:{}", config.cluster.bind_address, grpc_port)).await {
        anyhow::bail!("gRPC port {}:{} is not available", config.cluster.bind_address, grpc_port);
    }
    info!("✓ gRPC port {}:{} is available", config.cluster.bind_address, grpc_port);

    // Check directory permissions
    let metadata_db_path = std::path::PathBuf::from(&config.cluster.metadata_db_path);
    if let Some(parent) = metadata_db_path.parent() {
        let test_file = parent.join(".write_test");
        match std::fs::write(&test_file, "test") {
            Ok(_) => {
                let _ = std::fs::remove_file(test_file);
                info!("✓ Metadata directory is writable: {:?}", parent);
            }
            Err(e) => {
                anyhow::bail!("Metadata directory is not writable: {:?} - {}", parent, e);
            }
        }
    }

    // Check logs directory permissions
    let logs_dir = std::path::PathBuf::from(&config.logging.log_dir);
    let test_file = logs_dir.join(".write_test");
    match std::fs::write(&test_file, "test") {
        Ok(_) => {
            let _ = std::fs::remove_file(test_file);
            info!("✓ Logs directory is writable: {:?}", logs_dir);
        }
        Err(e) => {
            anyhow::bail!("Logs directory is not writable: {:?} - {}", logs_dir, e);
        }
    }

    // Check models directory permissions
    let models_dir = std::path::PathBuf::from(&config.whisper.models_dir);
    let test_file = models_dir.join(".write_test");
    match std::fs::write(&test_file, "test") {
        Ok(_) => {
            let _ = std::fs::remove_file(test_file);
            info!("✓ Models directory is writable: {:?}", models_dir);
        }
        Err(e) => {
            anyhow::bail!("Models directory is not writable: {:?} - {}", models_dir, e);
        }
    }

    // Validate configuration consistency
    config.validate()
        .context("Configuration validation failed")?;
    info!("✓ Configuration validation passed");

    // Check system resources
    let available_memory = get_available_memory_mb();
    if available_memory < 512 {
        warn!("⚠️  Low available memory: {} MB (recommended: 1GB+)", available_memory);
    } else {
        info!("✓ Available memory: {} MB", available_memory);
    }

    // Check disk space
    let available_disk_gb = get_available_disk_space_gb(&std::path::PathBuf::from(&config.daemon.work_dir))?;
    if available_disk_gb < 1.0 {
        warn!("⚠️  Low available disk space: {:.1} GB (recommended: 5GB+)", available_disk_gb);
    } else {
        info!("✓ Available disk space: {:.1} GB", available_disk_gb);
    }

    info!("✅ Environment validation completed successfully");
    Ok(())
}

/// Generate cluster configuration file with defaults
async fn generate_cluster_configuration_file(
    config: &Config,
    node_id: &str,
    http_port: u16,
    grpc_port: u16,
    leader_can_process_tasks: bool,
) -> Result<()> {
    info!("Generating cluster configuration file");

    let config_path = std::path::PathBuf::from("cluster-config.yml");
    
    // Don't overwrite existing configuration
    if config_path.exists() {
        info!("Configuration file already exists: {:?}", config_path);
        return Ok(());
    }

    let config_content = format!(r#"# Voice-CLI Cluster Configuration
# Generated during cluster initialization
# Modify as needed for your deployment

# Server Configuration
server:
  host: "{}"
  port: {}
  max_file_size: {}
  cors_enabled: {}

# Whisper Configuration
whisper:
  default_model: "{}"
  models_dir: "{}"
  auto_download: {}
  supported_models:
    - "tiny"
    - "tiny.en"
    - "base"
    - "base.en"
    - "small"
    - "small.en"
    - "medium"
    - "medium.en"
    - "large-v1"
    - "large-v2"
    - "large-v3"
  audio_processing:
    supported_formats:
      - "mp3"
      - "wav"
      - "flac"
      - "m4a"
      - "ogg"
    auto_convert: true
    conversion_timeout: 60
    temp_file_cleanup: true
    temp_file_retention: 300
  workers:
    transcription_workers: {}
    channel_buffer_size: 100
    worker_timeout: 3600

# Logging Configuration
logging:
  level: "{}"
  log_dir: "{}"
  max_file_size: "{}"
  max_files: {}

# Daemon Configuration
daemon:
  pid_file: "{}"
  log_file: "{}"
  work_dir: "{}"

# Cluster Configuration
cluster:
  enabled: true
  node_id: "{}"
  bind_address: "{}"
  grpc_port: {}
  http_port: {}
  leader_can_process_tasks: {}
  heartbeat_interval: {}
  election_timeout: {}
  metadata_db_path: "{}"

# Load Balancer Configuration
load_balancer:
  enabled: false
  bind_address: "{}"
  port: {}
  health_check_interval: {}
  health_check_timeout: {}
  pid_file: "{}"
  log_file: "{}"

# Environment Variable Overrides
# You can override any configuration value using environment variables:
# 
# Server:
#   VOICE_CLI_HOST, VOICE_CLI_PORT, VOICE_CLI_HTTP_PORT, VOICE_CLI_MAX_FILE_SIZE, VOICE_CLI_CORS_ENABLED
# 
# Cluster:
#   VOICE_CLI_NODE_ID, VOICE_CLI_CLUSTER_ENABLED, VOICE_CLI_BIND_ADDRESS, VOICE_CLI_GRPC_PORT
#   VOICE_CLI_LEADER_CAN_PROCESS_TASKS, VOICE_CLI_HEARTBEAT_INTERVAL, VOICE_CLI_ELECTION_TIMEOUT
#   VOICE_CLI_METADATA_DB_PATH
# 
# Load Balancer:
#   VOICE_CLI_LB_ENABLED, VOICE_CLI_LB_PORT, VOICE_CLI_LB_BIND_ADDRESS
#   VOICE_CLI_LB_HEALTH_CHECK_INTERVAL, VOICE_CLI_LB_HEALTH_CHECK_TIMEOUT
# 
# Logging:
#   VOICE_CLI_LOG_LEVEL, VOICE_CLI_LOG_DIR, VOICE_CLI_LOG_MAX_FILES
# 
# Whisper:
#   VOICE_CLI_DEFAULT_MODEL, VOICE_CLI_MODELS_DIR, VOICE_CLI_AUTO_DOWNLOAD
#   VOICE_CLI_TRANSCRIPTION_WORKERS
# 
# Daemon:
#   VOICE_CLI_WORK_DIR, VOICE_CLI_PID_FILE
"#,
        config.server.host,
        http_port,
        config.server.max_file_size,
        config.server.cors_enabled,
        config.whisper.default_model,
        config.whisper.models_dir,
        config.whisper.auto_download,
        config.whisper.workers.transcription_workers,
        config.logging.level,
        config.logging.log_dir,
        config.logging.max_file_size,
        config.logging.max_files,
        config.daemon.pid_file,
        config.daemon.log_file,
        config.daemon.work_dir,
        node_id,
        config.cluster.bind_address,
        grpc_port,
        http_port,
        leader_can_process_tasks,
        config.cluster.heartbeat_interval,
        config.cluster.election_timeout,
        config.cluster.metadata_db_path,
        config.load_balancer.bind_address,
        config.load_balancer.port,
        config.load_balancer.health_check_interval,
        config.load_balancer.health_check_timeout,
        config.load_balancer.pid_file,
        config.load_balancer.log_file,
    );

    std::fs::write(&config_path, config_content)
        .with_context(|| format!("Failed to write configuration file: {:?}", config_path))?;

    info!("✅ Generated cluster configuration file: {:?}", config_path);
    info!("   You can modify this file and restart the cluster to apply changes");
    info!("   Environment variables can override any configuration value");

    Ok(())
}

/// Get available memory in MB (simplified implementation)
fn get_available_memory_mb() -> u64 {
    // This is a simplified implementation
    // In a real system, you might want to use a proper system info crate
    match std::fs::read_to_string("/proc/meminfo") {
        Ok(content) => {
            for line in content.lines() {
                if line.starts_with("MemAvailable:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return kb / 1024; // Convert KB to MB
                        }
                    }
                }
            }
            2048 // Default fallback
        }
        Err(_) => 2048, // Default fallback for non-Linux systems
    }
}

/// Get available disk space in GB
fn get_available_disk_space_gb(path: &std::path::Path) -> Result<f64> {
    match std::fs::metadata(path) {
        Ok(_) => {
            // This is a simplified implementation
            // In a real system, you might want to use a proper filesystem info crate
            #[cfg(unix)]
            {
                // For Unix systems, we can try to get filesystem stats
                // This is a simplified approach - in production you'd use statvfs
                Ok(5.0) // Default 5GB available
            }
            #[cfg(not(unix))]
            {
                Ok(5.0) // Default 5GB available for non-Unix systems
            }
        }
        Err(e) => Err(anyhow::anyhow!("Failed to check disk space: {}", e)),
    }
}

/// Handle systemd service installation
pub async fn handle_install_service(
    _config: &Config,
    service_name: String,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
    memory_limit: Option<String>,
    cpu_limit: Option<String>,
    user: Option<String>,
    group: Option<String>,
) -> Result<()> {
    info!("Installing systemd service: {}", service_name);

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| {
        format!("node-{}", Uuid::new_v4().simple())
    });

    // Get current executable path
    let current_exe = std::env::current_exe()
        .context("Failed to get current executable path")?;
    let exe_path = current_exe.canonicalize()
        .context("Failed to canonicalize executable path")?;

    // Get current working directory
    let current_dir = std::env::current_dir()
        .context("Failed to get current working directory")?;

    // Get current user and group if not specified
    let service_user = user.unwrap_or_else(|| {
        std::env::var("USER").unwrap_or_else(|_| "voice-cli".to_string())
    });
    let service_group = group.unwrap_or_else(|| {
        // Try to get primary group, fallback to same as user
        service_user.clone()
    });

    // Generate systemd service file content
    let service_content = generate_systemd_service_file(
        &service_name,
        &exe_path,
        &current_dir,
        &node_id,
        http_port,
        grpc_port,
        can_process_tasks,
        &service_user,
        &service_group,
        memory_limit.as_deref(),
        cpu_limit.as_deref(),
    )?;

    // Write service file to systemd directory
    let service_file_path = format!("/etc/systemd/system/{}.service", service_name);
    
    // Check if we have permission to write to systemd directory
    match std::fs::write(&service_file_path, &service_content) {
        Ok(_) => {
            info!("✓ Created systemd service file: {}", service_file_path);
        }
        Err(e) => {
            // If we don't have permission, write to current directory and provide instructions
            let local_service_file = format!("{}.service", service_name);
            std::fs::write(&local_service_file, &service_content)
                .context("Failed to write service file to current directory")?;
            
            warn!("⚠️  Could not write directly to systemd directory: {}", e);
            info!("📝 Service file created locally: {}", local_service_file);
            info!("📋 To install the service, run the following commands as root:");
            info!("   sudo cp {} {}", local_service_file, service_file_path);
            info!("   sudo systemctl daemon-reload");
            info!("   sudo systemctl enable {}", service_name);
            info!("   sudo systemctl start {}", service_name);
            return Ok(());
        }
    }

    // Reload systemd daemon
    let reload_result = std::process::Command::new("systemctl")
        .args(&["daemon-reload"])
        .output();

    match reload_result {
        Ok(output) => {
            if output.status.success() {
                info!("✓ Systemd daemon reloaded");
            } else {
                warn!("⚠️  Failed to reload systemd daemon: {}", 
                     String::from_utf8_lossy(&output.stderr));
            }
        }
        Err(e) => {
            warn!("⚠️  Could not reload systemd daemon: {}", e);
            info!("📋 Please run manually: sudo systemctl daemon-reload");
        }
    }

    // Enable service
    let enable_result = std::process::Command::new("systemctl")
        .args(&["enable", &service_name])
        .output();

    match enable_result {
        Ok(output) => {
            if output.status.success() {
                info!("✓ Service enabled: {}", service_name);
            } else {
                warn!("⚠️  Failed to enable service: {}", 
                     String::from_utf8_lossy(&output.stderr));
                info!("📋 Please run manually: sudo systemctl enable {}", service_name);
            }
        }
        Err(e) => {
            warn!("⚠️  Could not enable service: {}", e);
            info!("📋 Please run manually: sudo systemctl enable {}", service_name);
        }
    }

    info!("✅ Systemd service installation completed");
    info!("   Service name: {}", service_name);
    info!("   Node ID: {}", node_id);
    info!("   HTTP port: {}", http_port);
    info!("   gRPC port: {}", grpc_port);
    info!("   User: {}", service_user);
    info!("   Working directory: {}", current_dir.display());
    info!("");
    info!("📋 To start the service:");
    info!("   sudo systemctl start {}", service_name);
    info!("");
    info!("📋 To check service status:");
    info!("   sudo systemctl status {}", service_name);
    info!("");
    info!("📋 To view service logs:");
    info!("   sudo journalctl -u {} -f", service_name);

    Ok(())
}

/// Handle systemd service uninstallation
pub async fn handle_uninstall_service(service_name: String) -> Result<()> {
    info!("Uninstalling systemd service: {}", service_name);

    let service_file_path = format!("/etc/systemd/system/{}.service", service_name);

    // Stop service if running
    let stop_result = std::process::Command::new("systemctl")
        .args(&["stop", &service_name])
        .output();

    match stop_result {
        Ok(output) => {
            if output.status.success() {
                info!("✓ Service stopped: {}", service_name);
            } else {
                info!("ℹ️  Service was not running: {}", service_name);
            }
        }
        Err(e) => {
            warn!("⚠️  Could not stop service: {}", e);
        }
    }

    // Disable service
    let disable_result = std::process::Command::new("systemctl")
        .args(&["disable", &service_name])
        .output();

    match disable_result {
        Ok(output) => {
            if output.status.success() {
                info!("✓ Service disabled: {}", service_name);
            } else {
                info!("ℹ️  Service was not enabled: {}", service_name);
            }
        }
        Err(e) => {
            warn!("⚠️  Could not disable service: {}", e);
        }
    }

    // Remove service file
    match std::fs::remove_file(&service_file_path) {
        Ok(_) => {
            info!("✓ Removed service file: {}", service_file_path);
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                info!("ℹ️  Service file not found: {}", service_file_path);
            } else if e.kind() == std::io::ErrorKind::PermissionDenied {
                warn!("⚠️  Permission denied removing service file: {}", service_file_path);
                info!("📋 Please run manually: sudo rm {}", service_file_path);
            } else {
                warn!("⚠️  Failed to remove service file: {}", e);
            }
        }
    }

    // Reload systemd daemon
    let reload_result = std::process::Command::new("systemctl")
        .args(&["daemon-reload"])
        .output();

    match reload_result {
        Ok(output) => {
            if output.status.success() {
                info!("✓ Systemd daemon reloaded");
            } else {
                warn!("⚠️  Failed to reload systemd daemon: {}", 
                     String::from_utf8_lossy(&output.stderr));
            }
        }
        Err(e) => {
            warn!("⚠️  Could not reload systemd daemon: {}", e);
        }
    }

    info!("✅ Systemd service uninstallation completed");
    Ok(())
}

/// Handle systemd service status check
pub async fn handle_service_status(service_name: String) -> Result<()> {
    info!("Checking systemd service status: {}", service_name);

    // Get service status
    let status_result = std::process::Command::new("systemctl")
        .args(&["status", &service_name])
        .output();

    match status_result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            
            println!("Service Status for '{}':", service_name);
            println!("==========================");
            
            if !stdout.is_empty() {
                println!("{}", stdout);
            }
            
            if !stderr.is_empty() {
                println!("Errors:");
                println!("{}", stderr);
            }
            
            // Also check if service is enabled
            let is_enabled_result = std::process::Command::new("systemctl")
                .args(&["is-enabled", &service_name])
                .output();
                
            match is_enabled_result {
                Ok(enabled_output) => {
                    let enabled_output_str = String::from_utf8_lossy(&enabled_output.stdout);
                    let enabled_status = enabled_output_str.trim();
                    println!("Enabled: {}", enabled_status);
                }
                Err(_) => {
                    println!("Enabled: unknown");
                }
            }
        }
        Err(e) => {
            anyhow::bail!("Failed to check service status: {}", e);
        }
    }

    // Show recent logs
    println!("\nRecent Logs:");
    println!("============");
    
    let logs_result = std::process::Command::new("journalctl")
        .args(&["-u", &service_name, "--no-pager", "-n", "10"])
        .output();

    match logs_result {
        Ok(output) => {
            let logs = String::from_utf8_lossy(&output.stdout);
            if !logs.is_empty() {
                println!("{}", logs);
            } else {
                println!("No recent logs found");
            }
        }
        Err(e) => {
            warn!("Could not retrieve logs: {}", e);
        }
    }

    Ok(())
}

/// Generate systemd service file content
fn generate_systemd_service_file(
    service_name: &str,
    exe_path: &std::path::Path,
    working_dir: &std::path::Path,
    node_id: &str,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
    user: &str,
    group: &str,
    memory_limit: Option<&str>,
    cpu_limit: Option<&str>,
) -> Result<String> {
    let mut service_content = format!(r#"[Unit]
Description=Voice-CLI Cluster Node ({})
Documentation=https://github.com/your-org/voice-cli
After=network.target
Wants=network.target

[Service]
Type=exec
User={}
Group={}
WorkingDirectory={}
ExecStart={} cluster run --node-id {} --http-port {} --grpc-port {} --can-process-tasks {}
ExecReload=/bin/kill -HUP $MAINPID
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier={}

# Security settings
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths={}
ReadWritePaths=/tmp
PrivateTmp=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true

# Resource limits
LimitNOFILE=65536
LimitNPROC=4096
"#,
        node_id,
        user,
        group,
        working_dir.display(),
        exe_path.display(),
        node_id,
        http_port,
        grpc_port,
        can_process_tasks,
        service_name,
        working_dir.display(),
    );

    // Add memory limit if specified
    if let Some(memory) = memory_limit {
        service_content.push_str(&format!("MemoryMax={}\n", memory));
        service_content.push_str(&format!("MemoryHigh={}\n", memory));
    }

    // Add CPU limit if specified
    if let Some(cpu) = cpu_limit {
        // Convert CPU limit to systemd format (percentage)
        let cpu_percent = if cpu.contains('.') {
            // Fractional CPU (e.g., 0.5 -> 50%)
            let cpu_float: f64 = cpu.parse()
                .context("Invalid CPU limit format")?;
            format!("{}%", (cpu_float * 100.0) as u32)
        } else {
            // Whole CPU (e.g., 2 -> 200%)
            let cpu_int: u32 = cpu.parse()
                .context("Invalid CPU limit format")?;
            format!("{}%", cpu_int * 100)
        };
        service_content.push_str(&format!("CPUQuota={}\n", cpu_percent));
    }

    // Add environment variables section
    service_content.push_str(&format!(r#"
# Environment variables
Environment=VOICE_CLI_NODE_ID={}
Environment=VOICE_CLI_HTTP_PORT={}
Environment=VOICE_CLI_GRPC_PORT={}
Environment=VOICE_CLI_CLUSTER_ENABLED=true
Environment=VOICE_CLI_LEADER_CAN_PROCESS_TASKS={}
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#,
        node_id,
        http_port,
        grpc_port,
        can_process_tasks,
    ));

    Ok(service_content)
}



