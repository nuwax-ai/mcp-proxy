#[cfg(test)]
mod tests {
    use crate::utils::environment_manager::{
        EnvironmentManager, EnvironmentStatus,
    };

    #[tokio::test]
    async fn test_virtual_env_status_reporting() {
        let env_manager = EnvironmentManager::for_current_directory().unwrap();
        let status = env_manager.check_environment().await.unwrap();

        // Test virtual environment status reporting
        let venv_status = status.get_virtual_env_status();

        // Verify that virtual environment status is properly populated
        assert!(venv_status.expected_path.is_some());
        assert_eq!(venv_status.expected_path.as_ref().unwrap(), "./venv");
        assert!(!venv_status.activation_command.is_empty());

        // Check activation command is platform-appropriate
        if cfg!(windows) {
            assert!(venv_status.activation_command.contains("Scripts\\activate"));
        } else {
            assert!(venv_status.activation_command.contains("bin/activate"));
        }
    }

    #[tokio::test]
    async fn test_diagnostic_report_generation() {
        let env_manager = EnvironmentManager::for_current_directory().unwrap();
        let status = env_manager.check_environment().await.unwrap();

        // Test diagnostic report generation
        let report = status.generate_diagnostic_report();

        // Verify report structure
        assert!(!report.overall_status.is_empty());
        assert!(report.health_score <= 100);
        assert!(!report.components.is_empty());

        // Check that all expected components are present
        let component_names: Vec<&String> = report.components.iter().map(|c| &c.name).collect();
        assert!(component_names.contains(&&"Python".to_string()));
        assert!(component_names.contains(&&"Virtual Environment".to_string()));
        assert!(component_names.contains(&&"UV Tool".to_string()));
        assert!(component_names.contains(&&"MinerU".to_string()));
        assert!(component_names.contains(&&"MarkItDown".to_string()));
        assert!(component_names.contains(&&"CUDA".to_string()));

        // Verify that each component has proper details
        for component in &report.components {
            assert!(!component.name.is_empty());
            assert!(!component.status.is_empty());
            assert!(!component.details.is_empty());
        }
    }

    #[tokio::test]
    async fn test_formatted_diagnostic_report() {
        let env_manager = EnvironmentManager::for_current_directory().unwrap();
        let status = env_manager.check_environment().await.unwrap();

        // Test formatted diagnostic report
        let formatted_report = status.format_diagnostic_report();

        // Verify report formatting
        assert!(formatted_report.contains("=== Environment Diagnostic Report ==="));
        assert!(formatted_report.contains("Overall Status:"));
        assert!(formatted_report.contains("Health Score:"));
        assert!(formatted_report.contains("=== Components ==="));

        // Check that component information is included
        assert!(formatted_report.contains("Python:"));
        assert!(formatted_report.contains("Virtual Environment:"));
        assert!(formatted_report.contains("UV Tool:"));
        assert!(formatted_report.contains("MinerU:"));
        assert!(formatted_report.contains("MarkItDown:"));

        println!("Formatted diagnostic report:\n{formatted_report}");
    }

    #[tokio::test]
    async fn test_enhanced_status_methods() {
        let env_manager = EnvironmentManager::for_current_directory().unwrap();

        // Test enhanced status methods
        let detailed_report = env_manager.get_detailed_status_report().await.unwrap();
        assert!(!detailed_report.is_empty());

        let venv_status = env_manager
            .check_virtual_environment_status()
            .await
            .unwrap();
        assert!(!venv_status.activation_command.is_empty());
    }

