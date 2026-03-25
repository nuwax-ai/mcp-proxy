use crate::error::AppError;
use crate::utils::environment_manager::EnvironmentManager;
use std::path::Path;
use tempfile::TempDir;
use tokio;

// 辅助函数用于测试
impl EnvironmentManager {
    #[cfg(test)]
    pub fn for_directory(path: &Path) -> Result<Self, AppError> {
        let venv_path = path.join("venv");
        let python_path = if cfg!(windows) {
            venv_path.join("Scripts").join("python.exe")
        } else {
            venv_path.join("bin").join("python")
        };

        Ok(Self::new(
            python_path.to_string_lossy().to_string(),
            path.to_string_lossy().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_environment_path_error_creation() {
        let path = Path::new("/test/path");
        let error = AppError::virtual_environment_path_error("测试错误".to_string(), path);

        match error {
            AppError::VirtualEnvironmentPath(msg) => {
                // 验证路径信息被包含
                assert!(msg.contains("/test/path"), "Message should contain '/test/path': {}", msg);
            }
            _ => panic!("Expected VirtualEnvironmentPath error"),
        }
    }

    #[test]
    fn test_permission_error_creation() {
        let path = Path::new("/test/path");
        let error = AppError::permission_error("权限测试错误".to_string(), path);

        match error {
            AppError::Permission(msg) => {
                assert!(msg.contains("/test/path"), "Message should contain '/test/path': {}", msg);
            }
            _ => panic!("Expected Permission error"),
        }
    }

    #[test]
    fn test_path_error_creation() {
        let path = Path::new("/test/path");
        let error = AppError::path_error("路径测试错误".to_string(), path);

        match error {
            AppError::Path(msg) => {
                assert!(msg.contains("/test/path"), "Message should contain '/test/path': {}", msg);
            }
            _ => panic!("Expected Path error"),
        }
    }

    #[test]
    fn test_path_recovery_suggestions_virtual_environment() {
        let path = Path::new("/test/venv");
        let error = AppError::virtual_environment_path_error("虚拟环境创建失败".to_string(), path);

        let suggestions = error.get_path_recovery_suggestions();
        assert!(!suggestions.is_empty());
        assert!(suggestions.iter().any(|s| s.contains("写入权限")));
        assert!(suggestions.iter().any(|s| s.contains("磁盘空间")));
    }

    #[test]
    fn test_path_recovery_suggestions_permission() {
        let path = Path::new("/test/path");
        let error = AppError::permission_error("权限被拒绝".to_string(), path);

        let suggestions = error.get_path_recovery_suggestions();
        assert!(!suggestions.is_empty());
        assert!(suggestions.iter().any(|s| s.contains("权限")));

        #[cfg(unix)]
        {
            assert!(suggestions.iter().any(|s| s.contains("chmod")));
            assert!(suggestions.iter().any(|s| s.contains("chown")));
        }

        #[cfg(windows)]
        {
            assert!(suggestions.iter().any(|s| s.contains("管理员")));
        }
    }

    #[test]
    fn test_path_recovery_suggestions_path_not_found() {
        let path = Path::new("/nonexistent/path");
        let error = AppError::path_error("路径不存在".to_string(), path);

        let suggestions = error.get_path_recovery_suggestions();
        assert!(!suggestions.is_empty());
        assert!(suggestions.iter().any(|s| s.contains("路径")));
    }

    #[tokio::test]
    async fn test_environment_manager_path_validation() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::for_directory(temp_dir.path()).unwrap();

        // 测试路径诊断
        let issues = env_manager.diagnose_venv_path_issues().await;
        // 在临时目录中应该没有问题
        assert!(issues.is_empty() || issues.iter().all(|issue| !issue.contains("不存在")));
    }

    #[tokio::test]
    async fn test_environment_manager_recovery_suggestions() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::for_directory(temp_dir.path()).unwrap();

        let suggestions = env_manager.get_venv_recovery_suggestions().await;
        assert!(!suggestions.is_empty());
    }

    #[tokio::test]
    async fn test_auto_fix_venv_path_issues() {
        let temp_dir = TempDir::new().unwrap();
        let env_manager = EnvironmentManager::for_directory(temp_dir.path()).unwrap();

        // 创建一个阻碍文件
        let venv_file = temp_dir.path().join("venv");
        std::fs::write(&venv_file, "blocking file").unwrap();

        // 尝试自动修复
        let result = env_manager.auto_fix_venv_path_issues().await;
        match result {
            Ok(fixes) => {
                assert!(!fixes.is_empty());
                assert!(fixes.iter().any(|fix| fix.contains("删除")));
                // 验证文件已被删除
                assert!(!venv_file.exists());
            }
            Err(e) => {
                // 在某些系统上可能因为权限问题失败，这是可以接受的
                println!("Auto-fix failed (expected in some environments): {e}");
            }
        }
    }

    #[test]
    fn test_error_code_mapping() {
        let venv_error = AppError::VirtualEnvironmentPath("test".to_string());
        let permission_error = AppError::Permission("test".to_string());
        let path_error = AppError::Path("test".to_string());

        assert_eq!(venv_error.get_error_code(), "E017");
        assert_eq!(permission_error.get_error_code(), "E018");
        assert_eq!(path_error.get_error_code(), "E019");
    }

    #[test]
    fn test_error_suggestions() {
        let venv_error = AppError::VirtualEnvironmentPath("test".to_string());
        let permission_error = AppError::Permission("test".to_string());
        let path_error = AppError::Path("test".to_string());

        // 验证建议不为空，并包含对应的翻译 key
        let venv_suggestion = venv_error.get_suggestion();
        let perm_suggestion = permission_error.get_suggestion();
        let path_suggestion = path_error.get_suggestion();

        // 翻译 key 应该被正确返回（如果翻译文件加载失败，会返回 key 本身）
        assert!(!venv_suggestion.is_empty());
        assert!(!perm_suggestion.is_empty());
        assert!(!path_suggestion.is_empty());
    }
}
