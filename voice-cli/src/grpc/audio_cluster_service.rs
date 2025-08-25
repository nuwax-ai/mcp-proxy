use crate::cluster::{HeartbeatEvent, SimpleTaskScheduler};
use crate::grpc::proto::audio_cluster_service_server::{
    AudioClusterService, AudioClusterServiceServer,
};
use crate::grpc::proto::{
    ClusterHealth, ClusterStatusRequest, ClusterStatusResponse, HeartbeatRequest,
    HeartbeatResponse, JoinRequest, JoinResponse, LeaveRequest, LeaveResponse, NodeInfo, NodeRole,
    NodeStatus, TaskAssignmentRequest, TaskAssignmentResponse, TaskCompletionRequest,
    TaskCompletionResponse, TaskState,
};
use crate::models::{ClusterError, ClusterNode, MetadataStore, TaskMetadata};
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::{Code, Request, Response, Status};
use tracing::{debug, error, info, warn};

/// gRPC service implementation for AudioClusterService
#[derive(Clone)]
pub struct AudioClusterServiceImpl {
    /// Current node information
    node_info: ClusterNode,
    /// Metadata store for cluster data
    metadata_store: Arc<MetadataStore>,
    /// Task scheduler for leader nodes
    task_scheduler: Option<Arc<SimpleTaskScheduler>>,
    /// Heartbeat service for health monitoring
    heartbeat_service: Option<mpsc::UnboundedSender<HeartbeatEvent>>,
}

impl AudioClusterServiceImpl {
    /// Create a new AudioClusterServiceImpl
    pub fn new(
        node_info: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        task_scheduler: Option<Arc<SimpleTaskScheduler>>,
        heartbeat_service: Option<mpsc::UnboundedSender<HeartbeatEvent>>,
    ) -> Self {
        Self {
            node_info,
            metadata_store,
            task_scheduler,
            heartbeat_service,
        }
    }

    /// Convert ClusterNode to protobuf NodeInfo
    fn cluster_node_to_proto(&self, node: &ClusterNode) -> NodeInfo {
        NodeInfo {
            node_id: node.node_id.clone(),
            address: node.address.clone(),
            grpc_port: node.grpc_port as u32,
            http_port: node.http_port as u32,
            role: match node.role {
                crate::models::NodeRole::Leader => NodeRole::Leader as i32,
                crate::models::NodeRole::Follower => NodeRole::Follower as i32,
                crate::models::NodeRole::Candidate => NodeRole::Candidate as i32,
            },
            status: match node.status {
                crate::models::NodeStatus::Healthy => NodeStatus::Healthy as i32,
                crate::models::NodeStatus::Unhealthy => NodeStatus::Unhealthy as i32,
                crate::models::NodeStatus::Joining => NodeStatus::Joining as i32,
                crate::models::NodeStatus::Leaving => NodeStatus::Leaving as i32,
            },
            last_heartbeat: node.last_heartbeat,
        }
    }

    /// Convert protobuf NodeInfo to ClusterNode
    fn proto_to_cluster_node(&self, proto: &NodeInfo) -> Result<ClusterNode, ClusterError> {
        let role = match NodeRole::try_from(proto.role) {
            Ok(NodeRole::Leader) => crate::models::NodeRole::Leader,
            Ok(NodeRole::Follower) => crate::models::NodeRole::Follower,
            Ok(NodeRole::Candidate) => crate::models::NodeRole::Candidate,
            Err(_) => {
                return Err(ClusterError::InvalidOperation(
                    "Invalid node role".to_string(),
                ))
            }
        };

        let status = match NodeStatus::try_from(proto.status) {
            Ok(NodeStatus::Healthy) => crate::models::NodeStatus::Healthy,
            Ok(NodeStatus::Unhealthy) => crate::models::NodeStatus::Unhealthy,
            Ok(NodeStatus::Joining) => crate::models::NodeStatus::Joining,
            Ok(NodeStatus::Leaving) => crate::models::NodeStatus::Leaving,
            Err(_) => {
                return Err(ClusterError::InvalidOperation(
                    "Invalid node status".to_string(),
                ))
            }
        };

        let mut node = ClusterNode::new(
            proto.node_id.clone(),
            proto.address.clone(),
            proto.grpc_port as u16,
            proto.http_port as u16,
        );

        node.role = role;
        node.status = status;
        node.last_heartbeat = proto.last_heartbeat;

        Ok(node)
    }

