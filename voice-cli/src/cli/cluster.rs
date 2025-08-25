use crate::cluster::{ClusterServiceManager, ClusterState, SimpleTaskScheduler};
use crate::config::{ConfigTemplateGenerator, ServiceConfigLoader, ServiceType};
use crate::error::ClusterResultExt;
use crate::models::Config;
use crate::models::{ClusterError, ClusterNode, MetadataStore, NodeRole, NodeStatus, TaskState};
use crate::utils::get_cluster_ip;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use anyhow::{Context, Result};
use serde_yaml;

/// Initialize cluster configuration
pub async fn handle_cluster_init(
    config_path: Option<PathBuf>,
    http_port: Option<u16>,
    grpc_port: Option<u16>,
    force: bool,
) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("cluster-config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 生成基础配置
    ConfigTemplateGenerator::generate_config_file(ServiceType::Cluster, &output_path)?;

    // 如果指定了端口参数，更新配置文件
    if http_port.is_some() || grpc_port.is_some() {
        let mut config =
            ServiceConfigLoader::load_service_config(ServiceType::Cluster, Some(&output_path))?;

        if let Some(port) = http_port {
            config.server.port = port;
            config.cluster.http_port = port;
        }

        if let Some(port) = grpc_port {
            config.cluster.grpc_port = port;
        }

        config
            .save(&output_path)
            .map_err(|e| crate::VoiceCliError::Config(e.to_string()))?;
    }

    println!("✅ Cluster configuration initialized: {:?}", output_path);
    if let Some(http_port) = http_port {
        println!("🔧 HTTP port set to: {}", http_port);
    }
    if let Some(grpc_port) = grpc_port {
        println!("🔧 gRPC port set to: {}", grpc_port);
    }
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli cluster start --config {:?}", output_path);

    Ok(())
}

// 全局任务管理器，用于管理后台运行的集群节点
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static CLUSTER_TASKS: OnceLock<Mutex<HashMap<String, (CancellationToken, broadcast::Sender<()>)>>> =
    OnceLock::new();

fn get_cluster_tasks() -> &'static Mutex<HashMap<String, (CancellationToken, broadcast::Sender<()>)>>
{
    CLUSTER_TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Resolve the advertise IP address for cluster communication
/// Returns the advertise IP based on priority:
/// 1. User-provided advertise_ip parameter (with validation)
/// 2. Auto-detected cluster IP (non-loopback)
/// 3. Fallback to converted bind address (0.0.0.0 -> 127.0.0.1)
async fn resolve_advertise_ip(
    config: &Config,
    advertise_ip: Option<String>,
    require_explicit: bool,
) -> Result<String> {
    // For join operations, require explicit IP specification
    if require_explicit && advertise_ip.is_none() {
        anyhow::bail!(
            "Advertise IP is required for cluster join operations. \
             Please specify --advertise-ip with the IP address this node should use for cluster communication.\n\
             Example: voice-cli cluster join <peer-address> --advertise-ip 192.168.1.100"
        );
    }

    // Priority 1: Use user-provided advertise IP
    if let Some(ip_str) = advertise_ip {
        // Validate the provided IP address
        match ip_str.parse::<IpAddr>() {
            Ok(ip_addr) => {
                // Additional validation for user-provided IP
                if let Err(validation_error) = validate_ip_for_cluster_use(&ip_addr).await {
                    warn!(
                        "User-provided IP '{}' validation warning: {}",
                        ip_str, validation_error
                    );
                    // Continue anyway but log the warning
                }
                info!("Using user-provided advertise IP: {}", ip_str);
                return Ok(ip_str);
            }
            Err(e) => {
                if require_explicit {
                    anyhow::bail!("Invalid advertise IP '{}': {}", ip_str, e);
                }
                warn!(
                    "Invalid advertise IP '{}': {}, falling back to auto-detection",
                    ip_str, e
                );
            }
        }
    }

    // Priority 2: Auto-detect cluster IP (only if not required to be explicit)
    if !require_explicit {
        match get_cluster_ip() {
            Ok(detected_ip) => {
                let ip_str = detected_ip.to_string();
                info!("Auto-detected cluster advertise IP: {}", ip_str);

                // Validate the auto-detected IP
                if let Err(validation_error) = validate_ip_for_cluster_use(&detected_ip).await {
                    warn!("Auto-detected IP validation warning: {}", validation_error);
                }

                return Ok(ip_str);
            }
            Err(e) => {
                warn!("Failed to auto-detect cluster IP: {}, using fallback", e);
            }
        }
    }

    // Priority 3: Fallback to converted bind address (only if not required to be explicit)
    if !require_explicit {
        let fallback_ip = if config.cluster.bind_address == "0.0.0.0" {
            "127.0.0.1".to_string()
        } else {
            config.cluster.bind_address.clone()
        };

        warn!("Using fallback advertise IP: {}", fallback_ip);
        return Ok(fallback_ip);
    }

    anyhow::bail!("Failed to resolve advertise IP address");
}

/// Validate IP address for cluster use
/// This function performs various checks to ensure the IP is suitable for cluster communication
async fn validate_ip_for_cluster_use(ip: &IpAddr) -> Result<(), String> {
    // Check 1: Ensure it's not a loopback address (unless explicitly allowed)
    if ip.is_loopback() {
        return Err("Loopback addresses may cause issues in multi-node clusters".to_string());
    }

    // Check 2: Ensure it's not a link-local address
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_link_local() {
                return Err(
                    "Link-local IPv4 addresses are not suitable for cluster communication"
                        .to_string(),
                );
            }
        }
        IpAddr::V6(v6) => {
            // IPv6 link-local addresses start with fe80::
            let segments = v6.segments();
            if segments[0] & 0xffc0 == 0xfe80 {
                return Err(
                    "Link-local IPv6 addresses are not suitable for cluster communication"
                        .to_string(),
                );
            }
        }
    }

    // Check 3: Basic reachability test (optional, for better UX)
    // We'll skip this for now to avoid blocking startup, but it could be added later

    Ok(())
}

