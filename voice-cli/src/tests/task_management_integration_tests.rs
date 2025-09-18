#[cfg(test)]
mod task_management_integration_tests {
    use crate::models::{
        AsyncTranscriptionTask, Config, TaskManagementConfig, TaskStatus,
    };
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn create_test_config() -> (Arc<Config>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_tasks.db");

        let mut config = Config::default();
        config.task_management = TaskManagementConfig {
            max_concurrent_tasks: 2,
            retry_attempts: 2,
            task_timeout_seconds: 30,
            catch_panic: true,
            sqlite_db_path: db_path.to_string_lossy().to_string(),
            task_retention_minutes: 1440, // 24 hours in minutes
            sled_db_path: "./data/sled".to_string(),
        };

        (Arc::new(config), temp_dir)
    }

    // #[tokio::test]
    // async fn test_task_store_crud_operations() {
    //     let (config, _temp_dir) = create_test_config().await;
    //     let task_store = Arc::new(TaskStore::from_config(&config.task_management).await.unwrap());

    //     // Create test task
    //     let task = AsyncTranscriptionTask::new(
    //         "test-crud-task".to_string(),
    //         PathBuf::from("test.mp3"),
    //         "test.mp3".to_string(),
    //         Some("base".to_string()),
    //         Some("json".to_string()),
    //     );

    //     // Test save task
    //     task_store.save_task("test-crud-task", &task).await.unwrap();

    //     // Test get task
    //     let retrieved_task = task_store.get_task("test-crud-task").await.unwrap();
    //     assert!(retrieved_task.is_some());
    //     let retrieved_task = retrieved_task.unwrap();
    //     assert_eq!(retrieved_task.task_id, "test-crud-task");
    //     assert_eq!(retrieved_task.original_filename, "test.mp3");

    //     // Test get status
    //     let status = task_store.get_status("test-crud-task").await.unwrap();
    //     assert!(status.is_some());
    //     assert!(matches!(status.unwrap(), TaskStatus::Pending { .. }));
    // }
}