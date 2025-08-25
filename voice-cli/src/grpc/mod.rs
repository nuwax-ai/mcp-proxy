pub mod audio_cluster_service;
pub mod client;
pub mod server;
pub mod task_manager;

// Explicit re-exports to avoid conflicts
pub use audio_cluster_service::AudioClusterServiceImpl;
pub use client::{connect_to_cluster_node, AudioClusterClient};
pub use server::{AudioClusterGrpcServer, GrpcServerConfig};
pub use task_manager::{ClusterTaskManager, TaskManagerConfig, TaskManagerStats};

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("audio_cluster");
}