/// Handle cluster node run command (foreground mode)
pub async fn handle_cluster_run(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
    advertise_ip: Option<String>,
) -> Result<()> {
    info!("Running cluster node in foreground mode");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple()));

    info!("Cluster node run requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Can process tasks: {}", can_process_tasks);
    info!("  Advertise IP: {:?}", advertise_ip);
    info!("  Metadata DB Path: {}", config.cluster.metadata_db_path);

    // Resolve the advertise IP address - require explicit specification for join
    let resolved_advertise_ip = resolve_advertise_ip(config, advertise_ip, true)
        .await
        .context("Failed to resolve advertise IP address")?;

    info!("Using advertise IP: {}", resolved_advertise_ip);

    // Create a modified config with the specified parameters
    let mut cluster_config = config.clone();
    cluster_config.cluster.node_id = node_id;
    cluster_config.cluster.http_port = http_port;
    cluster_config.cluster.grpc_port = grpc_port;
    cluster_config.cluster.leader_can_process_tasks = can_process_tasks;
    // Set the resolved advertise IP as the bind address for cluster operations
    cluster_config.cluster.bind_address = resolved_advertise_ip;

    // Start the cluster node server
    start_cluster_node_server(Arc::new(cluster_config)).await
}

/// Handle cluster node start command (background daemon mode)
pub async fn handle_cluster_start(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
    save_config: bool,
    advertise_ip: Option<String>,
) -> Result<()> {
    info!("Starting cluster node in background daemon mode");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple()));

    // Check if already running
    if is_cluster_node_running(http_port).await? {
        anyhow::bail!("Cluster node is already running on port {}", http_port);
    }

    // Check if this node is already managed
    {
        let tasks = get_cluster_tasks().lock().unwrap();
        if tasks.contains_key(&node_id) {
            anyhow::bail!("Cluster node '{}' is already running", node_id);
        }
    }

    info!("Cluster node daemon start requested:");
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Can process tasks: {}", can_process_tasks);
    info!("  Save config: {}", save_config);
    info!("  Advertise IP: {:?}", advertise_ip);

    // Resolve the advertise IP address
    let resolved_advertise_ip = resolve_advertise_ip(config, advertise_ip, false)
        .await
        .context("Failed to resolve advertise IP address")?;

    info!("Using advertise IP: {}", resolved_advertise_ip);

    // Save configuration to file if requested
    if save_config {
        save_config_to_file(config, &node_id, http_port, grpc_port, can_process_tasks)
            .await
            .context("Failed to save configuration to file")?;
    }

    // Create a modified config with the specified parameters
    let mut cluster_config = config.clone();
    cluster_config.cluster.node_id = node_id.clone();
    cluster_config.cluster.http_port = http_port;
    cluster_config.cluster.grpc_port = grpc_port;
    cluster_config.cluster.leader_can_process_tasks = can_process_tasks;
    // Set the resolved advertise IP as the bind address for cluster operations
    cluster_config.cluster.bind_address = resolved_advertise_ip;

    // Create cancellation token and shutdown channel
    let cancellation_token = CancellationToken::new();
    let (shutdown_tx, _) = broadcast::channel(1);

    // Register the task in global manager
    {
        let mut tasks = get_cluster_tasks().lock().unwrap();
        tasks.insert(
            node_id.clone(),
            (cancellation_token.clone(), shutdown_tx.clone()),
        );
    }

    // Generate task ID file path
    let task_id_file = format!("./voice-cli-cluster-{}.pid", node_id);

    // Spawn the cluster node server as a background task
    spawn_cluster_node_daemon(
        Arc::new(cluster_config),
        task_id_file.clone(),
        cancellation_token.clone(),
        shutdown_tx,
    )
    .await?;

    // Store task identifier for management
    let task_id = Uuid::new_v4().simple().to_string();
    std::fs::write(&task_id_file, &task_id).context("Failed to write task ID file")?;

    info!("Cluster node daemon started with task ID: {}", task_id);
    info!("Task ID file: {}", task_id_file);
    info!("Node ID: {}", node_id);

    // Wait a moment and check if it's actually running
    tokio::time::sleep(Duration::from_secs(3)).await;

    if is_cluster_node_running(http_port).await? {
        info!("✅ Cluster node is running successfully");
        info!("   HTTP API: {}:{}", config.cluster.bind_address, http_port);
        info!(
            "   gRPC Cluster: {}:{}",
            config.cluster.bind_address, grpc_port
        );
        info!("   Node ID: {}", node_id);
        Ok(())
    } else {
        // Clean up task ID file and task registration if startup failed
        let _ = std::fs::remove_file(&task_id_file);
        {
            let mut tasks = get_cluster_tasks().lock().unwrap();
            tasks.remove(&node_id);
        }
        anyhow::bail!("Cluster node daemon failed to start")
    }
}

