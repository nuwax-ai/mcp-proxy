use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::mpsc;
use document_parser::utils::environment_manager::{
    EnvironmentManager, InstallProgress, RetryConfig, IssueSeverity
};

#[tokio::test]
async fn test_environment_manager_with_retry_config() {
    let temp_dir = TempDir::new().unwrap();
    let retry_config = RetryConfig {
        max_attempts: 2,
        base_delay: Duration::from_millis(100),
        max_delay: Duration::from_secs(5),
        backoff_multiplier: 2.0,
    };
    
    let manager = EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    )
    .with_retry_config(retry_config)
    .with_timeout(Duration::from_secs(30))
    .with_cache_ttl(Duration::from_secs(60));
    
    // 测试环境检查
    let result = manager.check_environment().await;
    assert!(result.is_ok());
    
    let status = result.unwrap();
    assert!(status.health_score() <= 100);
    
    // 测试缓存功能
    let cached_result = manager.check_environment().await;
    assert!(cached_result.is_ok());
}

#[tokio::test]
async fn test_environment_manager_with_progress_tracking() {
    let temp_dir = TempDir::new().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel::<InstallProgress>();
    
    let manager = EnvironmentManager::with_progress_tracking(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
        tx,
    );
    
    // 在后台任务中监听进度
    let progress_task = tokio::spawn(async move {
        let mut progress_count = 0;
        while let Some(progress) = rx.recv().await {
            progress_count += 1;
            println!("Progress: {} - {} ({}%)", 
                progress.package, progress.message, progress.progress);
            
            // 避免无限等待
            if progress_count > 10 {
                break;
            }
        }
        progress_count
    });
    
    // 执行环境检查
    let result = manager.check_environment().await;
    assert!(result.is_ok());
    
    // 等待进度任务完成（带超时）
    let progress_result = tokio::time::timeout(
        Duration::from_secs(5), 
        progress_task
    ).await;
    
    // 验证进度跟踪是否工作
    if let Ok(Ok(count)) = progress_result {
        println!("Received {} progress updates", count);
    }
}

#[tokio::test]
async fn test_environment_status_analysis() {
    let temp_dir = TempDir::new().unwrap();
    let manager = EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    );
    
    let status = manager.check_environment().await.unwrap();
    
    // 测试状态分析方法
    println!("Environment ready: {}", status.is_ready());
    println!("Health score: {}/100", status.health_score());
    println!("Has CUDA support: {}", status.has_cuda_support());
    
    // 测试问题分析
    let critical_issues = status.get_critical_issues();
    let auto_fixable_issues = status.get_auto_fixable_issues();
    
    println!("Critical issues: {}", critical_issues.len());
    println!("Auto-fixable issues: {}", auto_fixable_issues.len());
    
    for issue in critical_issues {
        assert_eq!(issue.severity, IssueSeverity::Critical);
        println!("Critical issue: {} - {}", issue.component, issue.message);
    }
    
    for issue in auto_fixable_issues {
        assert!(issue.auto_fixable);
        println!("Auto-fixable issue: {} - {}", issue.component, issue.suggestion);
    }
}

#[tokio::test]
async fn test_environment_reporting() {
    let temp_dir = TempDir::new().unwrap();
    let manager = EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    );
    
    // 测试环境报告生成
    let report = manager.generate_environment_report().await;
    assert!(report.is_ok());
    
    let report_content = report.unwrap();
    assert!(report_content.contains("=== 环境检查报告 ==="));
    assert!(report_content.contains("=== 组件状态 ==="));
    
    println!("Environment Report:\n{}", report_content);
    
    // 测试环境摘要
    let summary = manager.get_environment_summary().await;
    assert!(summary.is_ok());
    
    let summary_content = summary.unwrap();
    assert!(summary_content.contains("环境状态:"));
    assert!(summary_content.contains("健康评分:"));
    
    println!("Environment Summary: {}", summary_content);
}

#[tokio::test]
async fn test_environment_validation() {
    let temp_dir = TempDir::new().unwrap();
    let manager = EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    );
    
    // 测试环境验证
    let is_valid = manager.validate_environment().await;
    assert!(is_valid.is_ok());
    
    let validation_result = is_valid.unwrap();
    println!("Environment validation result: {}", validation_result);
    
    // 测试引擎验证
    let engines_valid = manager.validate_engines().await;
    assert!(engines_valid.is_ok());
    
    let engines_result = engines_valid.unwrap();
    println!("Engines validation result: {}", engines_result);
}

#[tokio::test]
async fn test_cache_functionality() {
    let temp_dir = TempDir::new().unwrap();
    let manager = EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    ).with_cache_ttl(Duration::from_secs(1)); // 短缓存时间用于测试
    
    // 第一次检查
    let start_time = std::time::Instant::now();
    let result1 = manager.check_environment().await.unwrap();
    let first_check_duration = start_time.elapsed();
    
    // 立即第二次检查（应该使用缓存）
    let start_time = std::time::Instant::now();
    let result2 = manager.check_environment().await.unwrap();
    let second_check_duration = start_time.elapsed();
    
    // 缓存的检查应该更快
    assert!(second_check_duration < first_check_duration);
    
    // 等待缓存过期
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // 清除缓存
    manager.clear_cache().await;
    
    // 第三次检查（缓存已过期）
    let start_time = std::time::Instant::now();
    let result3 = manager.check_environment().await.unwrap();
    let third_check_duration = start_time.elapsed();
    
    // 过期后的检查应该比缓存检查慢
    assert!(third_check_duration > second_check_duration);
    
    println!("First check: {:?}", first_check_duration);
    println!("Second check (cached): {:?}", second_check_duration);
    println!("Third check (expired): {:?}", third_check_duration);
}

#[tokio::test]
async fn test_concurrent_environment_checks() {
    let temp_dir = TempDir::new().unwrap();
    let manager = std::sync::Arc::new(EnvironmentManager::new(
        "python3".to_string(),
        temp_dir.path().to_string_lossy().to_string(),
    ));
    
    // 并发执行多个环境检查
    let mut handles = Vec::new();
    
    for i in 0..5 {
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let result = manager_clone.check_environment().await;
            println!("Concurrent check {} completed", i);
            result
        });
        handles.push(handle);
    }
    
    // 等待所有检查完成
    let results = futures::future::join_all(handles).await;
    
    // 验证所有检查都成功
    for (i, result) in results.into_iter().enumerate() {
        assert!(result.is_ok(), "Concurrent check {} failed", i);
        let status = result.unwrap().unwrap();
        assert!(status.health_score() <= 100);
    }
}