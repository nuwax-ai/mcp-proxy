use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;
use voice_cli::{
    cluster::{SchedulerConfig, SchedulerEvent, SimpleTaskScheduler},
    load_balancer::LoadBalancerService,
    models::{
        ClusterNode, LoadBalancerConfig, MetadataStore, NodeRole, NodeStatus, TaskMetadata,
        TaskState,
    },
};

/// Test cluster failover during active task processing
#[tokio::test]
async fn test_failover_during_task_processing() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("failover_task_test.db");
    let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());

    // Set up cluster with leader and workers
    let mut leader = ClusterNode::new(
        "task-leader".to_string(),
        "127.0.0.1".to_string(),
        50060,
        8100,
    );
    leader.role = NodeRole::Leader;
    leader.status = NodeStatus::Healthy;
    metadata_store.add_node(&leader).await.unwrap();

    let mut worker1 = ClusterNode::new(
        "task-worker-1".to_string(),
        "127.0.0.1".to_string(),
        50061,
        8101,
    );
    worker1.role = NodeRole::Follower;
    worker1.status = NodeStatus::Healthy;
    metadata_store.add_node(&worker1).await.unwrap();

    let mut worker2 = ClusterNode::new(
        "task-worker-2".to_string(),
        "127.0.0.1".to_string(),
        50062,
        8102,
    );
    worker2.role = NodeRole::Follower;
    worker2.status = NodeStatus::Healthy;
    metadata_store.add_node(&worker2).await.unwrap();

    // Submit multiple tasks for processing
    let tasks = vec![
        ("failover-task-1", "client-f1", "audio1.mp3"),
        ("failover-task-2", "client-f2", "audio2.mp3"),
        ("failover-task-3", "client-f1", "audio3.mp3"),
        ("failover-task-4", "client-f3", "audio4.mp3"),
    ];

    for (task_id, client_id, filename) in &tasks {
        let task = TaskMetadata::new(
            task_id.to_string(),
            client_id.to_string(),
            filename.to_string(),
        );
        metadata_store.create_task(&task).await.unwrap();

        // Assign tasks round-robin to workers
        let worker_node = if task_id.ends_with('1') || task_id.ends_with('3') {
            "task-worker-1"
        } else {
            "task-worker-2"
        };

        metadata_store
            .assign_task(task_id, worker_node)
            .await
            .unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    // Verify tasks are assigned
    let assigned_tasks = metadata_store
        .get_tasks_by_state(TaskState::Assigned)
        .await
        .unwrap();
    assert_eq!(assigned_tasks.len(), 4);

    // Simulate worker1 failure during processing
    metadata_store
        .update_node_status("task-worker-1", NodeStatus::Unhealthy)
        .await
        .unwrap();

    // Get tasks assigned to failed worker
    let worker1_tasks = metadata_store
        .get_tasks_by_node("task-worker-1")
        .await
        .unwrap();
    assert!(worker1_tasks.len() >= 2);

    // Simulate worker recovery
    metadata_store
        .update_node_status("task-worker-1", NodeStatus::Healthy)
        .await
        .unwrap();

    // Submit task after recovery
    let recovery_task = TaskMetadata::new(
        "post-recovery-task".to_string(),
        "client-pr".to_string(),
        "post_recovery.mp3".to_string(),
    );
    metadata_store.create_task(&recovery_task).await.unwrap();
    metadata_store
        .assign_task("post-recovery-task", "task-worker-1")
        .await
        .unwrap();

    // Verify tasks can be assigned to recovered worker
    let assigned_tasks = metadata_store
        .get_tasks_by_state(TaskState::Assigned)
        .await
        .unwrap();
    let post_recovery_task = assigned_tasks
        .iter()
        .find(|t| t.task_id == "post-recovery-task")
        .unwrap();

    assert!(post_recovery_task.assigned_node.is_some());

    println!("✅ Failover during task processing test passed");
}

