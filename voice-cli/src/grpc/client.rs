use crate::grpc::proto::audio_cluster_service_client::AudioClusterServiceClient;
use crate::grpc::proto::{
    JoinRequest, JoinResponse, LeaveRequest, LeaveResponse,
    ClusterStatusRequest, ClusterStatusResponse, HeartbeatRequest, HeartbeatResponse,
    TaskAssignmentRequest, TaskAssignmentResponse, TaskCompletionRequest, TaskCompletionResponse,
    NodeInfo, NodeStatus, TaskState,
};
use crate::models::{ClusterError, ClusterNode, TaskMetadata};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};
use tracing::{debug, error, info, warn};

/// gRPC client for AudioClusterService
pub struct AudioClusterClient {
    client: AudioClusterServiceClient<Channel>,
    target_address: String,
}

impl AudioClusterClient {
    /// Create a new client connected to the specified address
    pub async fn connect(address: &str) -> Result<Self, ClusterError> {
        let endpoint = Endpoint::from_shared(format!("http://{}", address))
            .map_err(|e| ClusterError::Config(format!("Invalid endpoint: {}", e)))?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30));

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| ClusterError::Network(format!("Failed to connect to {}: {}", address, e)))?;

        let client = AudioClusterServiceClient::new(channel);

        Ok(Self {
            client,
            target_address: address.to_string(),
        })
    }

    /// Create a client with custom channel configuration
    pub fn new(channel: Channel, target_address: String) -> Self {
        let client = AudioClusterServiceClient::new(channel);
        Self {
            client,
            target_address,
        }
    }

    /// Join a cluster
    pub async fn join_cluster(
        &mut self,
        node_info: &ClusterNode,
        cluster_token: Option<String>,
    ) -> Result<JoinResponse, ClusterError> {
        debug!("Sending join request to {}", self.target_address);

        let node_info_proto = self.cluster_node_to_proto(node_info);
        let request = Request::new(JoinRequest {
            node_info: Some(node_info_proto),
            cluster_token: cluster_token.unwrap_or_default(),
        });

        match self.client.join_cluster(request).await {
            Ok(response) => {
                let join_response = response.into_inner();
                if join_response.success {
                    info!("Successfully joined cluster via {}", self.target_address);
                } else {
                    warn!("Failed to join cluster: {}", join_response.message);
                }
                Ok(join_response)
            }
            Err(status) => {
                error!("gRPC join_cluster failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Leave a cluster
    pub async fn leave_cluster(
        &mut self,
        node_id: &str,
        reason: Option<String>,
    ) -> Result<LeaveResponse, ClusterError> {
        debug!("Sending leave request to {}", self.target_address);

        let request = Request::new(LeaveRequest {
            node_id: node_id.to_string(),
            reason: reason.unwrap_or_default(),
        });

        match self.client.leave_cluster(request).await {
            Ok(response) => {
                let leave_response = response.into_inner();
                if leave_response.success {
                    info!("Successfully left cluster via {}", self.target_address);
                } else {
                    warn!("Failed to leave cluster: {}", leave_response.message);
                }
                Ok(leave_response)
            }
            Err(status) => {
                error!("gRPC leave_cluster failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Get cluster status
    pub async fn get_cluster_status(&mut self, node_id: &str) -> Result<ClusterStatusResponse, ClusterError> {
        debug!("Requesting cluster status from {}", self.target_address);

        let request = Request::new(ClusterStatusRequest {
            node_id: node_id.to_string(),
        });

        match self.client.get_cluster_status(request).await {
            Ok(response) => {
                let status_response = response.into_inner();
                debug!("Received cluster status with {} nodes", status_response.nodes.len());
                Ok(status_response)
            }
            Err(status) => {
                error!("gRPC get_cluster_status failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Send heartbeat
    pub async fn send_heartbeat(
        &mut self,
        node_id: &str,
        status: NodeStatus,
        timestamp: i64,
    ) -> Result<HeartbeatResponse, ClusterError> {
        debug!("Sending heartbeat to {}", self.target_address);

        let request = Request::new(HeartbeatRequest {
            node_id: node_id.to_string(),
            status: status as i32,
            timestamp,
        });

        match self.client.heartbeat(request).await {
            Ok(response) => {
                let heartbeat_response = response.into_inner();
                debug!("Heartbeat acknowledged by {}", self.target_address);
                Ok(heartbeat_response)
            }
            Err(status) => {
                error!("gRPC heartbeat failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Assign a task
    pub async fn assign_task(
        &mut self,
        task_id: &str,
        client_id: &str,
        filename: &str,
        audio_file_path: &str,
        model: Option<String>,
        response_format: Option<String>,
        created_at: i64,
    ) -> Result<TaskAssignmentResponse, ClusterError> {
        info!("Assigning task {} to cluster via {}", task_id, self.target_address);

        let request = Request::new(TaskAssignmentRequest {
            task_id: task_id.to_string(),
            client_id: client_id.to_string(),
            filename: filename.to_string(),
            audio_file_path: audio_file_path.to_string(),
            model: model.unwrap_or_default(),
            response_format: response_format.unwrap_or_default(),
            created_at,
        });

        match self.client.assign_task(request).await {
            Ok(response) => {
                let assignment_response = response.into_inner();
                if assignment_response.success {
                    info!("Task {} assigned successfully to node {}", 
                          task_id, assignment_response.assigned_node_id);
                } else {
                    warn!("Failed to assign task {}: {}", task_id, assignment_response.message);
                }
                Ok(assignment_response)
            }
            Err(status) => {
                error!("gRPC assign_task failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Report task completion
    pub async fn report_task_completion(
        &mut self,
        task_id: &str,
        final_state: TaskState,
        error_message: Option<String>,
        result_data: Option<String>,
        completed_at: i64,
    ) -> Result<TaskCompletionResponse, ClusterError> {
        info!("Reporting completion of task {} to cluster via {}", task_id, self.target_address);

        let request = Request::new(TaskCompletionRequest {
            task_id: task_id.to_string(),
            final_state: final_state as i32,
            error_message: error_message.unwrap_or_default(),
            result_data: result_data.unwrap_or_default(),
            completed_at,
        });

        match self.client.report_task_completion(request).await {
            Ok(response) => {
                let completion_response = response.into_inner();
                if completion_response.success {
                    info!("Task {} completion reported successfully", task_id);
                } else {
                    warn!("Failed to report task {} completion: {}", task_id, completion_response.message);
                }
                Ok(completion_response)
            }
            Err(status) => {
                error!("gRPC report_task_completion failed: {}", status);
                Err(self.status_to_cluster_error(status))
            }
        }
    }

    /// Helper: Convert ClusterNode to protobuf NodeInfo
    fn cluster_node_to_proto(&self, node: &ClusterNode) -> NodeInfo {
        NodeInfo {
            node_id: node.node_id.clone(),
            address: node.address.clone(),
            grpc_port: node.grpc_port as u32,
            http_port: node.http_port as u32,
            role: match node.role {
                crate::models::NodeRole::Leader => crate::grpc::proto::NodeRole::Leader as i32,
                crate::models::NodeRole::Follower => crate::grpc::proto::NodeRole::Follower as i32,
                crate::models::NodeRole::Candidate => crate::grpc::proto::NodeRole::Candidate as i32,
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

    /// Helper: Convert gRPC Status to ClusterError
    fn status_to_cluster_error(&self, status: Status) -> ClusterError {
        match status.code() {
            tonic::Code::NotFound => ClusterError::NodeNotFound(status.message().to_string()),
            tonic::Code::InvalidArgument => ClusterError::InvalidOperation(status.message().to_string()),
            tonic::Code::PermissionDenied => ClusterError::InvalidOperation(status.message().to_string()),
            tonic::Code::Unavailable => ClusterError::Network(status.message().to_string()),
            tonic::Code::DeadlineExceeded => ClusterError::Timeout(status.message().to_string()),
            _ => ClusterError::Network(format!("gRPC error: {}", status.message())),
        }
    }

    /// Get the target address this client is connected to
    pub fn target_address(&self) -> &str {
        &self.target_address
    }
}

/// Convenience function to create a client and connect
pub async fn connect_to_cluster_node(address: &str) -> Result<AudioClusterClient, ClusterError> {
    AudioClusterClient::connect(address).await
}

/// Helper function to assign a task from TaskMetadata
pub async fn assign_task_from_metadata(
    client: &mut AudioClusterClient,
    task: &TaskMetadata,
    audio_file_path: &str,
) -> Result<TaskAssignmentResponse, ClusterError> {
    client.assign_task(
        &task.task_id,
        &task.client_id,
        &task.filename,
        audio_file_path,
        task.model.clone(),
        task.response_format.clone(),
        task.created_at,
    ).await
}

/// Helper function to report task completion from TaskMetadata
pub async fn report_completion_from_metadata(
    client: &mut AudioClusterClient,
    task: &TaskMetadata,
    result_data: Option<String>,
) -> Result<TaskCompletionResponse, ClusterError> {
    let final_state = match task.state {
        crate::models::TaskState::Completed => TaskState::Completed,
        crate::models::TaskState::Failed => TaskState::Failed,
        _ => return Err(ClusterError::InvalidOperation("Task is not in a final state".to_string())),
    };

    client.report_task_completion(
        &task.task_id,
        final_state,
        task.error_message.clone(),
        result_data,
        task.completed_at.unwrap_or_else(|| chrono::Utc::now().timestamp()),
    ).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NodeRole;

    #[tokio::test]
    async fn test_cluster_node_to_proto() {
        let client = AudioClusterClient {
            client: AudioClusterServiceClient::new(
                tonic::transport::Channel::from_static("http://[::1]:50051")
                    .connect().await.unwrap()
            ),
            target_address: "test".to_string(),
        };

        let node = ClusterNode::new(
            "test-node".to_string(),
            "127.0.0.1".to_string(),
            50051,
            8080,
        );

        let proto = client.cluster_node_to_proto(&node);
        assert_eq!(proto.node_id, "test-node");
        assert_eq!(proto.address, "127.0.0.1");
        assert_eq!(proto.grpc_port, 50051);
        assert_eq!(proto.http_port, 8080);
    }
}