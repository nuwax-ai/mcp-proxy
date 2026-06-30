#[cfg(test)]
mod tests {
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
