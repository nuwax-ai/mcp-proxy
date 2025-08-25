use crate::grpc::proto::audio_cluster_service_server::AudioClusterServiceServer;
use crate::grpc::AudioClusterServiceImpl;
use crate::models::{ClusterError, MetadataStore, ClusterNode};
use crate::cluster::{SimpleTaskScheduler, HeartbeatEvent};
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::transport::Server;
use tracing::{info, error};

/// gRPC server configuration
#[derive(Debug, Clone)]
pub struct GrpcServerConfig {
    /// Address to bind the gRPC server
    pub bind_address: String,
    /// Port to bind the gRPC server
    pub port: u16,
    /// Maximum message size for gRPC requests
    pub max_message_size: usize,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 50051,
            max_message_size: 4 * 1024 * 1024, // 4MB
        }
    }
}

/// gRPC server wrapper for AudioClusterService
pub struct AudioClusterGrpcServer {
    config: GrpcServerConfig,
    service_impl: AudioClusterServiceImpl,
}

impl AudioClusterGrpcServer {
    /// Create a new gRPC server instance
    pub fn new(
        config: GrpcServerConfig,
        node_info: ClusterNode,
        metadata_store: Arc<MetadataStore>,
        task_scheduler: Option<Arc<SimpleTaskScheduler>>,
        heartbeat_service: Option<mpsc::UnboundedSender<HeartbeatEvent>>,
    ) -> Self {
        let service_impl = AudioClusterServiceImpl::new(
            node_info,
            metadata_store,
            task_scheduler,
            heartbeat_service,
        );

        Self {
            config,
            service_impl,
        }
    }

    /// Start the gRPC server
    pub async fn start(&self) -> Result<(), ClusterError> {
        let addr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| ClusterError::Config(format!("Invalid server address: {}", e)))?;

        info!("Starting gRPC server on {}", addr);

        // Create the service server with configuration
        let service = AudioClusterServiceServer::new(self.service_impl.clone())
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        // Build and start the server
        let result = Server::builder()
            .add_service(service)
            .serve(addr)
            .await;

        if let Err(e) = result {
            error!("gRPC server failed: {}", e);
            return Err(ClusterError::Network(format!("gRPC server error: {}", e)));
        }

        Ok(())
    }

    /// Start the server with graceful shutdown support
    pub async fn start_with_shutdown(
        &self,
        shutdown_signal: impl std::future::Future<Output = ()>,
    ) -> Result<(), ClusterError> {
        let addr = format!("{}:{}", self.config.bind_address, self.config.port)
            .parse()
            .map_err(|e| ClusterError::Config(format!("Invalid server address: {}", e)))?;

        info!("Starting gRPC server on {} with graceful shutdown support", addr);

        // Create the service server with configuration
        let service = AudioClusterServiceServer::new(self.service_impl.clone())
            .max_decoding_message_size(self.config.max_message_size)
            .max_encoding_message_size(self.config.max_message_size);

        // Build and start the server with graceful shutdown
        let result = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, shutdown_signal)
            .await;

        if let Err(e) = result {
            error!("gRPC server failed: {}", e);
            return Err(ClusterError::Network(format!("gRPC server error: {}", e)));
        }

        info!("gRPC server shut down gracefully");
        Ok(())
    }

    /// Get the server configuration
    pub fn config(&self) -> &GrpcServerConfig {
        &self.config
    }

    /// Get the service implementation (for testing or direct access)
    pub fn service(&self) -> &AudioClusterServiceImpl {
        &self.service_impl
    }
}



/// Helper function to create a configured gRPC server
pub fn create_grpc_server(
    config: GrpcServerConfig,
    node_info: ClusterNode,
    metadata_store: Arc<MetadataStore>,
    task_scheduler: Option<Arc<SimpleTaskScheduler>>,
) -> AudioClusterGrpcServer {
    AudioClusterGrpcServer::new(
        config,
        node_info,
        metadata_store,
        task_scheduler,
        None, // heartbeat_service can be added later if needed
    )
}

/// Helper function to create a gRPC server with default configuration
pub fn create_default_grpc_server(
    node_info: ClusterNode,
    metadata_store: Arc<MetadataStore>,
) -> AudioClusterGrpcServer {
    create_grpc_server(
        GrpcServerConfig::default(),
        node_info,
        metadata_store,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    

    #[tokio::test]
    async fn test_grpc_server_creation() {
        let metadata_store = Arc::new(MetadataStore::new_temp().unwrap());
        let node_info = ClusterNode::new(
            "test-node".to_string(),
            "127.0.0.1".to_string(),
            50051,
            8080,
        );

        let server = create_default_grpc_server(node_info, metadata_store);
        assert_eq!(server.config().port, 50051);
        assert_eq!(server.config().bind_address, "0.0.0.0");
    }

    #[test]
    fn test_grpc_server_config_default() {
        let config = GrpcServerConfig::default();
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.port, 50051);
        assert_eq!(config.max_message_size, 4 * 1024 * 1024);
    }
}