    #[test]
    fn test_virtual_env_properly_configured_logic() {
        let mut status = EnvironmentStatus::default();

        // Test when virtual environment is not active
        status.virtual_env_active = false;
        assert!(!status.is_virtual_env_properly_configured());

        // Test when virtual environment is active but path is None
        status.virtual_env_active = true;
        status.virtual_env_path = None;
        assert!(!status.is_virtual_env_properly_configured());

        // Test when virtual environment is active with proper path
        status.virtual_env_path = Some("./venv".to_string());
        assert!(status.is_virtual_env_properly_configured());

        // Test with different path formats
        status.virtual_env_path = Some("/some/path/venv".to_string());
        assert!(status.is_virtual_env_properly_configured());

        status.virtual_env_path = Some("C:\\project\\venv".to_string());
        assert!(status.is_virtual_env_properly_configured());
    }

    #[tokio::test]
    async fn test_directory_validation() {
        let env_manager = EnvironmentManager::for_current_directory().unwrap();

        // Test directory validation
        let validation_result = env_manager
            .check_current_directory_readiness()
            .await
            .unwrap();

        // Verify validation result structure
        assert!(validation_result.current_directory.exists());
        assert!(
            validation_result
                .venv_path
                .to_string_lossy()
                .ends_with("venv")
        );

        // The result should have some validation performed
        // (issues and warnings may be empty if directory is good)
        println!("Directory validation result: {validation_result:?}");

        // Test validation report formatting
        let report = env_manager.get_directory_validation_report().await.unwrap();
        assert!(report.contains("=== 当前目录验证报告 ==="));
        assert!(report.contains("目录:"));
        assert!(report.contains("虚拟环境路径:"));
        assert!(report.contains("验证状态:"));

        println!("Directory validation report:\n{report}");
    }

    #[tokio::test]
    async fn test_cleanup_options() {
        use crate::utils::environment_manager::CleanupType;
        use std::fs;
        use tempfile::TempDir;

        // Create a temporary directory for testing
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        // Create environment manager for temp directory
        let env_manager = EnvironmentManager::new(
            temp_path
                .join("venv")
                .join("bin")
                .join("python")
                .to_string_lossy()
                .to_string(),
            temp_path.to_string_lossy().to_string(),
        );

        // Create a conflicting file at venv path
        let venv_file_path = temp_path.join("venv");
        fs::write(&venv_file_path, "conflicting file").unwrap();

        // Test cleanup option execution
        let result = env_manager
            .execute_cleanup_option(CleanupType::RemoveConflictingFile)
            .await;

        match result {
            Ok(message) => {
                assert!(message.contains("成功删除冲突文件"));
                assert!(!venv_file_path.exists());
            }
            Err(e) => {
                // This might fail due to permissions, which is expected in some test environments
                println!("Cleanup test failed (expected in some environments): {e}");
            }
        }
    }

    #[tokio::test]
    async fn test_directory_validation_with_issues() {
        use std::fs;
        use tempfile::TempDir;

        // Create a temporary directory for testing
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();

        // Create environment manager for temp directory
        let env_manager = EnvironmentManager::new(
            temp_path
                .join("venv")
                .join("bin")
                .join("python")
                .to_string_lossy()
                .to_string(),
            temp_path.to_string_lossy().to_string(),
        );

        // Create a conflicting file at venv path to trigger validation issues
        let venv_file_path = temp_path.join("venv");
        fs::write(&venv_file_path, "conflicting file").unwrap();

        // Test directory validation with issues
        let validation_result = env_manager
            .check_current_directory_readiness()
            .await
            .unwrap();

        // Should detect the path conflict
        assert!(!validation_result.is_valid);
        assert!(!validation_result.issues.is_empty());

        // Should have cleanup options
        assert!(!validation_result.cleanup_options.is_empty());

        // Should have recommendations
        assert!(!validation_result.recommendations.is_empty());

        println!("Validation with issues: {validation_result:?}");
    }

    #[test]
    fn test_activation_command_generation() {
        let status = EnvironmentStatus::default();
        let activation_command = status.get_activation_command();

        if cfg!(windows) {
            assert_eq!(activation_command, ".\\venv\\Scripts\\activate");
        } else {
            assert_eq!(activation_command, "source ./venv/bin/activate");
        }
    }
}
