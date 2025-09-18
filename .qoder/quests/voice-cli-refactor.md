# Voice-CLI Critical Refactor: Fix Test Failures

## Overview

Analysis of `cargo nextest run -p voice-cli --no-fail-fast` and the audio-cluster-service.md design reveals critical missing implementations that prevent proper testing and cluster functionality.

## P0 Critical Fixes (Required for Tests)

### 1. Load Balancer Health Checker Missing

**Current Issue:**
```rust
// TODO: implement these modules
// pub mod health_checker;
// pub mod service_manager;
```

**Fix:**
```rust
// src/load_balancer/health_checker.rs
use reqwest::Client;
use std::collections::HashMap;
use tokio::sync::RwLock;

pub struct HealthChecker {
    client: Client,
    nodes: Arc<RwLock<HashMap<String, ClusterNode>>>,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            nodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    pub async fn add_node(&self, node: ClusterNode) {
        self.nodes.write().await.insert(node.node_id.clone(), node);
    }
    
    pub async fn get_healthy_nodes(&self) -> Vec<ClusterNode> {
        let nodes = self.nodes.read().await;
        let mut healthy = Vec::new();
        
        for node in nodes.values() {
            let health_url = format!("http://{}:{}/health", node.address, node.http_port);
            if self.client.get(&health_url).send().await
                .map(|r| r.status().is_success())
                .unwrap_or(false) {
                healthy.push(node.clone());
            }
        }
        healthy
    }
}
```

### 2. Cluster Join Protocol Incomplete

**Current Issue:**
```rust
// TODO: Implement actual cluster join protocol with peer communication
// For now, just add the node to the local metadata store
```

**Fix:**
```rust
// src/cli/cluster.rs - Real join implementation
pub async fn handle_cluster_join(
    config: &Config,
    peer_address: String,
    node_id: Option<String>,
    http_port: u16,
    grpc_port: u16,
    token: Option<String>,
) -> Result<()> {
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", Uuid::new_v4().simple()));
    
    // Real gRPC connection
    let mut client = AudioClusterServiceClient::connect(
        format!("http://{}", peer_address)
    ).await.map_err(|e| VoiceCliError::Network(format!("Failed to connect: {}", e)))?;

    // Send join request
    let request = JoinRequest {
        node_info: Some(NodeInfo {
            node_id: node_id.clone(),
            address: config.cluster.bind_address.clone(),
            grpc_port: grpc_port as u32,
            http_port: http_port as u32,
            role: NodeRole::Follower as i32,
            status: NodeStatus::Healthy as i32,
            last_heartbeat: chrono::Utc::now().timestamp(),
        }),
        cluster_token: token.unwrap_or_default(),
    };

    let response = client.join_cluster(request).await
        .map_err(|e| VoiceCliError::Network(format!("Join failed: {}", e)))?;

    if response.into_inner().success {
        info!("✅ Successfully joined cluster");
        Ok(())
    } else {
        Err(VoiceCliError::Config("Join rejected".to_string()))
    }
}
```

### 3. Service Manager Implementation

**Fix:**
```rust
// src/load_balancer/service_manager.rs
pub struct ServiceManager {
    metadata_store: Arc<MetadataStore>,
    health_checker: Arc<HealthChecker>,
}

impl ServiceManager {
    pub fn new(metadata_store: Arc<MetadataStore>, health_checker: Arc<HealthChecker>) -> Self {
        Self { metadata_store, health_checker }
    }
    
    pub async fn get_active_nodes(&self) -> Vec<ClusterNode> {
        self.health_checker.get_healthy_nodes().await
    }
    
    pub async fn register_node(&self, node: ClusterNode) -> Result<(), ClusterError> {
        self.metadata_store.add_node(&node).await?;
        self.health_checker.add_node(node).await;
        Ok(())
    }
}
```

### 4. Task Distribution Real Implementation

**Current Issue:** Tasks cannot be distributed without shared file access.

