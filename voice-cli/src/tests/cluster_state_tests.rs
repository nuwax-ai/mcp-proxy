#[cfg(test)]
mod cluster_state_tests {
    use crate::cluster::ClusterState;
    use crate::models::{ClusterError, ClusterNode, NodeRole, NodeStatus, TaskMetadata, TaskState};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use uuid::Uuid;

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

    /// Helper function to create a test task
    fn create_test_task(task_id: &str, client_id: &str, filename: &str) -> TaskMetadata {
        TaskMetadata::new(
            task_id.to_string(),
            client_id.to_string(),
            filename.to_string(),
        )
    }

    #[test]
    fn test_cluster_state_creation() {
        let state = ClusterState::new();

        // Verify initial state
        assert_eq!(state.get_all_nodes().len(), 0);
        assert_eq!(state.get_all_tasks().len(), 0);

        let stats = state.get_stats();
        assert_eq!(stats.total_nodes, 0);
        assert_eq!(stats.healthy_nodes, 0);
        assert_eq!(stats.total_tasks, 0);
    }

    #[test]
    fn test_node_upsert_operations() {
        let state = ClusterState::new();

        // Test inserting new node
        let node1 = create_test_node("node-1", NodeRole::Leader, NodeStatus::Healthy);
        state.upsert_node(node1.clone());

        assert!(state.node_exists("node-1"));
        assert_eq!(state.get_all_nodes().len(), 1);

        let retrieved = state.get_node("node-1").unwrap();
        assert_eq!(retrieved.node_id, "node-1");
        assert_eq!(retrieved.role, NodeRole::Leader);
        assert_eq!(retrieved.status, NodeStatus::Healthy);

        // Test updating existing node
        let mut updated_node = node1.clone();
        updated_node.status = NodeStatus::Unhealthy;
        state.upsert_node(updated_node);

        let retrieved_updated = state.get_node("node-1").unwrap();
        assert_eq!(retrieved_updated.status, NodeStatus::Unhealthy);
        assert_eq!(state.get_all_nodes().len(), 1); // Still only one node
    }

    #[test]
    fn test_node_removal() {
        let state = ClusterState::new();

        // Add a node
        let node = create_test_node("node-1", NodeRole::Follower, NodeStatus::Healthy);
        state.upsert_node(node.clone());

        // Add a task assigned to this node
        let task = create_test_task("task-1", "client-1", "test.wav");
        state.upsert_task(task);
        state.assign_task("task-1", "node-1").unwrap();

        // Verify task is assigned
        let assigned_task = state.get_task("task-1").unwrap();
        assert_eq!(assigned_task.assigned_node, Some("node-1".to_string()));

        // Remove the node
        let removed = state.remove_node("node-1").unwrap();
        assert_eq!(removed.node_id, "node-1");
        assert!(!state.node_exists("node-1"));

        // Verify task assignment was cleared
        let unassigned_task = state.get_task("task-1").unwrap();
        assert_eq!(unassigned_task.assigned_node, None);
    }

    #[test]
    fn test_node_status_update() {
        let state = ClusterState::new();

        let node = create_test_node("node-1", NodeRole::Follower, NodeStatus::Joining);
        state.upsert_node(node);

        // Update status
        state
            .update_node_status("node-1", NodeStatus::Healthy)
            .unwrap();

        let updated = state.get_node("node-1").unwrap();
        assert_eq!(updated.status, NodeStatus::Healthy);

        // Test updating non-existent node
        let result = state.update_node_status("non-existent", NodeStatus::Healthy);
        assert!(matches!(result, Err(ClusterError::NodeNotFound(_))));
    }