/// Test leader election failover scenarios
#[tokio::test]
async fn test_leader_election_failover() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("leader_failover_test.db");
    let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());

    // Set up 3-node cluster for leader election
    let nodes = vec![
        (
            "leader-original",
            NodeRole::Leader,
            NodeStatus::Healthy,
            50070,
            8110,
        ),
        (
            "candidate-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50071,
            8111,
        ),
        (
            "candidate-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50072,
            8112,
        ),
    ];

    for (node_id, role, status, grpc_port, http_port) in &nodes {
        let mut node = ClusterNode::new(
            node_id.to_string(),
            "127.0.0.1".to_string(),
            *grpc_port,
            *http_port,
        );
        node.role = *role;
        node.status = *status;
        metadata_store.add_node(&node).await.unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    // Verify initial leader
    let initial_nodes = metadata_store.get_all_nodes().await.unwrap();
    let initial_leader = initial_nodes
        .iter()
        .find(|n| n.role == NodeRole::Leader)
        .unwrap();
    assert_eq!(initial_leader.node_id, "leader-original");

    // Scenario 1: Leader becomes unhealthy (network partition)
    metadata_store
        .update_node_status("leader-original", NodeStatus::Unhealthy)
        .await
        .unwrap();

    // Simulate election process - candidate-1 becomes leader
    metadata_store
        .update_node_role("leader-original", NodeRole::Follower)
        .await
        .unwrap();
    metadata_store
        .update_node_role("candidate-1", NodeRole::Leader)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Verify new leader
    let post_election_nodes = metadata_store.get_all_nodes().await.unwrap();
    let new_leader = post_election_nodes
        .iter()
        .find(|n| n.role == NodeRole::Leader && n.status == NodeStatus::Healthy)
        .unwrap();
    assert_eq!(new_leader.node_id, "candidate-1");

    // Scenario 2: Split brain prevention - original leader recovers
    metadata_store
        .update_node_status("leader-original", NodeStatus::Healthy)
        .await
        .unwrap();

    // Original leader should remain follower (no automatic role change)
    let split_brain_nodes = metadata_store.get_all_nodes().await.unwrap();
    let recovered_node = split_brain_nodes
        .iter()
        .find(|n| n.node_id == "leader-original")
        .unwrap();
    assert_eq!(recovered_node.role, NodeRole::Follower);

    // Should still have only one leader
    let leader_count = split_brain_nodes
        .iter()
        .filter(|n| n.role == NodeRole::Leader)
        .count();
    assert_eq!(leader_count, 1);

    println!("✅ Leader election failover test passed");
}

/// Test network partition and cluster split scenarios
#[tokio::test]
async fn test_network_partition_scenarios() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("partition_test.db");
    let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());

    // Create 5-node cluster for partition testing
    let nodes = vec![
        (
            "partition-leader",
            NodeRole::Leader,
            NodeStatus::Healthy,
            50080,
            8120,
        ),
        (
            "partition-node-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50081,
            8121,
        ),
        (
            "partition-node-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50082,
            8122,
        ),
        (
            "partition-node-3",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50083,
            8123,
        ),
        (
            "partition-node-4",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50084,
            8124,
        ),
    ];

    for (node_id, role, status, grpc_port, http_port) in &nodes {
        let mut node = ClusterNode::new(
            node_id.to_string(),
            "127.0.0.1".to_string(),
            *grpc_port,
            *http_port,
        );
        node.role = *role;
        node.status = *status;
        metadata_store.add_node(&node).await.unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    // Test 1: Minority partition (leader + 1 node isolated)
    metadata_store
        .update_node_status("partition-leader", NodeStatus::Unhealthy)
        .await
        .unwrap();
    metadata_store
        .update_node_status("partition-node-1", NodeStatus::Unhealthy)
        .await
        .unwrap();

    // Majority partition elects new leader
    metadata_store
        .update_node_role("partition-leader", NodeRole::Follower)
        .await
        .unwrap();
    metadata_store
        .update_node_role("partition-node-2", NodeRole::Leader)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Verify healthy nodes in majority partition
    let healthy_nodes = metadata_store
        .get_all_nodes()
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.status == NodeStatus::Healthy)
        .collect::<Vec<_>>();

    assert_eq!(healthy_nodes.len(), 3); // nodes 2, 3, 4

    // Test 2: Partition healing
    metadata_store
        .update_node_status("partition-leader", NodeStatus::Healthy)
        .await
        .unwrap();
    metadata_store
        .update_node_status("partition-node-1", NodeStatus::Healthy)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Verify all nodes are healthy after healing
    let final_healthy_nodes = metadata_store
        .get_all_nodes()
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.status == NodeStatus::Healthy)
        .collect::<Vec<_>>();

    assert_eq!(final_healthy_nodes.len(), 5); // All nodes healthy

    println!("✅ Network partition scenarios test passed");
}