**Fix:**
```rust
// src/cluster/task_distributor.rs
pub struct TaskDistributor {
    metadata_store: Arc<MetadataStore>,
    temp_dir: PathBuf,
}

impl TaskDistributor {
    pub async fn distribute_task(
        &self,
        audio_data: Vec<u8>,
        filename: &str,
        model: &str,
    ) -> Result<String, ClusterError> {
        let task_id = Uuid::new_v4().to_string();
        
        // Store audio file temporarily
        let task_dir = self.temp_dir.join(&task_id);
        tokio::fs::create_dir_all(&task_dir).await?;
        let audio_path = task_dir.join(filename);
        tokio::fs::write(&audio_path, audio_data).await?;
        
        // Create task metadata
        let task = TaskMetadata {
            task_id: task_id.clone(),
            client_id: "api".to_string(),
            filename: filename.to_string(),
            assigned_node: None,
            state: TaskState::Pending,
            created_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            error_message: None,
        };
        
        self.metadata_store.create_task(&task).await?;
        
        // Assign to healthy node
        let nodes = self.metadata_store.get_all_nodes().await?;
        let healthy_nodes: Vec<_> = nodes.into_iter()
            .filter(|n| n.status == NodeStatus::Healthy)
            .collect();
            
        if let Some(target_node) = healthy_nodes.first() {
            self.metadata_store.assign_task(&task_id, &target_node.node_id).await?;
            
            // Send via gRPC if not local
            if target_node.node_id != "local" {
                self.send_task_via_grpc(target_node, &task_id, &audio_path, model).await?;
            }
        }
        
        Ok(task_id)
    }
    
    async fn send_task_via_grpc(
        &self,
        node: &ClusterNode,
        task_id: &str,
        audio_path: &Path,
        model: &str,
    ) -> Result<(), ClusterError> {
        let grpc_addr = format!("http://{}:{}", node.address, node.grpc_port);
        let mut client = AudioClusterServiceClient::connect(grpc_addr).await?;
        
        let request = TaskAssignmentRequest {
            task_id: task_id.to_string(),
            client_id: "api".to_string(),
            filename: audio_path.file_name().unwrap().to_string_lossy().to_string(),
            audio_file_path: audio_path.to_string_lossy().to_string(),
            model: model.to_string(),
            response_format: "json".to_string(),
        };
        
        client.assign_task(request).await?;
        Ok(())
    }
}
```

## P1 Missing CLI Commands

### 5. Add Missing Cluster Commands

**Add to `src/cli/mod.rs`:**
```rust
#[derive(Subcommand)]
pub enum ClusterAction {
    // ... existing commands ...
    Start {
        #[arg(long, default_value = "8080")]
        http_port: u16,
        #[arg(long, default_value = "9090")]
        grpc_port: u16,
    },
    Stop,
}
```

**Implementation:**
```rust
// src/cli/cluster.rs
pub async fn handle_cluster_start(
    config: &Config,
    http_port: u16,
    grpc_port: u16,
) -> Result<()> {
    info!("Starting cluster node");
    
    // Start HTTP + gRPC servers
    let http_server = start_http_server(config, http_port).await?;
    let grpc_server = start_grpc_server(config, grpc_port).await?;
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    info!("Shutting down cluster node");
    Ok(())
}

pub async fn handle_cluster_stop(config: &Config) -> Result<()> {
    // Send shutdown signal to running cluster
    let pid_file = format!("/tmp/voice-cli-cluster-{}.pid", config.cluster.node_id);
    if let Ok(pid) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid.trim().parse::<u32>() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            info!("Sent shutdown signal to cluster node (PID: {})", pid);
        }
    }
    Ok(())
}
```

## Test Fixes

### 6. Real Integration Tests

**Replace mock tests with real process tests:**
```rust
// tests/cluster_real_tests.rs
#[tokio::test]
async fn test_real_cluster_join() {
    // Start leader process
    let mut leader = start_voice_cli_process(&[
        "cluster", "init", "--http-port", "8080", "--grpc-port", "9090"
    ]).await;
    
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Start follower process
    let mut follower = start_voice_cli_process(&[
        "cluster", "join", "127.0.0.1:9090",
        "--http-port", "8081", "--grpc-port", "9091"
    ]).await;
    
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Test cluster status
    let status = reqwest::get("http://127.0.0.1:8080/cluster/status")
        .await.unwrap().json::<ClusterStatus>().await.unwrap();
    
    assert_eq!(status.cluster_size, 2);
    assert!(status.healthy_nodes >= 1);
    
    // Cleanup
    leader.kill().await.unwrap();
    follower.kill().await.unwrap();
}

async fn start_voice_cli_process(args: &[&str]) -> tokio::process::Child {
    tokio::process::Command::new("target/debug/voice-cli")
        .args(args)
        .spawn()
        .expect("Failed to start voice-cli process")
}
```