    #[test]
    fn test_nodes_by_role_and_status() {
        let state = ClusterState::new();

        // Add nodes with different roles and statuses
        state.upsert_node(create_test_node(
            "leader-1",
            NodeRole::Leader,
            NodeStatus::Healthy,
        ));
        state.upsert_node(create_test_node(
            "follower-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        state.upsert_node(create_test_node(
            "follower-2",
            NodeRole::Follower,
            NodeStatus::Unhealthy,
        ));
        state.upsert_node(create_test_node(
            "candidate-1",
            NodeRole::Candidate,
            NodeStatus::Joining,
        ));

        // Test filtering by role
        let leaders = state.get_nodes_by_role(&NodeRole::Leader);
        assert_eq!(leaders.len(), 1);
        assert_eq!(leaders[0].node_id, "leader-1");

        let followers = state.get_nodes_by_role(&NodeRole::Follower);
        assert_eq!(followers.len(), 2);

        // Test filtering by status
        let healthy_nodes = state.get_nodes_by_status(&NodeStatus::Healthy);
        assert_eq!(healthy_nodes.len(), 2);

        let unhealthy_nodes = state.get_nodes_by_status(&NodeStatus::Unhealthy);
        assert_eq!(unhealthy_nodes.len(), 1);
        assert_eq!(unhealthy_nodes[0].node_id, "follower-2");
    }

    #[test]
    fn test_task_operations() {
        let state = ClusterState::new();

        // Test creating task
        let task_id = state.create_task(
            "client-1".to_string(),
            "test.wav".to_string(),
            Some("whisper-1".to_string()),
            Some("json".to_string()),
        );

        assert!(state.task_exists(&task_id));

        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.client_id, "client-1");
        assert_eq!(task.filename, "test.wav");
        assert_eq!(task.model, Some("whisper-1".to_string()));
        assert_eq!(task.response_format, Some("json".to_string()));
        assert_eq!(task.state, TaskState::Pending);

        // Test updating task state
        state
            .update_task_state(&task_id, TaskState::Processing)
            .unwrap();
        let updated = state.get_task(&task_id).unwrap();
        assert_eq!(updated.state, TaskState::Processing);

        // Test completing task
        state.complete_task(&task_id, 2.5).unwrap();
        let completed = state.get_task(&task_id).unwrap();
        assert_eq!(completed.state, TaskState::Completed);
        assert_eq!(completed.processing_duration, Some(2.5));
        assert!(completed.completed_at.is_some());

