#[cfg(test)]
mod task_scheduler_tests {
    use crate::cluster::{
        task_scheduler::{SchedulerConfig, SimpleTaskScheduler},
        ClusterState,
    };
    use crate::models::{ClusterError, ClusterNode, NodeRole, NodeStatus, TaskState};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::timeout;

    /// Helper function to create a test cluster node
    fn create_test_node(node_id: &str, role: NodeRole, status: NodeStatus) -> ClusterNode {
        ClusterNode {
            node_id: node_id.to_string(),
            address: "127.0.0.1".to_string(),
            grpc_port: 9090,
            http_port: 8080,
            role,
            status,
            last_heartbeat: chrono::Utc::now().timestamp(),
        }
    }

    /// Create a test scheduler with default configuration
    fn create_test_scheduler(
        cluster_state: Arc<ClusterState>,
        leader_can_process: bool,
        leader_node_id: &str,
    ) -> SimpleTaskScheduler {
        let config = SchedulerConfig {
            max_tasks_per_node: 3,
            assignment_timeout: Duration::from_secs(5),
            cache_refresh_interval: Duration::from_secs(10),
            max_queue_size: 100,
            max_retry_attempts: 2,
            retry_delay: Duration::from_secs(1),
        };

        SimpleTaskScheduler::new_with_cluster_state(
            cluster_state,
            leader_can_process,
            leader_node_id.to_string(),
            config,
        )
    }

    #[tokio::test]
    async fn test_round_robin_assignment_followers_only() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add leader (should not process tasks)
        cluster_state.upsert_node(create_test_node(
            "leader",
            NodeRole::Leader,
            NodeStatus::Healthy,
        ));

        // Add followers
        for i in 0..3 {
            cluster_state.upsert_node(create_test_node(
                &format!("follower-{}", i),
                NodeRole::Follower,
                NodeStatus::Healthy,
            ));
        }

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign multiple tasks and verify round-robin distribution
        let mut assigned_nodes = vec![];
        for i in 0..6 {
            let task_id = cluster_state.create_task(
                format!("client-{}", i),
                format!("test-{}.wav", i),
                None,
                None,
            );

            let assigned_node = scheduler.assign_next_task(task_id).await.unwrap();
            assigned_nodes.push(assigned_node);
        }

        // Verify round-robin pattern: all followers should be used evenly
        let mut node_counts = std::collections::HashMap::new();
        for node in &assigned_nodes {
            *node_counts.entry(node.clone()).or_insert(0) += 1;
        }