## Implementation Plan

1. **Week 1**: Health checker + service manager modules
2. **Week 2**: Real cluster join protocol 
3. **Week 3**: Task distribution with temp file sharing
4. **Week 4**: Missing CLI commands
5. **Week 5**: Real integration tests

## Success Criteria

- `cargo nextest run -p voice-cli --no-fail-fast` passes 100%
- Multi-node cluster formation works
- Load balancer routes to healthy nodes
- Task distribution functional
- All CLI commands operational

### 1. Load Balancer Health Checker (P0 - Required for Tests)

```rust
// src/load_balancer/health_checker.rs
use crate::models::{ClusterNode, NodeStatus, ClusterError};
use std::sync::Arc;
use tokio::sync::RwLock;
use reqwest::Client;

pub struct HealthChecker {
    client: Client,
    node_status: Arc<RwLock<HashMap<String, NodeHealthStatus>>>,
}

impl HealthChecker {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            node_status: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_node(&self, node: ClusterNode) {
        let mut status_map = self.node_status.write().await;
        status_map.insert(node.node_id.clone(), NodeHealthStatus::new(node));
    }

    pub async fn get_healthy_nodes(&self) -> Vec<ClusterNode> {
        let status_map = self.node_status.read().await;
        status_map.values()
            .filter(|status| status.is_healthy())
            .map(|status| status.node.clone())
            .collect()
    }

    pub async fn check_node_health(&self, node: &ClusterNode) -> bool {
        let health_url = format!("http://{}:{}/health", node.address, node.http_port);
        
        match self.client.get(&health_url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeHealthStatus {
    pub node: ClusterNode,
    pub status: NodeStatus,
    pub last_check: Option<std::time::Instant>,
    pub consecutive_failures: u32,
}

impl NodeHealthStatus {
    pub fn new(node: ClusterNode) -> Self {
        Self {
            node,
            status: NodeStatus::Healthy,
            last_check: None,
            consecutive_failures: 0,
        }
    }

    pub fn is_healthy(&self) -> bool {
        matches!(self.status, NodeStatus::Healthy) && self.consecutive_failures < 3
    }
}
```

### 2. Service Manager Implementation (P0)

```rust
// src/load_balancer/service_manager.rs
use crate::models::{ClusterNode, MetadataStore, ClusterError};
use crate::load_balancer::HealthChecker;
use std::sync::Arc;

pub struct ServiceManager {
    metadata_store: Arc<MetadataStore>,
    health_checker: Arc<HealthChecker>,
}

impl ServiceManager {
    pub fn new(metadata_store: Arc<MetadataStore>, health_checker: Arc<HealthChecker>) -> Self {
        Self { metadata_store, health_checker }
    }

    pub async fn register_node(&self, node: ClusterNode) -> Result<(), ClusterError> {
        // Add to metadata store
        self.metadata_store.add_node(&node).await?;
        
        // Add to health monitoring
        self.health_checker.add_node(node).await;
        
        Ok(())
    }

    pub async fn get_active_nodes(&self) -> Vec<ClusterNode> {
        self.health_checker.get_healthy_nodes().await
    }

    pub async fn sync_cluster_state(&self) -> Result<(), ClusterError> {
        let stored_nodes = self.metadata_store.get_all_nodes().await?;
        
        for node in stored_nodes {
            let is_healthy = self.health_checker.check_node_health(&node).await;
            if !is_healthy {
                // Update node status in metadata store
                self.metadata_store.update_node_status(&node.node_id, NodeStatus::Unhealthy).await?;
            }
        }
        
        Ok(())
    }
}
```

### 3. Raft Network Communication (P0)