    /// Convert ClusterError to gRPC Status
    fn cluster_error_to_status(&self, error: ClusterError) -> Status {
        match error {
            ClusterError::NodeNotFound(msg) => Status::new(Code::NotFound, msg),
            ClusterError::TaskNotFound(msg) => Status::new(Code::NotFound, msg),
            ClusterError::NoAvailableNodes => Status::new(Code::Unavailable, "No available nodes"),
            ClusterError::InvalidOperation(msg) => Status::new(Code::InvalidArgument, msg),
            ClusterError::Config(msg) => Status::new(Code::InvalidArgument, msg),
            ClusterError::Network(msg) => Status::new(Code::Unavailable, msg),
            ClusterError::Timeout(msg) => Status::new(Code::DeadlineExceeded, msg),
            ClusterError::TranscriptionFailed(msg) => Status::new(Code::Internal, msg),
            _ => Status::new(Code::Internal, error.to_string()),
        }
    }
}

#[tonic::async_trait]
impl AudioClusterService for AudioClusterServiceImpl {
    /// Handle node join requests
    async fn join_cluster(
        &self,
        request: Request<JoinRequest>,
    ) -> Result<Response<JoinResponse>, Status> {
        let req = request.into_inner();

        info!("Received join request from node: {:?}", req.node_info);

        // Validate request
        let node_info = req.node_info.ok_or_else(|| {
            warn!("Join request missing node info");
            Status::new(Code::InvalidArgument, "Missing node info")
        })?;

        // Validate node info fields
        if node_info.node_id.is_empty() {
            warn!("Join request has empty node_id");
            return Err(Status::new(
                Code::InvalidArgument,
                "Node ID cannot be empty",
            ));
        }

        if node_info.address.is_empty() {
            warn!("Join request has empty address");
            return Err(Status::new(
                Code::InvalidArgument,
                "Node address cannot be empty",
            ));
        }

        if node_info.grpc_port == 0 || node_info.http_port == 0 {
            warn!(
                "Join request has invalid ports: grpc={}, http={}",
                node_info.grpc_port, node_info.http_port
            );
            return Err(Status::new(Code::InvalidArgument, "Invalid port numbers"));
        }

        // Convert to ClusterNode
        let joining_node = self
            .proto_to_cluster_node(&node_info)
            .map_err(|e| self.cluster_error_to_status(e))?;

        // Add node to metadata store
        match self.metadata_store.add_node(&joining_node).await {
            Ok(_) => {
                info!(
                    "Successfully added node {} to cluster",
                    joining_node.node_id
                );

                // Get current cluster nodes
                let cluster_nodes = match self.metadata_store.get_all_nodes().await {
                    Ok(nodes) => nodes
                        .into_iter()
                        .map(|node| self.cluster_node_to_proto(&node))
                        .collect(),
                    Err(e) => {
                        error!("Failed to get cluster nodes: {}", e);
                        Vec::new()
                    }
                };

                let response = JoinResponse {
                    success: true,
                    message: format!("Node {} successfully joined cluster", joining_node.node_id),
                    cluster_nodes,
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                warn!("Failed to add node to cluster: {}", e);

                let response = JoinResponse {
                    success: false,
                    message: format!("Failed to join cluster: {}", e),
                    cluster_nodes: Vec::new(),
                };

                Ok(Response::new(response))
            }
        }
    }

    /// Handle node leave requests
    async fn leave_cluster(
        &self,
        request: Request<LeaveRequest>,
    ) -> Result<Response<LeaveResponse>, Status> {
        let req = request.into_inner();

        info!("Received leave request from node: {}", req.node_id);

        // Remove node from metadata store
        match self.metadata_store.remove_node(&req.node_id).await {
            Ok(_) => {
                info!("Successfully removed node {} from cluster", req.node_id);

                let response = LeaveResponse {
                    success: true,
                    message: format!("Node {} successfully left cluster", req.node_id),
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                warn!("Failed to remove node from cluster: {}", e);

                let response = LeaveResponse {
                    success: false,
                    message: format!("Failed to leave cluster: {}", e),
                };

                Ok(Response::new(response))
            }
        }
    }

    /// Get cluster status
    async fn get_cluster_status(
        &self,
        request: Request<ClusterStatusRequest>,
    ) -> Result<Response<ClusterStatusResponse>, Status> {
        let req = request.into_inner();

        debug!("Received cluster status request from node: {}", req.node_id);

        // Get all cluster nodes
        let nodes = self
            .metadata_store
            .get_all_nodes()
            .await
            .map_err(|e| self.cluster_error_to_status(e))?;

        // Convert to protobuf format
        let proto_nodes: Vec<NodeInfo> = nodes
            .iter()
            .map(|node| self.cluster_node_to_proto(node))
            .collect();

        // Find leader node
        let leader_node_id = nodes
            .iter()
            .find(|node| node.role == crate::models::NodeRole::Leader)
            .map(|node| node.node_id.clone())
            .unwrap_or_default();

        // Calculate cluster health
        let healthy_nodes = nodes
            .iter()
            .filter(|node| node.status == crate::models::NodeStatus::Healthy)
            .count();

        let cluster_health = if nodes.is_empty() {
            ClusterHealth::UnhealthyCluster
        } else {
            let health_ratio = healthy_nodes as f64 / nodes.len() as f64;
            if health_ratio >= 0.8 {
                ClusterHealth::HealthyCluster
            } else if health_ratio >= 0.5 {
                ClusterHealth::Degraded
            } else {
                ClusterHealth::UnhealthyCluster
            }
        };

        let response = ClusterStatusResponse {
            nodes: proto_nodes,
            leader_node_id,
            cluster_health: cluster_health as i32,
        };

        Ok(Response::new(response))
    }

    /// Handle heartbeat messages
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();

        debug!("Received heartbeat from node: {}", req.node_id);

        // Convert status and role
        let status = match NodeStatus::try_from(req.status) {
            Ok(NodeStatus::Healthy) => crate::models::NodeStatus::Healthy,
            Ok(NodeStatus::Unhealthy) => crate::models::NodeStatus::Unhealthy,
            Ok(NodeStatus::Joining) => crate::models::NodeStatus::Joining,
            Ok(NodeStatus::Leaving) => crate::models::NodeStatus::Leaving,
            Err(_) => return Err(Status::new(Code::InvalidArgument, "Invalid node status")),
        };

        // Update heartbeat in metadata store
        if let Err(e) = self.metadata_store.update_heartbeat(&req.node_id).await {
            warn!("Failed to update heartbeat for node {}: {}", req.node_id, e);
        }

        // Update node status
        if let Err(e) = self
            .metadata_store
            .update_node_status(&req.node_id, status)
            .await
        {
            warn!("Failed to update status for node {}: {}", req.node_id, e);
        }

        // Send heartbeat event to heartbeat service if available
        if let Some(ref heartbeat_tx) = self.heartbeat_service {
            let event = HeartbeatEvent::PeerHeartbeat {
                node_id: req.node_id.clone(),
                status,
                role: self.node_info.role, // Use current node's role for now
                timestamp: req.timestamp,
            };

            if let Err(_) = heartbeat_tx.send(event) {
                warn!("Failed to send heartbeat event to service");
            }
        }

        let response = HeartbeatResponse {
            success: true,
            message: "Heartbeat received".to_string(),
            current_role: match self.node_info.role {
                crate::models::NodeRole::Leader => NodeRole::Leader as i32,
                crate::models::NodeRole::Follower => NodeRole::Follower as i32,
                crate::models::NodeRole::Candidate => NodeRole::Candidate as i32,
            },
        };

        Ok(Response::new(response))
    }

    /// Assign task to a node
    async fn assign_task(
        &self,
        request: Request<TaskAssignmentRequest>,
    ) -> Result<Response<TaskAssignmentResponse>, Status> {
        let req = request.into_inner();

        info!(
            "Received task assignment request: task_id={}, client_id={}, filename={}",
            req.task_id, req.client_id, req.filename
        );

        // Validate request fields
        if req.task_id.is_empty() {
            warn!("Task assignment request has empty task_id");
            return Err(Status::new(
                Code::InvalidArgument,
                "Task ID cannot be empty",
            ));
        }

        if req.client_id.is_empty() {
            warn!("Task assignment request has empty client_id");
            return Err(Status::new(
                Code::InvalidArgument,
                "Client ID cannot be empty",
            ));
        }

        if req.filename.is_empty() {
            warn!("Task assignment request has empty filename");
            return Err(Status::new(
                Code::InvalidArgument,
                "Filename cannot be empty",
            ));
        }

        if req.audio_file_path.is_empty() {
            warn!("Task assignment request has empty audio_file_path");
            return Err(Status::new(
                Code::InvalidArgument,
                "Audio file path cannot be empty",
            ));
        }

        // Only leaders can assign tasks
        if self.node_info.role != crate::models::NodeRole::Leader {
            warn!(
                "Non-leader node {} attempted to assign task",
                self.node_info.node_id
            );
            return Err(Status::new(
                Code::PermissionDenied,
                "Only leader can assign tasks",
            ));
        }

        // Get task scheduler
        let scheduler = self
            .task_scheduler
            .as_ref()
            .ok_or_else(|| Status::new(Code::Internal, "Task scheduler not available"))?;

        // Create task metadata
        let task = TaskMetadata {
            task_id: req.task_id.clone(),
            client_id: req.client_id.clone(),
            filename: req.filename.clone(),
            audio_file_path: Some(req.audio_file_path.clone()),
            assigned_node: None,
            state: crate::models::TaskState::Pending,
            created_at: req.created_at,
            completed_at: None,
            error_message: None,
            model: if req.model.is_empty() {
                None
            } else {
                Some(req.model.clone())
            },
            response_format: if req.response_format.is_empty() {
                None
            } else {
                Some(req.response_format.clone())
            },
            processing_duration: None,
        };

        // Store task in metadata store
        if let Err(e) = self.metadata_store.create_task(&task).await {
            error!("Failed to create task: {}", e);
            return Err(self.cluster_error_to_status(e));
        }

        // Assign task using scheduler
        match scheduler.assign_next_task(req.task_id.clone()).await {
            Ok(assigned_node_id) => {
                info!("Task {} assigned to node {}", req.task_id, assigned_node_id);

                let response = TaskAssignmentResponse {
                    success: true,
                    message: format!("Task assigned to node {}", assigned_node_id),
                    assigned_node_id,
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                error!("Failed to assign task: {}", e);

                let response = TaskAssignmentResponse {
                    success: false,
                    message: format!("Failed to assign task: {}", e),
                    assigned_node_id: String::new(),
                };

                Ok(Response::new(response))
            }
        }
    }

    /// Report task completion
    async fn report_task_completion(
        &self,
        request: Request<TaskCompletionRequest>,
    ) -> Result<Response<TaskCompletionResponse>, Status> {
        let req = request.into_inner();

        info!(
            "Received task completion report: task_id={}, state={:?}",
            req.task_id, req.final_state
        );

        // Convert task state
        let final_state = match TaskState::try_from(req.final_state) {
            Ok(TaskState::Completed) => crate::models::TaskState::Completed,
            Ok(TaskState::Failed) => crate::models::TaskState::Failed,
            _ => return Err(Status::new(Code::InvalidArgument, "Invalid final state")),
        };

        // Update task in metadata store
        let result = match final_state {
            crate::models::TaskState::Completed => {
                // Calculate processing duration from task creation to completion
                let task_opt = self
                    .metadata_store
                    .get_task(&req.task_id)
                    .await
                    .map_err(|e| self.cluster_error_to_status(e))?;

                let processing_duration = if let Some(task) = task_opt {
                    if task.created_at > 0 && req.completed_at > task.created_at {
                        ((req.completed_at - task.created_at) as f64) as f32
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                info!(
                    "Task {} completed in {:.2} seconds",
                    req.task_id, processing_duration
                );
                self.metadata_store
                    .complete_task(&req.task_id, processing_duration)
                    .await
            }
            crate::models::TaskState::Failed => {
                error!("Task {} failed: {}", req.task_id, req.error_message);
                self.metadata_store
                    .fail_task(&req.task_id, &req.error_message)
                    .await
            }
            _ => {
                return Err(Status::new(Code::InvalidArgument, "Invalid final state"));
            }
        };

        match result {
            Ok(_) => {
                let response = TaskCompletionResponse {
                    success: true,
                    message: "Task completion reported successfully".to_string(),
                };

                Ok(Response::new(response))
            }
            Err(e) => {
                error!("Failed to update task completion: {}", e);

                let response = TaskCompletionResponse {
                    success: false,
                    message: format!("Failed to update task: {}", e),
                };

                Ok(Response::new(response))
            }
        }
    }
}

/// Create and configure the gRPC server
pub fn create_grpc_server(
    service_impl: AudioClusterServiceImpl,
) -> AudioClusterServiceServer<AudioClusterServiceImpl> {
    AudioClusterServiceServer::new(service_impl)
}