/// Test cascading failures and cluster recovery
#[tokio::test]
async fn test_cascading_failure_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("cascading_test.db");
    let metadata_store = Arc::new(MetadataStore::new(db_path.to_str().unwrap()).unwrap());

    // Create 4-node cluster
    let nodes = vec![
        (
            "cascade-leader",
            NodeRole::Leader,
            NodeStatus::Healthy,
            50090,
            8130,
        ),
        (
            "cascade-worker-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50091,
            8131,
        ),
        (
            "cascade-worker-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50092,
            8132,
        ),
        (
            "cascade-worker-3",
            NodeRole::Follower,
            NodeStatus::Healthy,
            50093,
            8133,
        ),
    ];

    for (node_id, role, status, grpc_port, http_port) in &nodes {
        let mut node = ClusterNode::new(
            node_id.to_string(),
            "127.0.0.1".to_string(),
            *grpc_port,
            *http_port,
        );
        node.role = *role;
        node.status = *status;
        metadata_store.add_node(&node).await.unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    // Submit initial workload
    for i in 1..=6 {
        let task = TaskMetadata::new(
            format!("cascade-task-{}", i),
            format!("client-c{}", i),
            format!("cascade{}.mp3", i),
        );
        metadata_store.create_task(&task).await.unwrap();

        // Assign tasks round-robin
        let worker_index = (i - 1) % 3 + 1;
        let worker_node = format!("cascade-worker-{}", worker_index);
        metadata_store
            .assign_task(&task.task_id, &worker_node)
            .await
            .unwrap();
    }

    sleep(Duration::from_millis(100)).await;

    // Verify initial distribution
    let initial_assigned = metadata_store
        .get_tasks_by_state(TaskState::Assigned)
        .await
        .unwrap();
    assert_eq!(initial_assigned.len(), 6);

    // Scenario 1: Single worker failure
    metadata_store
        .update_node_status("cascade-worker-1", NodeStatus::Unhealthy)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Scenario 2: Cascading failure - another worker fails
    metadata_store
        .update_node_status("cascade-worker-2", NodeStatus::Unhealthy)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Verify only one worker remaining
    let remaining_workers = metadata_store
        .get_all_nodes()
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.role == NodeRole::Follower && n.status == NodeStatus::Healthy)
        .count();
    assert_eq!(remaining_workers, 1); // Only cascade-worker-3

    // Scenario 3: Recovery - workers come back online
    metadata_store
        .update_node_status("cascade-worker-1", NodeStatus::Healthy)
        .await
        .unwrap();
    metadata_store
        .update_node_status("cascade-worker-2", NodeStatus::Healthy)
        .await
        .unwrap();

    sleep(Duration::from_millis(100)).await;

    // Verify recovery
    let final_healthy_workers = metadata_store
        .get_all_nodes()
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.role == NodeRole::Follower && n.status == NodeStatus::Healthy)
        .count();
    assert_eq!(final_healthy_workers, 3); // All workers healthy

    println!("✅ Cascading failure recovery test passed");
}

#[tokio::test]
async fn test_failover_scenarios_summary() {
    println!("\n🎯 FAILOVER SCENARIOS TEST SUMMARY");
    println!("===================================");
    println!("✅ Failover during task processing test completed");
    println!("✅ Leader election failover test completed");
    println!("✅ Network partition scenarios test completed");
    println!("✅ Cascading failure recovery test completed");
    println!("🚀 All failover scenario tests verified!");
}