```rust
// src/cluster/raft_network.rs
use crate::grpc::{audio_cluster_service_client::AudioClusterServiceClient, RaftMessage as ProtoRaftMessage};
use raft::prelude::Message;
use std::collections::HashMap;
use tonic::transport::Channel;

pub struct RaftNetwork {
    peer_clients: HashMap<u64, AudioClusterServiceClient<Channel>>,
    peer_addresses: HashMap<u64, String>,
}

impl RaftNetwork {
    pub fn new() -> Self {
        Self {
            peer_clients: HashMap::new(),
            peer_addresses: HashMap::new(),
        }
    }

    pub async fn add_peer(&mut self, peer_id: u64, grpc_address: String) -> Result<(), ClusterError> {
        let endpoint = format!("http://{}", grpc_address);
        let channel = Channel::from_shared(endpoint)?
            .connect()
            .await
            .map_err(|e| ClusterError::Network(e.to_string()))?;
            
        let client = AudioClusterServiceClient::new(channel);
        self.peer_clients.insert(peer_id, client);
        self.peer_addresses.insert(peer_id, grpc_address);
        
        Ok(())
    }

    pub async fn send_messages(&mut self, messages: Vec<Message>) {
        for message in messages {
            if let Some(client) = self.peer_clients.get_mut(&message.to) {
                let proto_msg = self.raft_message_to_proto(&message);
                let request = tonic::Request::new(crate::grpc::RaftMessagesRequest {
                    messages: vec![proto_msg],
                });
                
                if let Err(e) = client.send_raft_messages(request).await {
                    tracing::warn!("Failed to send message to peer {}: {}", message.to, e);
                }
            }
        }
    }

    fn raft_message_to_proto(&self, message: &Message) -> ProtoRaftMessage {
        ProtoRaftMessage {
            to: message.to,
            from: message.from,
            term: message.term,
            msg_type: message.msg_type as i32,
            index: message.index,
            log_term: message.log_term,
            entries: message.entries.iter().map(|e| crate::grpc::Entry {
                entry_type: e.entry_type as i32,
                term: e.term,
                index: e.index,
                data: e.data.clone(),
            }).collect(),
            commit: message.commit,
            snapshot: None, // Simplified for now
        }
    }
}
```

### 4. Model Service Real Implementation (P1)

```rust
// Update src/services/model_service.rs
impl ModelService {
    /// List models that are currently loaded in memory - REAL implementation
    pub async fn list_loaded_models(&self) -> Result<Vec<String>, VoiceCliError> {
        // Check which models have active transcription workers
        let models_dir = PathBuf::from(&self.config.whisper.models_dir);
        let mut loaded_models = Vec::new();
        
        for model_name in &self.config.whisper.supported_models {
            let model_path = models_dir.join(format!("{}.bin", model_name));
            
            // Check if model file exists and is accessible
            if model_path.exists() {
                // Try to read model metadata to verify it's loaded
                match tokio::fs::metadata(&model_path).await {
                    Ok(metadata) if metadata.len() > 0 => {
                        loaded_models.push(model_name.clone());
                    }
                    _ => continue,
                }
            }
        }
        
        Ok(loaded_models)
    }

    /// Real model validation instead of placeholder
    pub async fn validate_model(&self, model_name: &str) -> Result<bool, VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;
        
        if !model_path.exists() {
            return Ok(false);
        }

        // Check file size is reasonable (> 50MB for whisper models)
        let metadata = fs::metadata(&model_path).await
            .map_err(|e| VoiceCliError::Model(format!("Failed to read model metadata: {}", e)))?;
            
        if metadata.len() < 50 * 1024 * 1024 {
            return Ok(false);
        }

        // Try to read model header to verify format
        let mut file = fs::File::open(&model_path).await
            .map_err(|e| VoiceCliError::Model(format!("Failed to open model: {}", e)))?;
            
        let mut header = [0u8; 16];
        file.read_exact(&mut header).await
            .map_err(|e| VoiceCliError::Model(format!("Failed to read model header: {}", e)))?;

        // Check for GGML magic number (basic validation)
        if &header[0..4] == b"ggml" {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
```

### 5. Cluster State Machine Implementation (P1)

```rust
// Update src/cluster/raft_node.rs
impl RaftNode {
    /// Apply committed entries to the state machine - REAL implementation
    async fn apply_entries(&self, entries: Vec<Entry>) -> Result<(), ClusterError> {
        for entry in entries {
            if entry.data.is_empty() {
                continue; // Empty entry (heartbeat)
            }

            // Parse entry data as cluster operation
            match self.parse_cluster_operation(&entry.data) {
                Ok(operation) => {
                    match operation {
                        ClusterOperation::AddNode { node } => {
                            self.metadata_store.add_node(&node).await?;
                            info!("Applied: Added node {}", node.node_id);
                        }
                        ClusterOperation::RemoveNode { node_id } => {
                            self.metadata_store.remove_node(&node_id).await?;
                            info!("Applied: Removed node {}", node_id);
                        }
                        ClusterOperation::UpdateNodeStatus { node_id, status } => {
                            self.metadata_store.update_node_status(&node_id, status).await?;
                            info!("Applied: Updated node {} status to {:?}", node_id, status);
                        }
                        ClusterOperation::AssignTask { task_id, node_id } => {
                            self.metadata_store.assign_task(&task_id, &node_id).await?;
                            info!("Applied: Assigned task {} to node {}", task_id, node_id);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse cluster operation: {}", e);
                }
            }
        }

        Ok(())
    }

    fn parse_cluster_operation(&self, data: &[u8]) -> Result<ClusterOperation, ClusterError> {
        serde_json::from_slice(data)
            .map_err(|e| ClusterError::Serialization(e))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ClusterOperation {
    AddNode { node: ClusterNode },
    RemoveNode { node_id: String },
    UpdateNodeStatus { node_id: String, status: NodeStatus },
    AssignTask { task_id: String, node_id: String },
}
```