        // Each follower should get exactly 2 tasks (6 tasks / 3 followers = 2 each)
        assert_eq!(node_counts.len(), 3); // All 3 followers should be used
        for (node_id, count) in node_counts {
            assert!(node_id.starts_with("follower-"));
            assert_eq!(
                count, 2,
                "Node {} should have exactly 2 tasks, got {}",
                node_id, count
            );
        }
    }

    #[tokio::test]
    async fn test_round_robin_assignment_with_leader() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add leader (can process tasks)
        cluster_state.upsert_node(create_test_node(
            "leader",
            NodeRole::Leader,
            NodeStatus::Healthy,
        ));

        // Add followers
        for i in 0..2 {
            cluster_state.upsert_node(create_test_node(
                &format!("follower-{}", i),
                NodeRole::Follower,
                NodeStatus::Healthy,
            ));
        }

        let scheduler = create_test_scheduler(cluster_state.clone(), true, "leader");

        // Assign multiple tasks
        let mut assigned_nodes = vec![];
        for i in 0..6 {
            let task_id = cluster_state.create_task(
                format!("client-{}", i),
                format!("test-{}.wav", i),
                None,
                None,
            );

            let assigned_node = scheduler.assign_next_task(task_id).await.unwrap();
            assigned_nodes.push(assigned_node);
        }

        // Verify round-robin includes leader: all nodes (leader + followers) should be used evenly
        let mut node_counts = std::collections::HashMap::new();
        for node in &assigned_nodes {
            *node_counts.entry(node.clone()).or_insert(0) += 1;
        }

        // All 3 nodes (leader + 2 followers) should be used, each getting 2 tasks
        assert_eq!(node_counts.len(), 3); // All 3 nodes should be used
        assert!(
            node_counts.contains_key("leader"),
            "Leader should process tasks"
        );

        let follower_nodes: Vec<_> = node_counts
            .keys()
            .filter(|k| k.starts_with("follower-"))
            .collect();
        assert_eq!(follower_nodes.len(), 2, "Should have 2 followers");

        for (node_id, count) in node_counts {
            assert_eq!(
                count, 2,
                "Node {} should have exactly 2 tasks, got {}",
                node_id, count
            );
        }
    }

    #[tokio::test]
    async fn test_no_available_nodes() {
        let cluster_state = Arc::new(ClusterState::new());
        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Try to assign task with no nodes
        let task_id =
            cluster_state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);
        let result = scheduler.assign_next_task(task_id).await;

        assert!(matches!(result, Err(ClusterError::NoAvailableNodes)));
    }

    #[tokio::test]
    async fn test_only_unhealthy_nodes() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add unhealthy nodes
        cluster_state.upsert_node(create_test_node(
            "node-1",
            NodeRole::Follower,
            NodeStatus::Unhealthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "node-2",
            NodeRole::Follower,
            NodeStatus::Joining,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Try to assign task
        let task_id =
            cluster_state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);
        let result = scheduler.assign_next_task(task_id).await;

        assert!(matches!(result, Err(ClusterError::NoAvailableNodes)));
    }

    #[tokio::test]
    async fn test_node_capacity_limits() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add one node
        cluster_state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign tasks up to capacity (3 tasks per node in test config)
        let mut task_ids = vec![];
        for i in 0..3 {
            let task_id = cluster_state.create_task(
                format!("client-{}", i),
                format!("test-{}.wav", i),
                None,
                None,
            );
            task_ids.push(task_id.clone());

            let assigned_node = scheduler.assign_next_task(task_id).await.unwrap();
            assert_eq!(assigned_node, "worker");
        }

        // Verify node is at capacity
        assert_eq!(cluster_state.get_node_active_task_count("worker"), 3);

        // Try to assign one more task - should fail
        let overflow_task = cluster_state.create_task(
            "client-overflow".to_string(),
            "overflow.wav".to_string(),
            None,
            None,
        );
        let result = scheduler.assign_next_task(overflow_task).await;
        assert!(matches!(result, Err(ClusterError::NoAvailableNodes)));

        // Complete one task to free up capacity
        cluster_state.complete_task(&task_ids[0], 2.0).unwrap();
        assert_eq!(cluster_state.get_node_active_task_count("worker"), 2);

        // Now assignment should work again
        let new_task =
            cluster_state.create_task("client-new".to_string(), "new.wav".to_string(), None, None);
        let assigned_node = scheduler.assign_next_task(new_task).await.unwrap();
        assert_eq!(assigned_node, "worker");
    }

    #[tokio::test]
    async fn test_mixed_node_statuses() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add nodes with mixed statuses
        cluster_state.upsert_node(create_test_node(
            "healthy-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "unhealthy",
            NodeRole::Follower,
            NodeStatus::Unhealthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "healthy-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "joining",
            NodeRole::Follower,
            NodeStatus::Joining,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign multiple tasks
        let mut assigned_nodes = vec![];
        for i in 0..4 {
            let task_id = cluster_state.create_task(
                format!("client-{}", i),
                format!("test-{}.wav", i),
                None,
                None,
            );

            let assigned_node = scheduler.assign_next_task(task_id).await.unwrap();
            assigned_nodes.push(assigned_node);
        }

        // Should only assign to healthy nodes
        let mut healthy_node_assignments = std::collections::HashMap::new();
        for node in &assigned_nodes {
            assert!(
                node == "healthy-1" || node == "healthy-2",
                "Task assigned to unhealthy node: {}",
                node
            );
            *healthy_node_assignments.entry(node.clone()).or_insert(0) += 1;
        }

        // Should follow round-robin between healthy nodes - both should get 2 tasks each
        assert_eq!(healthy_node_assignments.len(), 2); // Both healthy nodes used
        assert_eq!(healthy_node_assignments["healthy-1"], 2);
        assert_eq!(healthy_node_assignments["healthy-2"], 2);
    }

    #[tokio::test]
    async fn test_node_becomes_unhealthy_during_assignment() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add healthy nodes
        cluster_state.upsert_node(create_test_node(
            "node-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "node-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign first task - should go to one of the healthy nodes
        let task1 =
            cluster_state.create_task("client-1".to_string(), "test1.wav".to_string(), None, None);
        let assigned1 = scheduler.assign_next_task(task1).await.unwrap();
        assert!(
            assigned1 == "node-1" || assigned1 == "node-2",
            "Task assigned to unexpected node: {}",
            assigned1
        );

        // Make one node unhealthy (the other one should still be healthy)
        let unhealthy_node = if assigned1 == "node-1" {
            "node-1"
        } else {
            "node-2"
        };
        let healthy_node = if assigned1 == "node-1" {
            "node-2"
        } else {
            "node-1"
        };
        cluster_state
            .update_node_status(unhealthy_node, NodeStatus::Unhealthy)
            .unwrap();

        // Assign second task - should go to the remaining healthy node
        let task2 =
            cluster_state.create_task("client-2".to_string(), "test2.wav".to_string(), None, None);
        let assigned2 = scheduler.assign_next_task(task2).await.unwrap();
        assert_eq!(assigned2, healthy_node);

        // Assign third task - should still go to the healthy node
        let task3 =
            cluster_state.create_task("client-3".to_string(), "test3.wav".to_string(), None, None);
        let assigned3 = scheduler.assign_next_task(task3).await.unwrap();
        assert_eq!(assigned3, healthy_node);
    }

    #[tokio::test]
    async fn test_should_leader_process_logic() {
        let cluster_state = Arc::new(ClusterState::new());

        // Test with leader processing enabled
        let scheduler_with_leader =
            create_test_scheduler(cluster_state.clone(), true, "leader-node");
        assert!(scheduler_with_leader.should_leader_process("leader-node"));
        assert!(!scheduler_with_leader.should_leader_process("follower-node"));

        // Test with leader processing disabled
        let scheduler_no_leader =
            create_test_scheduler(cluster_state.clone(), false, "leader-node");
        assert!(!scheduler_no_leader.should_leader_process("leader-node"));
        assert!(!scheduler_no_leader.should_leader_process("follower-node"));
    }

    #[tokio::test]
    async fn test_task_assignment_with_existing_tasks() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add nodes
        cluster_state.upsert_node(create_test_node(
            "node-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        cluster_state.upsert_node(create_test_node(
            "node-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        // Manually assign some tasks to create uneven distribution
        let existing_task1 = cluster_state.create_task(
            "existing-1".to_string(),
            "existing1.wav".to_string(),
            None,
            None,
        );
        let existing_task2 = cluster_state.create_task(
            "existing-2".to_string(),
            "existing2.wav".to_string(),
            None,
            None,
        );

        cluster_state
            .assign_task(&existing_task1, "node-1")
            .unwrap();
        cluster_state
            .assign_task(&existing_task2, "node-1")
            .unwrap();

        // Now node-1 has 2 tasks, node-2 has 0 tasks
        assert_eq!(cluster_state.get_node_active_task_count("node-1"), 2);
        assert_eq!(cluster_state.get_node_active_task_count("node-2"), 0);

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign new task - round-robin should still work, but capacity matters
        let new_task =
            cluster_state.create_task("new-client".to_string(), "new.wav".to_string(), None, None);
        let assigned_node = scheduler.assign_next_task(new_task).await.unwrap();

        // Should assign to first available node in round-robin order
        // Since both nodes are under capacity, it should follow round-robin
        assert!(assigned_node == "node-1" || assigned_node == "node-2");
    }

    #[tokio::test]
    async fn test_concurrent_task_assignments() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add nodes
        for i in 0..3 {
            cluster_state.upsert_node(create_test_node(
                &format!("node-{}", i),
                NodeRole::Follower,
                NodeStatus::Healthy,
            ));
        }

        let scheduler = Arc::new(create_test_scheduler(
            cluster_state.clone(),
            false,
            "leader",
        ));

        // Concurrently assign multiple tasks (9 tasks for 3 nodes with capacity 3 each)
        let mut handles = vec![];
        for i in 0..9 {
            let scheduler_clone = Arc::clone(&scheduler);
            let cluster_state_clone = Arc::clone(&cluster_state);

            let handle = tokio::spawn(async move {
                let task_id = cluster_state_clone.create_task(
                    format!("client-{}", i),
                    format!("test-{}.wav", i),
                    None,
                    None,
                );

                scheduler_clone.assign_next_task(task_id).await
            });
            handles.push(handle);
        }

        // Wait for all assignments to complete
        let mut results = vec![];
        for handle in handles {
            let result = handle.await.unwrap();
            results.push(result);
        }

        // All assignments should succeed
        assert_eq!(results.len(), 9);
        for (i, result) in results.iter().enumerate() {
            if result.is_err() {
                eprintln!("Task {} failed: {:?}", i, result);
            }
            assert!(result.is_ok(), "Task {} failed: {:?}", i, result);
        }

        // Verify tasks were distributed across nodes
        let node_counts: Vec<usize> = (0..3)
            .map(|i| cluster_state.get_node_active_task_count(&format!("node-{}", i)))
            .collect();

        // Each node should have some tasks (not perfectly balanced due to concurrency)
        let total_assigned: usize = node_counts.iter().sum();
        assert_eq!(total_assigned, 9);

        // No node should be overloaded (max 3 tasks per node in test config)
        for count in node_counts {
            assert!(count <= 3);
        }
    }

    #[tokio::test]
    async fn test_scheduler_stats_tracking() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add a node
        cluster_state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Get initial stats
        let initial_stats = scheduler.get_stats_direct().await;
        assert_eq!(initial_stats.total_scheduled, 0);
        assert_eq!(initial_stats.completed_tasks, 0);
        assert_eq!(initial_stats.failed_tasks, 0);

        // Schedule a task through the scheduler's external API
        let task_id = scheduler
            .schedule_task(
                "client-1".to_string(),
                "test.wav".to_string(),
                "/path/to/test.wav".to_string(),
                Some("whisper-1".to_string()),
                Some("json".to_string()),
            )
            .await
            .unwrap();

        // Note: Since we're not running the event loop, stats won't be updated
        // This test verifies the API works, but full stats testing requires integration tests
        assert!(!task_id.is_empty());
    }

    #[tokio::test]
    async fn test_assignment_timeout() {
        let cluster_state = Arc::new(ClusterState::new());
        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Try to assign task with no available nodes
        let task_id =
            cluster_state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);

        // Assignment should fail quickly (not timeout) since there are no nodes
        let start = std::time::Instant::now();
        let result = scheduler.assign_next_task(task_id).await;
        let duration = start.elapsed();

        assert!(matches!(result, Err(ClusterError::NoAvailableNodes)));
        assert!(duration < Duration::from_secs(1)); // Should fail immediately
    }

    #[tokio::test]
    async fn test_node_addition_during_operation() {
        let cluster_state = Arc::new(ClusterState::new());

        // Start with one node
        cluster_state.upsert_node(create_test_node(
            "node-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        // Assign first task
        let task1 =
            cluster_state.create_task("client-1".to_string(), "test1.wav".to_string(), None, None);
        let assigned1 = scheduler.assign_next_task(task1).await.unwrap();
        assert_eq!(assigned1, "node-1"); // Only one node available

        // Add another node
        cluster_state.upsert_node(create_test_node(
            "node-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        // Assign second task - could go to either node due to round-robin
        let task2 =
            cluster_state.create_task("client-2".to_string(), "test2.wav".to_string(), None, None);
        let assigned2 = scheduler.assign_next_task(task2).await.unwrap();
        assert!(
            assigned2 == "node-1" || assigned2 == "node-2",
            "Task assigned to unexpected node: {}",
            assigned2
        );

        // Assign third task - should go to the other node (round-robin)
        let task3 =
            cluster_state.create_task("client-3".to_string(), "test3.wav".to_string(), None, None);
        let assigned3 = scheduler.assign_next_task(task3).await.unwrap();
        assert!(
            assigned3 == "node-1" || assigned3 == "node-2",
            "Task assigned to unexpected node: {}",
            assigned3
        );

        // Verify both nodes are being used
        let node1_tasks = cluster_state.get_node_active_task_count("node-1");
        let node2_tasks = cluster_state.get_node_active_task_count("node-2");
        assert_eq!(node1_tasks + node2_tasks, 3); // Total 3 tasks assigned
        assert!(
            node1_tasks > 0 && node2_tasks > 0,
            "Both nodes should have tasks"
        );
    }

    #[test]
    fn test_scheduler_config_defaults() {
        let config = SchedulerConfig::default();

        assert_eq!(config.max_tasks_per_node, 5);
        assert_eq!(config.assignment_timeout, Duration::from_secs(30));
        assert_eq!(config.cache_refresh_interval, Duration::from_secs(10));
        assert_eq!(config.max_queue_size, 1000);
        assert_eq!(config.max_retry_attempts, 3);
        assert_eq!(config.retry_delay, Duration::from_secs(2));
    }

    #[tokio::test]
    async fn test_cluster_state_integration() {
        let cluster_state = Arc::new(ClusterState::new());

        // Add nodes through scheduler
        let scheduler = create_test_scheduler(cluster_state.clone(), false, "leader");

        let node1 = create_test_node("node-1", NodeRole::Follower, NodeStatus::Healthy);
        let node2 = create_test_node("node-2", NodeRole::Follower, NodeStatus::Healthy);

        scheduler.add_node(node1);
        scheduler.add_node(node2);

        // Verify nodes were added to cluster state
        assert_eq!(cluster_state.get_all_nodes().len(), 2);
        assert!(cluster_state.node_exists("node-1"));
        assert!(cluster_state.node_exists("node-2"));

        // Update node status through scheduler
        scheduler
            .update_node_status("node-1", NodeStatus::Unhealthy)
            .unwrap();

        let updated_node = cluster_state.get_node("node-1").unwrap();
        assert_eq!(updated_node.status, NodeStatus::Unhealthy);

        // Remove node through scheduler
        let removed = scheduler.remove_node("node-2");
        assert!(removed.is_some());
        assert!(!cluster_state.node_exists("node-2"));
    }
}