        // Test failing task
        let task_id2 =
            state.create_task("client-2".to_string(), "test2.wav".to_string(), None, None);
        state.fail_task(&task_id2, "Processing failed").unwrap();
        let failed = state.get_task(&task_id2).unwrap();
        assert_eq!(failed.state, TaskState::Failed);
        assert_eq!(failed.error_message, Some("Processing failed".to_string()));
    }

    #[test]
    fn test_task_assignment() {
        let state = ClusterState::new();

        // Add a node
        let node = create_test_node("node-1", NodeRole::Follower, NodeStatus::Healthy);
        state.upsert_node(node);

        // Create a task
        let task_id = state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);

        // Assign task to node
        state.assign_task(&task_id, "node-1").unwrap();

        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.assigned_node, Some("node-1".to_string()));
        assert_eq!(task.state, TaskState::Assigned);

        // Verify task appears in node's task list
        let node_tasks = state.get_tasks_by_node("node-1");
        assert_eq!(node_tasks.len(), 1);
        assert_eq!(node_tasks[0].task_id, task_id);

        // Test active task count
        assert_eq!(state.get_node_active_task_count("node-1"), 1);

        // Test assigning to non-existent node
        let result = state.assign_task(&task_id, "non-existent");
        assert!(matches!(result, Err(ClusterError::NodeNotFound(_))));

        // Test assigning non-existent task
        let result = state.assign_task("non-existent", "node-1");
        assert!(matches!(result, Err(ClusterError::TaskNotFound(_))));
    }

    #[test]
    fn test_task_reassignment() {
        let state = ClusterState::new();

        // Add two nodes
        state.upsert_node(create_test_node(
            "node-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        state.upsert_node(create_test_node(
            "node-2",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        // Create and assign task to first node
        let task_id = state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);
        state.assign_task(&task_id, "node-1").unwrap();

        assert_eq!(state.get_node_active_task_count("node-1"), 1);
        assert_eq!(state.get_node_active_task_count("node-2"), 0);

        // Reassign to second node
        state.assign_task(&task_id, "node-2").unwrap();

        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.assigned_node, Some("node-2".to_string()));

        // Verify task counts updated
        assert_eq!(state.get_node_active_task_count("node-1"), 0);
        assert_eq!(state.get_node_active_task_count("node-2"), 1);

        // Verify task lists updated
        assert_eq!(state.get_tasks_by_node("node-1").len(), 0);
        assert_eq!(state.get_tasks_by_node("node-2").len(), 1);
    }

    #[test]
    fn test_tasks_by_state() {
        let state = ClusterState::new();

        // Create tasks in different states
        let task1 = state.create_task("client-1".to_string(), "test1.wav".to_string(), None, None);
        let task2 = state.create_task("client-2".to_string(), "test2.wav".to_string(), None, None);
        let task3 = state.create_task("client-3".to_string(), "test3.wav".to_string(), None, None);

        // Update states
        state
            .update_task_state(&task2, TaskState::Processing)
            .unwrap();
        state.complete_task(&task3, 1.5).unwrap();

        // Test filtering by state
        let pending = state.get_tasks_by_state(&TaskState::Pending);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, task1);

        let processing = state.get_tasks_by_state(&TaskState::Processing);
        assert_eq!(processing.len(), 1);
        assert_eq!(processing[0].task_id, task2);

        let completed = state.get_tasks_by_state(&TaskState::Completed);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].task_id, task3);

        let failed = state.get_tasks_by_state(&TaskState::Failed);
        assert_eq!(failed.len(), 0);
    }

    #[test]
    fn test_cluster_stats() {
        let state = ClusterState::new();

        // Add nodes
        state.upsert_node(create_test_node(
            "leader",
            NodeRole::Leader,
            NodeStatus::Healthy,
        ));
        state.upsert_node(create_test_node(
            "follower-1",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        state.upsert_node(create_test_node(
            "follower-2",
            NodeRole::Follower,
            NodeStatus::Unhealthy,
        ));

        // Add tasks
        let task1 = state.create_task("client-1".to_string(), "test1.wav".to_string(), None, None);
        let task2 = state.create_task("client-2".to_string(), "test2.wav".to_string(), None, None);
        let task3 = state.create_task("client-3".to_string(), "test3.wav".to_string(), None, None);

        // Assign and update tasks
        state.assign_task(&task1, "leader").unwrap();
        state
            .update_task_state(&task2, TaskState::Processing)
            .unwrap();
        state.complete_task(&task3, 2.0).unwrap();

        let stats = state.get_stats();
        assert_eq!(stats.total_nodes, 3);
        assert_eq!(stats.healthy_nodes, 2);
        assert_eq!(stats.total_tasks, 3);
        assert_eq!(stats.assigned_tasks, 1);
        assert_eq!(stats.processing_tasks, 1);
        assert_eq!(stats.completed_tasks, 1);
        assert_eq!(stats.failed_tasks, 0);
    }

    #[tokio::test]
    async fn test_concurrent_node_operations() {
        let state = Arc::new(ClusterState::new());

        // Spawn multiple tasks that add nodes concurrently
        let mut handles = vec![];
        for i in 0..10 {
            let state_clone = Arc::clone(&state);
            let handle = tokio::spawn(async move {
                let node = create_test_node(
                    &format!("node-{}", i),
                    NodeRole::Follower,
                    NodeStatus::Healthy,
                );
                state_clone.upsert_node(node);
            });
            handles.push(handle);
        }

        // Wait for all operations to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all nodes were added
        assert_eq!(state.get_all_nodes().len(), 10);

        // Verify we can retrieve each node
        for i in 0..10 {
            assert!(state.node_exists(&format!("node-{}", i)));
        }
    }

    #[tokio::test]
    async fn test_concurrent_task_operations() {
        let state = Arc::new(ClusterState::new());

        // Add a node first
        state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));

        // Spawn multiple tasks that create and assign tasks concurrently
        let mut handles = vec![];
        for i in 0..20 {
            let state_clone = Arc::clone(&state);
            let handle = tokio::spawn(async move {
                let task_id = state_clone.create_task(
                    format!("client-{}", i),
                    format!("test-{}.wav", i),
                    None,
                    None,
                );

                // Try to assign task (some may fail due to concurrent access)
                let _ = state_clone.assign_task(&task_id, "worker");

                task_id
            });
            handles.push(handle);
        }

        // Wait for all operations to complete
        let mut task_ids = vec![];
        for handle in handles {
            let task_id = handle.await.unwrap();
            task_ids.push(task_id);
        }

        // Verify all tasks were created
        assert_eq!(state.get_all_tasks().len(), 20);

        // Verify we can retrieve each task
        for task_id in task_ids {
            assert!(state.task_exists(&task_id));
        }
    }

    #[tokio::test]
    async fn test_concurrent_status_updates() {
        let state = Arc::new(ClusterState::new());

        // Add nodes
        for i in 0..5 {
            state.upsert_node(create_test_node(
                &format!("node-{}", i),
                NodeRole::Follower,
                NodeStatus::Joining,
            ));
        }

        // Concurrently update node statuses
        let mut handles = vec![];
        for i in 0..5 {
            let state_clone = Arc::clone(&state);
            let handle = tokio::spawn(async move {
                // Simulate multiple status updates
                for status in [
                    NodeStatus::Healthy,
                    NodeStatus::Unhealthy,
                    NodeStatus::Healthy,
                ] {
                    state_clone
                        .update_node_status(&format!("node-{}", i), status)
                        .unwrap();
                    sleep(Duration::from_millis(1)).await;
                }
            });
            handles.push(handle);
        }

        // Wait for all updates to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify final state - all nodes should be healthy
        let healthy_nodes = state.get_nodes_by_status(&NodeStatus::Healthy);
        assert_eq!(healthy_nodes.len(), 5);
    }

    #[test]
    fn test_task_completion_cleanup() {
        let state = ClusterState::new();

        // Add node and create task
        state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        let task_id = state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);

        // Assign and verify active count
        state.assign_task(&task_id, "worker").unwrap();
        assert_eq!(state.get_node_active_task_count("worker"), 1);

        // Complete task and verify cleanup
        state.complete_task(&task_id, 3.0).unwrap();
        assert_eq!(state.get_node_active_task_count("worker"), 0);

        // Task should still exist but not be in node's active list
        assert!(state.task_exists(&task_id));
        let node_tasks = state.get_tasks_by_node("worker");
        assert_eq!(node_tasks.len(), 0);
    }

    #[test]
    fn test_task_failure_cleanup() {
        let state = ClusterState::new();

        // Add node and create task
        state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        let task_id = state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);

        // Assign and verify active count
        state.assign_task(&task_id, "worker").unwrap();
        assert_eq!(state.get_node_active_task_count("worker"), 1);

        // Fail task and verify cleanup
        state.fail_task(&task_id, "Processing error").unwrap();
        assert_eq!(state.get_node_active_task_count("worker"), 0);

        // Verify task state
        let task = state.get_task(&task_id).unwrap();
        assert_eq!(task.state, TaskState::Failed);
        assert_eq!(task.error_message, Some("Processing error".to_string()));
    }

    #[test]
    fn test_remove_task_cleanup() {
        let state = ClusterState::new();

        // Add node and create task
        state.upsert_node(create_test_node(
            "worker",
            NodeRole::Follower,
            NodeStatus::Healthy,
        ));
        let task_id = state.create_task("client-1".to_string(), "test.wav".to_string(), None, None);

        // Assign task
        state.assign_task(&task_id, "worker").unwrap();
        assert_eq!(state.get_node_active_task_count("worker"), 1);

        // Remove task and verify cleanup
        let removed = state.remove_task(&task_id).unwrap();
        assert_eq!(removed.task_id, task_id);
        assert!(!state.task_exists(&task_id));
        assert_eq!(state.get_node_active_task_count("worker"), 0);
    }
}
