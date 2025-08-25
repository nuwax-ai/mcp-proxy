use std::sync::Arc;
use voice_cli::cluster::ClusterState;
use voice_cli::models::{ClusterNode, MetadataStore, NodeRole, NodeStatus, TaskMetadata};

#[tokio::test]
async fn test_cluster_state_metadata_store_integration() {
    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());
    
    // Create metadata store with cluster state integration
    let metadata_store = MetadataStore::new_temp_with_cluster_state(cluster_state.clone()).unwrap();
    
    // Create test node
    let node = ClusterNode::new(
        "test-node-1".to_string(),
        "127.0.0.1".to_string(),
        9090,
        8080,
    );
    
    // Add node via metadata store (should update both cluster state and database)
    metadata_store.add_node(&node).await.unwrap();
    
    // Verify node exists in cluster state
    assert!(cluster_state.node_exists("test-node-1"));
    let cluster_node = cluster_state.get_node("test-node-1").unwrap();
    assert_eq!(cluster_node.node_id, "test-node-1");
    assert_eq!(cluster_node.address, "127.0.0.1");
    
    // Update node status via metadata store
    metadata_store.update_node_status("test-node-1", NodeStatus::Healthy).await.unwrap();
    
    // Verify status updated in cluster state
    let updated_node = cluster_state.get_node("test-node-1").unwrap();
    assert_eq!(updated_node.status, NodeStatus::Healthy);
    
    // Create test task
    let task = TaskMetadata::new(
        "task-1".to_string(),
        "client-1".to_string(),
        "test.wav".to_string(),
    );
    
    // Add task via metadata store
    metadata_store.create_task(&task).await.unwrap();
    
    // Verify task exists in cluster state
    assert!(cluster_state.task_exists("task-1"));
    let cluster_task = cluster_state.get_task("task-1").unwrap();
    assert_eq!(cluster_task.task_id, "task-1");
    assert_eq!(cluster_task.client_id, "client-1");
    
    // Assign task via metadata store
    metadata_store.assign_task("task-1", "test-node-1").await.unwrap();
    
    // Verify assignment in cluster state
    let assigned_task = cluster_state.get_task("task-1").unwrap();
    assert_eq!(assigned_task.assigned_node, Some("test-node-1".to_string()));
    
    // Verify task appears in node's task list
    let node_tasks = cluster_state.get_tasks_by_node("test-node-1");
    assert_eq!(node_tasks.len(), 1);
    assert_eq!(node_tasks[0].task_id, "task-1");
    
    // Complete task via metadata store
    metadata_store.complete_task("task-1", 2.5).await.unwrap();
    
    // Verify completion in cluster state
    let completed_task = cluster_state.get_task("task-1").unwrap();
    assert_eq!(completed_task.state, voice_cli::models::TaskState::Completed);
    assert_eq!(completed_task.processing_duration, Some(2.5));
    
    // Verify task no longer in node's active task list
    let active_task_count = cluster_state.get_node_active_task_count("test-node-1");
    assert_eq!(active_task_count, 0);
}

#[tokio::test]
async fn test_atomic_health_monitoring() {
    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());
    
    // Create metadata store with cluster state integration
    let metadata_store = MetadataStore::new_temp_with_cluster_state(cluster_state.clone()).unwrap();
    
    // Create test node
    let mut node = ClusterNode::new(
        "health-test-node".to_string(),
        "127.0.0.1".to_string(),
        9090,
        8080,
    );
    node.status = NodeStatus::Healthy;
    
    // Add node
    metadata_store.add_node(&node).await.unwrap();
    
    // Test atomic health update
    metadata_store.update_node_health_atomic("health-test-node", false).await.unwrap();
    
    // Verify health status updated atomically
    let health_status = metadata_store.get_node_health_atomic("health-test-node");
    assert_eq!(health_status, Some(NodeStatus::Unhealthy));
    
    // Verify in cluster state as well
    let cluster_node = cluster_state.get_node("health-test-node").unwrap();
    assert_eq!(cluster_node.status, NodeStatus::Unhealthy);
    
    // Test recovery
    metadata_store.update_node_health_atomic("health-test-node", true).await.unwrap();
    
    let recovered_status = metadata_store.get_node_health_atomic("health-test-node");
    assert_eq!(recovered_status, Some(NodeStatus::Healthy));
    
    // Get healthy nodes atomically
    let healthy_nodes = metadata_store.get_healthy_nodes_atomic();
    assert_eq!(healthy_nodes.len(), 1);
    assert_eq!(healthy_nodes[0].node_id, "health-test-node");
}

#[tokio::test]
async fn test_cluster_stats_atomic() {
    // Create cluster state
    let cluster_state = Arc::new(ClusterState::new());
    
    // Create metadata store with cluster state integration
    let metadata_store = MetadataStore::new_temp_with_cluster_state(cluster_state.clone()).unwrap();
    
    // Add multiple nodes
    for i in 1..=3 {
        let mut node = ClusterNode::new(
            format!("node-{}", i),
            "127.0.0.1".to_string(),
            9090 + i as u16,
            8080 + i as u16,
        );
        node.status = if i == 1 { NodeStatus::Healthy } else { NodeStatus::Unhealthy };
        node.role = if i == 1 { NodeRole::Leader } else { NodeRole::Follower };
        
        metadata_store.add_node(&node).await.unwrap();
    }
    
    // Add some tasks
    for i in 1..=5 {
        let task = TaskMetadata::new(
            format!("task-{}", i),
            "client-1".to_string(),
            format!("test-{}.wav", i),
        );
        metadata_store.create_task(&task).await.unwrap();
        
        if i <= 2 {
            metadata_store.assign_task(&format!("task-{}", i), "node-1").await.unwrap();
        }
    }
    
    // Get atomic cluster stats
    let stats = metadata_store.get_cluster_stats_atomic().unwrap();
    
    assert_eq!(stats.total_nodes, 3);
    assert_eq!(stats.healthy_nodes, 1);
    assert_eq!(stats.total_tasks, 5);
    assert_eq!(stats.pending_tasks, 3); // 3 unassigned tasks
    assert_eq!(stats.assigned_tasks, 2); // 2 assigned tasks
}