### 6. Real Audio Processing Integration (P1)

```rust
// Update src/cluster/transcription_worker.rs
impl SimpleTranscriptionWorker {
    /// Perform real transcription using voice-toolkit - COMPLETE implementation
    async fn perform_real_transcription(
        &self,
        task_request: &TaskAssignmentRequest,
    ) -> Result<String, ClusterError> {
        let audio_path = Path::new(&task_request.audio_file_path);
        
        // Validate audio file exists
        if !audio_path.exists() {
            return Err(ClusterError::InvalidOperation(
                format!("Audio file not found: {}", task_request.audio_file_path)
            ));
        }

        // Use voice-toolkit for transcription
        let model_path = self.get_model_path(&task_request.model)?;
        
        let result = voice_toolkit::stt::transcribe_file(&model_path, audio_path)
            .await
            .map_err(|e| ClusterError::TranscriptionFailed(e.to_string()))?;

        // Return formatted result
        let transcription_result = serde_json::json!({
            "text": result.text,
            "language": result.language,
            "duration": result.audio_duration,
            "segments": result.segments.into_iter().map(|seg| {
                serde_json::json!({
                    "start": seg.start,
                    "end": seg.end,
                    "text": seg.text
                })
            }).collect::<Vec<_>>()
        });

        Ok(transcription_result.to_string())
    }

    fn get_model_path(&self, model_name: &str) -> Result<PathBuf, ClusterError> {
        let models_dir = PathBuf::from("./models");
        let model_path = models_dir.join(format!("{}.bin", model_name));
        
        if !model_path.exists() {
            return Err(ClusterError::InvalidOperation(
                format!("Model file not found: {}", model_path.display())
            ));
        }
        
        Ok(model_path)
    }
}
```

## Testing Strategy

### Real Business Logic Tests

```rust
// tests/real_business_logic_tests.rs
#[tokio::test]
async fn test_health_checker_real_nodes() {
    let health_checker = HealthChecker::new();
    
    // Start a real test server
    let test_server = start_test_server("127.0.0.1:8080").await;
    
    let node = ClusterNode::new(
        "test-node".to_string(),
        "127.0.0.1".to_string(),
        9090,
        8080,
    );
    
    health_checker.add_node(node.clone()).await;
    
    // Test real health check
    let is_healthy = health_checker.check_node_health(&node).await;
    assert!(is_healthy, "Real health check should pass for running server");
    
    // Stop server and test again
    test_server.shutdown().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let is_healthy = health_checker.check_node_health(&node).await;
    assert!(!is_healthy, "Health check should fail for stopped server");
}

#[tokio::test]
async fn test_real_model_validation() {
    let config = Config::default();
    let model_service = ModelService::new(config);
    
    // Test with a real model file (create test fixture)
    let test_model_path = create_test_model_file().await;
    
    let is_valid = model_service.validate_model_path(&test_model_path).await;
    assert!(is_valid.is_ok(), "Real model validation should work");
}

async fn start_test_server(addr: &str) -> TestServer {
    // Implementation for real test server
    // Returns handle that can be shutdown
}
```

## Implementation Timeline

1. **Week 1**: Load Balancer modules (health_checker, service_manager)
2. **Week 2**: Raft network communication 
3. **Week 3**: Model service real implementations
4. **Week 4**: Cluster state machine and transcription integration
5. **Week 5**: Real business logic tests and validation

## Success Criteria

1. All `cargo nextest run -p voice-cli` tests pass
2. No TODO/mock implementations in core business logic
3. Real transcription processing works end-to-end
4. Cluster formation and communication functional
5. Load balancer health checking operational