/// Spawn cluster node server as a background task
/// Uses CancellationToken for proper shutdown management
async fn spawn_cluster_node_daemon(
    config: Arc<Config>,
    _pid_file: String,
    cancellation_token: CancellationToken,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<()> {
    info!("Spawning cluster node daemon task with cancellation support");

    // Clone config for the background task
    let task_config = Arc::clone(&config);
    let token = cancellation_token.clone();
    let shutdown_sender = shutdown_tx.clone();

    // Spawn the background task (detached)
    tokio::task::spawn(async move {
        // Set up multiple shutdown signals
        let ctrl_c_signal = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install CTRL+C signal handler");
            info!("Received CTRL+C signal in daemon task");
        };

        let cancellation_signal = token.cancelled();

        // Start the cluster node server
        tokio::select! {
            result = start_cluster_node_server_with_shutdown(task_config, token.clone()) => {
                match result {
                    Ok(_) => info!("Cluster node daemon completed successfully"),
                    Err(e) => error!("Cluster node daemon failed: {}", e),
                }
            }
            _ = ctrl_c_signal => {
                info!("Cluster node daemon shutting down via CTRL+C");
                token.cancel();
            }
            _ = cancellation_signal => {
                info!("Cluster node daemon shutting down via cancellation token");
            }
        }

        // Send shutdown signal to any listeners
        let _ = shutdown_sender.send(());
        info!("Cluster node daemon task completed");
    });

    info!("Cluster node daemon task spawned successfully with cancellation support");
    Ok(())
}

/// Handle cluster node stop command using standard cancellation mechanism
pub async fn handle_cluster_stop(config: &Config) -> Result<()> {
    info!("Stopping cluster nodes using cancellation token");

    let mut stopped_nodes = 0;

    // Try to stop all running tasks using the global task manager
    {
        let mut tasks = get_cluster_tasks().lock().unwrap();
        let running_nodes: Vec<String> = tasks.keys().cloned().collect();

        for node_id in running_nodes {
            if let Some((cancellation_token, shutdown_tx)) = tasks.remove(&node_id) {
                info!(
                    "Found running task for node {}, sending cancellation signal",
                    node_id
                );

                // Cancel the task using the standard cancellation token
                cancellation_token.cancel();

                // Wait for shutdown confirmation
                let mut shutdown_rx = shutdown_tx.subscribe();
                drop(tasks); // Release the lock before async operation

                // Wait for shutdown with timeout
                let shutdown_result =
                    tokio::time::timeout(Duration::from_secs(5), shutdown_rx.recv()).await;

                match shutdown_result {
                    Ok(Ok(())) => {
                        info!(
                            "✅ Cluster node '{}' stopped successfully via cancellation token",
                            node_id
                        );
                        stopped_nodes += 1;
                    }
                    Ok(Err(_)) => {
                        warn!(
                            "Shutdown channel closed unexpectedly for node '{}'.",
                            node_id
                        );
                    }
                    Err(_) => {
                        warn!("Shutdown timed out after 5 seconds for node '{}'.", node_id);
                    }
                }

                // Clean up task ID file
                let task_id_file = format!("./voice-cli-cluster-{}.pid", node_id);
                if std::path::Path::new(&task_id_file).exists() {
                    let _ = std::fs::remove_file(&task_id_file);
                    info!("Cleaned up task ID file: {}", task_id_file);
                }

                // Re-acquire lock for next iteration
                tasks = get_cluster_tasks().lock().unwrap();
            }
        }
    }

    // Also clean up any orphaned PID files
    let current_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Ok(entries) = std::fs::read_dir(&current_dir) {
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if file_name_str.starts_with("voice-cli-cluster-") && file_name_str.ends_with(".pid") {
                if let Err(e) = std::fs::remove_file(entry.path()) {
                    warn!("Failed to remove PID file {:?}: {}", entry.path(), e);
                } else {
                    info!("Cleaned up orphaned PID file: {:?}", entry.path());
                }
            }
        }
    }

    if stopped_nodes > 0 {
        info!("✅ Successfully stopped {} cluster node(s)", stopped_nodes);
    } else {
        info!("ℹ️  No running cluster nodes found to stop");

        // Check if there are any nodes in the cluster metadata that might be running
        if let Ok(metadata_store) = MetadataStore::new(&config.cluster.metadata_db_path) {
            if let Ok(nodes) = metadata_store.get_all_nodes().await {
                if !nodes.is_empty() {
                    info!(
                        "Found {} node(s) in cluster metadata. Checking for running processes...",
                        nodes.len()
                    );

                    for node in &nodes {
                        let is_online = check_node_online(&node.address, node.http_port).await;
                        if is_online {
                            warn!("Node '{}' appears to be running at {}:{} but not managed by this process", 
                                  node.node_id, node.address, node.http_port);
                            warn!("You may need to stop it manually or use system signals (SIGTERM/SIGINT)");
                        }
                    }
                }
            }
        }
    }

    info!("Cluster node stop process completed");
    Ok(())
}

/// Handle cluster node restart command
pub async fn handle_cluster_restart(
    config: &Config,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
    save_config: bool,
    advertise_ip: Option<String>,
) -> Result<()> {
    info!("Restarting cluster node");

    // Try to stop if running (ignore errors if not running)
    let _ = handle_cluster_stop(config).await;

    // Wait a moment for cleanup
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Start again
    handle_cluster_start(
        config,
        node_id,
        http_port,
        grpc_port,
        can_process_tasks,
        save_config,
        advertise_ip,
    )
    .await
}

/// Handle cluster join command
pub async fn handle_cluster_join(
    config: &Config,
    peer_address: String,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    token: Option<String>,
    advertise_ip: Option<String>,
) -> Result<()> {
    info!("Joining cluster via peer: {}", peer_address);

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple()));

    info!("Cluster join requested:");
    info!("  Peer Address: {}", peer_address);
    info!("  Node ID: {}", node_id);
    info!("  gRPC Port: {}", grpc_port);
    info!("  HTTP Port: {}", http_port);
    info!("  Token: {:?}", token);
    info!("  Advertise IP: {:?}", advertise_ip);
    info!("  Metadata DB Path: {}", config.cluster.metadata_db_path);

    // Resolve the advertise IP address - require explicit specification for join
    let resolved_advertise_ip = resolve_advertise_ip(config, advertise_ip, true)
        .await
        .context("Failed to resolve advertise IP address")?;

    info!("Using advertise IP: {}", resolved_advertise_ip);

    // Initialize metadata store
    let metadata_store = Arc::new(
        MetadataStore::new(&config.cluster.metadata_db_path)
            .map_err(ClusterError::from)
            .context("Failed to initialize metadata store")?,
    );

    // Create cluster node configuration for this joining node
    // Use the resolved advertise IP
    let cluster_node = ClusterNode {
        node_id: node_id.clone(),
        address: resolved_advertise_ip.clone(),
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
            info!(
                "   Cluster now has {} nodes",
                join_response.cluster_nodes.len()
            );

            // Update local metadata store with cluster nodes information
            for node_proto in &join_response.cluster_nodes {
                if let Ok(remote_node) = proto_to_cluster_node(node_proto) {
                    if remote_node.node_id != cluster_node.node_id {
                        // Add other cluster nodes to our local metadata
                        if let Err(e) = metadata_store.add_node(&remote_node).await {
                            warn!(
                                "Failed to add remote node {} to local metadata: {}",
                                remote_node.node_id, e
                            );
                        } else {
                            info!(
                                "Added remote node {} to local metadata",
                                remote_node.node_id
                            );
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
    info!("   Advertise IP: {}", resolved_advertise_ip);
    info!("   gRPC Address: {}:{}", resolved_advertise_ip, grpc_port);
    info!("   HTTP Address: {}:{}", resolved_advertise_ip, http_port);
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
            println!("Total Nodes: {}", nodes.len());

            if nodes.is_empty() {
                println!("❌ No nodes found in cluster");
                println!("\nℹ️  Run 'voice-cli cluster init' to initialize the cluster");
                return Ok(());
            }

            println!();

            // Show all nodes with their actual status
            for (i, node) in nodes.iter().enumerate() {
                let is_online = check_node_online(&node.address, node.http_port).await;
                let status_icon = if is_online { "🟢" } else { "🔴" };
                let status_text = if is_online { "ONLINE" } else { "OFFLINE" };

                let role = match node.role {
                    NodeRole::Leader => "Leader",
                    NodeRole::Follower => "Follower",
                    NodeRole::Candidate => "Candidate",
                };

                let node_status = match node.status {
                    NodeStatus::Healthy => "Healthy",
                    NodeStatus::Unhealthy => "Unhealthy",
                    NodeStatus::Joining => "Joining",
                    NodeStatus::Leaving => "Leaving",
                };

                println!("Node {} - {} {}", i + 1, status_icon, status_text);
                println!("  Node ID: {}", node.node_id);
                println!("  Role: {} ({})", role, node_status);
                println!("  HTTP API: {}:{}", node.address, node.http_port);
                println!("  gRPC: {}:{}", node.address, node.grpc_port);

                if detailed {
                    let last_seen = chrono::DateTime::from_timestamp(node.last_heartbeat, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                        .unwrap_or_else(|| "Unknown".to_string());
                    println!("  Last Heartbeat: {}", last_seen);
                }

                println!();
            }

            // Summary statistics
            let online_nodes = {
                let mut count = 0;
                for node in &nodes {
                    if check_node_online(&node.address, node.http_port).await {
                        count += 1;
                    }
                }
                count
            };

            let offline_nodes = nodes.len() - online_nodes;
            let healthy_nodes = nodes
                .iter()
                .filter(|n| matches!(n.status, NodeStatus::Healthy))
                .count();
            let leader_nodes = nodes
                .iter()
                .filter(|n| matches!(n.role, NodeRole::Leader))
                .count();

            println!("Cluster Summary:");
            println!("===============");
            println!("🟢 Online Nodes: {}", online_nodes);
            println!("🔴 Offline Nodes: {}", offline_nodes);
            println!("💚 Healthy Nodes: {}", healthy_nodes);
            println!("👑 Leader Nodes: {}", leader_nodes);

            // Cluster health assessment
            if online_nodes == 0 {
                println!("\n❌ Cluster Status: OFFLINE");
                println!("   All nodes are offline. Start nodes with 'voice-cli cluster start'");
            } else if online_nodes == nodes.len() {
                println!("\n✅ Cluster Status: FULLY OPERATIONAL");
                if leader_nodes == 0 {
                    println!("   ⚠️  Warning: No leader nodes detected");
                }
            } else {
                println!("\n⚠️  Cluster Status: PARTIALLY OPERATIONAL");
                println!("   {} of {} nodes are online", online_nodes, nodes.len());
            }

            if detailed {
                println!();
                println!("Configuration Details:");
                println!("=====================");
                println!("Config Cluster Enabled: {}", config.cluster.enabled);
                println!("Config Node ID: {}", config.cluster.node_id);
                println!("Config HTTP Port: {}", config.cluster.http_port);
                println!("Config gRPC Port: {}", config.cluster.grpc_port);
                println!(
                    "Leader can process tasks: {}",
                    config.cluster.leader_can_process_tasks
                );
                println!("Heartbeat interval: {}s", config.cluster.heartbeat_interval);
                println!("Election timeout: {}s", config.cluster.election_timeout);
                println!("Bind address: {}", config.cluster.bind_address);

                // Check for active tasks using public methods
                let pending_tasks = metadata_store
                    .get_tasks_by_state(TaskState::Pending)
                    .await
                    .unwrap_or_default();
                let assigned_tasks = metadata_store
                    .get_tasks_by_state(TaskState::Assigned)
                    .await
                    .unwrap_or_default();
                let processing_tasks = metadata_store
                    .get_tasks_by_state(TaskState::Processing)
                    .await
                    .unwrap_or_default();
                let completed_tasks = metadata_store
                    .get_tasks_by_state(TaskState::Completed)
                    .await
                    .unwrap_or_default();
                let failed_tasks = metadata_store
                    .get_tasks_by_state(TaskState::Failed)
                    .await
                    .unwrap_or_default();

                let mut all_tasks = Vec::new();
                all_tasks.extend(pending_tasks);
                all_tasks.extend(assigned_tasks);
                all_tasks.extend(processing_tasks);
                all_tasks.extend(completed_tasks);
                all_tasks.extend(failed_tasks);

                let active_tasks: Vec<_> = all_tasks
                    .iter()
                    .filter(|task| task.completed_at.is_none())
                    .collect();

                println!();
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
        }
        Err(e) => {
            error!("Failed to access cluster metadata: {}", e);
            println!("❌ Cluster Status: UNAVAILABLE");
            println!("Error: {}", e);
            println!();
            println!("This might indicate:");
            println!("- Cluster has not been initialized (run 'voice-cli cluster init')");
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
    std::fs::write(&output_path, config_content).context("Failed to write config file")?;

    println!("Cluster configuration generated: {}", output_path);
    Ok(())
}

/// Generate default cluster configuration
fn generate_default_cluster_config(config: &Config) -> String {
    format!(
        r#"# Voice-CLI Cluster Configuration
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
"#,
        config.cluster.metadata_db_path, config.logging.log_dir
    )
}

/// Generate production cluster configuration
fn generate_production_cluster_config(config: &Config) -> String {
    format!(
        r#"# Voice-CLI Production Cluster Configuration

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
"#,
        config.cluster.metadata_db_path, config.logging.log_dir
    )
}

/// Generate development cluster configuration
fn generate_development_cluster_config(config: &Config) -> String {
    format!(
        r#"# Voice-CLI Development Cluster Configuration

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
"#,
        config.cluster.metadata_db_path, config.logging.log_dir
    )
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
    let mut client = crate::grpc::client::AudioClusterClient::connect(&grpc_address)
        .await
        .with_context(|| format!("Failed to connect to peer {}", grpc_address))?;

    info!("Connected to peer, sending join request...");

    // Send join request
    let join_response = client
        .join_cluster(joining_node, cluster_token)
        .await
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
            let port = parts[1]
                .parse::<u16>()
                .with_context(|| format!("Invalid port in peer address: {}", peer_address))?;
            Ok((host, port))
        }
        _ => {
            anyhow::bail!(
                "Invalid peer address format: {}. Expected format: host:port or host",
                peer_address
            )
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

/// Start the actual cluster node server using ClusterServiceManager with shutdown support
async fn start_cluster_node_server_with_shutdown(
    config: Arc<Config>,
    cancellation_token: CancellationToken,
) -> Result<()> {
    info!("Starting cluster node server with shutdown support");

    let node_id = config.cluster.node_id.clone();
    let http_port = config.cluster.http_port;
    let grpc_port = config.cluster.grpc_port;
    let can_process_tasks = config.cluster.leader_can_process_tasks;

    info!("  Node ID: {}", node_id);
    info!("  HTTP Port: {}", http_port);
    info!("  gRPC Port: {}", grpc_port);
    info!("  Can process tasks: {}", can_process_tasks);

    // Initialize metadata store
    let metadata_store = Arc::new(
        MetadataStore::new(&config.cluster.metadata_db_path)
            .with_cluster_context("initialize metadata store")?,
    );

    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());

    // Create cluster node configuration
    // Convert bind address to connectable address (0.0.0.0 -> 127.0.0.1)
    let connectable_address = if config.cluster.bind_address == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        config.cluster.bind_address.clone()
    };

    let cluster_node = ClusterNode::new(node_id.clone(), connectable_address, grpc_port, http_port);

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

    info!(
        "✅ Starting cluster node '{}' services with cancellation support",
        node_id
    );
    info!("   HTTP API: {}:{}", config.cluster.bind_address, http_port);
    info!(
        "   gRPC Cluster: {}:{}",
        config.cluster.bind_address, grpc_port
    );
    info!(
        "   Task Processing: {}",
        if can_process_tasks {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    info!("   Metadata Store: {}", config.cluster.metadata_db_path);

    // Start services with cancellation support
    tokio::select! {
        result = service_manager.start() => {
            match result {
                Ok(_) => info!("Cluster node '{}' completed successfully", node_id),
                Err(e) => {
                    error!("Cluster node '{}' failed: {}", node_id, e);
                    return Err(e);
                }
            }
        }
        _ = cancellation_token.cancelled() => {
            info!("Cluster node '{}' received cancellation signal, shutting down", node_id);

            // Send shutdown signal to service manager
            let shutdown_handle = service_manager.shutdown_handle();
            let _ = shutdown_handle.send(());

            // Wait a bit for graceful shutdown
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    info!(
        "Cluster node '{}' shut down gracefully via cancellation token",
        node_id
    );
    Ok(())
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
    let metadata_store = Arc::new(
        MetadataStore::new(&config.cluster.metadata_db_path)
            .with_cluster_context("initialize metadata store")?,
    );

    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());

    // Create cluster node configuration
    // Convert bind address to connectable address (0.0.0.0 -> 127.0.0.1)
    let connectable_address = if config.cluster.bind_address == "0.0.0.0" {
        "127.0.0.1".to_string()
    } else {
        config.cluster.bind_address.clone()
    };

    let cluster_node = ClusterNode::new(node_id.clone(), connectable_address, grpc_port, http_port);

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
    info!(
        "   gRPC Cluster: {}:{}",
        config.cluster.bind_address, grpc_port
    );
    info!(
        "   Task Processing: {}",
        if can_process_tasks {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    info!("   Metadata Store: {}", config.cluster.metadata_db_path);

    // Start all services concurrently
    service_manager
        .start()
        .await
        .context("Failed to start cluster services")?;

    info!("Cluster node '{}' shut down gracefully", node_id);
    Ok(())
}

/// Check if cluster node is running on a specific port
async fn is_cluster_node_running(port: u16) -> Result<bool> {
    Ok(
        tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok(),
    )
}

/// Check if a node is online by attempting to connect to its HTTP port
async fn check_node_online(address: &str, port: u16) -> bool {
    // Try to connect to the HTTP port with a short timeout
    let connect_result = tokio::time::timeout(
        tokio::time::Duration::from_millis(1000), // 1 second timeout
        tokio::net::TcpStream::connect(format!("{}:{}", address, port)),
    )
    .await;

    connect_result.is_ok() && connect_result.unwrap().is_ok()
}

/// Save configuration parameters to config file
async fn save_config_to_file(
    config: &Config,
    node_id: &str,
    http_port: u16,
    grpc_port: u16,
    can_process_tasks: bool,
) -> Result<()> {
    info!("Saving configuration to file");

    // Create a new config with updated values
    let mut updated_config = config.clone();
    updated_config.cluster.enabled = true;
    updated_config.cluster.node_id = node_id.to_string();
    updated_config.cluster.http_port = http_port;
    updated_config.cluster.grpc_port = grpc_port;
    updated_config.cluster.leader_can_process_tasks = can_process_tasks;
    updated_config.server.port = http_port; // Keep server port in sync

    // Serialize the updated config to YAML
    let config_yaml = serde_yaml::to_string(&updated_config)
        .context("Failed to serialize configuration to YAML")?;

    // Write to config.yml in current directory
    let config_path = std::path::PathBuf::from("config.yml");
    std::fs::write(&config_path, config_yaml)
        .with_context(|| format!("Failed to write configuration to {:?}", config_path))?;

    info!("✅ Configuration saved to {:?}", config_path);
    info!("   Node ID: {}", node_id);
    info!("   HTTP Port: {}", http_port);
    info!("   gRPC Port: {}", grpc_port);
    info!("   Leader can process tasks: {}", can_process_tasks);
    info!("   Cluster enabled: true");

    Ok(())
}

/// Create proper directory structure for cluster initialization
async fn create_cluster_directory_structure(config: &Config) -> Result<()> {
    info!("Creating cluster directory structure");

    // Create metadata database directory
    let metadata_db_path = std::path::PathBuf::from(&config.cluster.metadata_db_path);
    if let Some(parent) = metadata_db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create metadata DB parent directory: {:?}",
                parent
            )
        })?;
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
async fn validate_cluster_environment(
    config: &Config,
    http_port: u16,
    grpc_port: u16,
) -> Result<()> {
    info!("Validating cluster environment and dependencies");

    // Check if ports are available
    if let Err(_) =
        tokio::net::TcpListener::bind(format!("{}:{}", config.cluster.bind_address, http_port))
            .await
    {
        anyhow::bail!(
            "HTTP port {}:{} is not available",
            config.cluster.bind_address,
            http_port
        );
    }
    info!(
        "✓ HTTP port {}:{} is available",
        config.cluster.bind_address, http_port
    );

    if let Err(_) =
        tokio::net::TcpListener::bind(format!("{}:{}", config.cluster.bind_address, grpc_port))
            .await
    {
        anyhow::bail!(
            "gRPC port {}:{} is not available",
            config.cluster.bind_address,
            grpc_port
        );
    }
    info!(
        "✓ gRPC port {}:{} is available",
        config.cluster.bind_address, grpc_port
    );

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
    config
        .validate()
        .context("Configuration validation failed")?;
    info!("✓ Configuration validation passed");

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

    let config_content = format!(
        r#"# Voice-CLI Cluster Configuration
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
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple()));

    // Get current executable path
    let current_exe = std::env::current_exe().context("Failed to get current executable path")?;
    let exe_path = current_exe
        .canonicalize()
        .context("Failed to canonicalize executable path")?;

    // Get current working directory
    let current_dir = std::env::current_dir().context("Failed to get current working directory")?;

    // Get current user and group if not specified
    let service_user =
        user.unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "voice-cli".to_string()));
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
                warn!(
                    "⚠️  Failed to reload systemd daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
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
                warn!(
                    "⚠️  Failed to enable service: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                info!(
                    "📋 Please run manually: sudo systemctl enable {}",
                    service_name
                );
            }
        }
        Err(e) => {
            warn!("⚠️  Could not enable service: {}", e);
            info!(
                "📋 Please run manually: sudo systemctl enable {}",
                service_name
            );
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
                warn!(
                    "⚠️  Permission denied removing service file: {}",
                    service_file_path
                );
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
                warn!(
                    "⚠️  Failed to reload systemd daemon: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
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
    let mut service_content = format!(
        r#"[Unit]
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
            let cpu_float: f64 = cpu.parse().context("Invalid CPU limit format")?;
            format!("{}%", (cpu_float * 100.0) as u32)
        } else {
            // Whole CPU (e.g., 2 -> 200%)
            let cpu_int: u32 = cpu.parse().context("Invalid CPU limit format")?;
            format!("{}%", cpu_int * 100)
        };
        service_content.push_str(&format!("CPUQuota={}\n", cpu_percent));
    }

    // Add environment variables section
    service_content.push_str(&format!(
        r#"
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
        node_id, http_port, grpc_port, can_process_tasks,
    ));

    Ok(service_content)
}
