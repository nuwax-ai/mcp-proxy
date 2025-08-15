//! 工具层单元测试

use tempfile::TempDir;
use crate::utils::*;
use crate::models::*;

#[cfg(test)]
mod file_utils_tests {
    use crate::utils::*;
    use crate::models::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_exists() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.txt");
        
        // 文件不存在
        assert!(!file_exists(test_file.to_str().unwrap()));
        
        // 创建文件
        std::fs::write(&test_file, "test content").expect("Failed to write test file");
        assert!(file_exists(test_file.to_str().unwrap()));
    }

    #[tokio::test]
    async fn test_create_temp_dir() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_dir = temp_dir.path().join("test_subdir");
        
        // 目录不存在时应该创建
        let result = create_temp_dir(test_dir.to_str().unwrap());
        assert!(result.is_ok());
        assert!(test_dir.exists());
        
        // 目录已存在时应该成功
        let result2 = create_temp_dir(test_dir.to_str().unwrap());
        assert!(result2.is_ok());
    }

    #[tokio::test]
    async fn test_get_file_size() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.txt");
        
        // 创建测试文件
        let test_content = "Hello, World! This is a test file.";
        std::fs::write(&test_file, test_content).expect("Failed to write test file");
        
        let result = get_file_size(test_file.to_str().unwrap());
        assert!(result.is_ok());
        
        let size = result.unwrap();
        assert_eq!(size, test_content.len() as u64);
    }

    #[tokio::test]
    async fn test_get_file_size_nonexistent() {
        let result = get_file_size("/nonexistent/file.txt");
        assert!(result.is_err());
        
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(
            error_msg.contains("not found") || 
            error_msg.contains("No such file"),
            "Expected file not found error, got: {}", error_msg
        );
    }

    #[tokio::test]
    async fn test_file_copy_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let source_file = temp_dir.path().join("source.txt");
        let dest_file = temp_dir.path().join("dest.txt");
        
        // 创建源文件
        let test_content = "This is test content for copying.";
        std::fs::write(&source_file, test_content).expect("Failed to write source file");
        
        // 使用标准库进行文件复制
        let result = std::fs::copy(&source_file, &dest_file);
        assert!(result.is_ok());
        
        // 验证文件已复制
        assert!(dest_file.exists());
        let copied_content = std::fs::read_to_string(&dest_file).expect("Failed to read dest file");
        assert_eq!(copied_content, test_content);
        
        // 源文件应该仍然存在
        assert!(source_file.exists());
        
        // 测试复制不存在的文件
        let nonexistent_file = temp_dir.path().join("nonexistent.txt");
        let result = std::fs::copy(&nonexistent_file, &dest_file);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_move_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let source_file = temp_dir.path().join("source.txt");
        let dest_file = temp_dir.path().join("dest.txt");
        
        // 创建源文件
        let test_content = "This is test content for moving.";
        std::fs::write(&source_file, test_content).expect("Failed to write source file");
        
        // 使用标准库进行文件移动
        let result = std::fs::rename(&source_file, &dest_file);
        assert!(result.is_ok());
        
        // 验证文件已移动
        assert!(dest_file.exists());
        assert!(!source_file.exists()); // 源文件应该不存在
        
        let moved_content = std::fs::read_to_string(&dest_file).expect("Failed to read dest file");
        assert_eq!(moved_content, test_content);
    }

    #[tokio::test]
    async fn test_file_delete_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("to_delete.txt");
        
        // 创建测试文件
        std::fs::write(&test_file, "Content to be deleted").expect("Failed to write test file");
        assert!(test_file.exists());
        
        // 使用标准库删除文件
        let result = std::fs::remove_file(&test_file);
        assert!(result.is_ok());
        assert!(!test_file.exists());
        
        // 测试删除不存在的文件
        let nonexistent_file = temp_dir.path().join("nonexistent.txt");
        let result = std::fs::remove_file(&nonexistent_file);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_directory_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        
        // 创建测试文件
        let files = vec!["file1.txt", "file2.pdf", "file3.docx"];
        for file in &files {
            let file_path = temp_dir.path().join(file);
            std::fs::write(&file_path, "test content").expect("Failed to write test file");
        }
        
        // 创建子目录
        let subdir = temp_dir.path().join("subdir");
        std::fs::create_dir(&subdir).expect("Failed to create subdir");
        
        // 验证文件和目录存在
        for file in &files {
            let file_path = temp_dir.path().join(file);
            assert!(file_path.exists());
        }
        assert!(subdir.exists());
        assert!(subdir.is_dir());
    }

    #[tokio::test]
    async fn test_get_file_extension() {
        assert_eq!(get_file_extension("test.pdf"), Some("pdf".to_string()));
        assert_eq!(get_file_extension("document.docx"), Some("docx".to_string()));
        assert_eq!(get_file_extension("image.PNG"), Some("png".to_string())); // 转小写
        assert_eq!(get_file_extension("file_without_extension"), None);
        assert_eq!(get_file_extension(".hidden"), None);
        assert_eq!(get_file_extension("path/to/file.txt"), Some("txt".to_string()));
    }

    #[tokio::test]
    async fn test_filename_operations() {
        // 测试文件扩展名获取
        assert_eq!(get_file_extension("normal_file.txt"), Some("txt".to_string()));
        assert_eq!(get_file_extension("file with spaces.pdf"), Some("pdf".to_string()));
        assert_eq!(get_file_extension("file.docx"), Some("docx".to_string()));
        assert_eq!(get_file_extension("中文文件名.pdf"), Some("pdf".to_string())); // 保留中文
        assert_eq!(get_file_extension("no_extension"), None);
    }
}

#[cfg(test)]
mod format_utils_tests {
    use crate::utils::*;
    use crate::models::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_detect_format_from_path() {
        // 测试基于文件扩展名的格式检测
        assert_eq!(detect_format_from_path("test.pdf").unwrap(), DocumentFormat::PDF);
        assert_eq!(detect_format_from_path("document.docx").unwrap(), DocumentFormat::Word);
        assert_eq!(detect_format_from_path("document.doc").unwrap(), DocumentFormat::Word);
        assert_eq!(detect_format_from_path("presentation.pptx").unwrap(), DocumentFormat::PowerPoint);
        assert_eq!(detect_format_from_path("presentation.ppt").unwrap(), DocumentFormat::PowerPoint);
        assert_eq!(detect_format_from_path("spreadsheet.xlsx").unwrap(), DocumentFormat::Excel);
        assert_eq!(detect_format_from_path("spreadsheet.xls").unwrap(), DocumentFormat::Excel);
        assert_eq!(detect_format_from_path("image.png").unwrap(), DocumentFormat::Image);
        assert_eq!(detect_format_from_path("image.jpg").unwrap(), DocumentFormat::Image);
        assert_eq!(detect_format_from_path("image.jpeg").unwrap(), DocumentFormat::Image);
        // 对于未知扩展名，函数返回 DocumentFormat::Other 而不是错误
        assert!(matches!(detect_format_from_path("unknown.xyz").unwrap(), DocumentFormat::Other(_)));
        // 无扩展名的文件会返回错误
        assert!(detect_format_from_path("no_extension").is_err());
    }

    #[tokio::test]
    async fn test_detect_format_from_path_case_insensitive() {
        assert_eq!(detect_format_from_path("test.PDF").unwrap(), DocumentFormat::PDF);
        assert_eq!(detect_format_from_path("document.DOCX").unwrap(), DocumentFormat::Word);
        assert_eq!(detect_format_from_path("image.PNG").unwrap(), DocumentFormat::Image);
    }

    #[tokio::test]
    async fn test_is_format_supported() {
        assert!(is_format_supported(&DocumentFormat::PDF));
        assert!(is_format_supported(&DocumentFormat::Word));
        assert!(is_format_supported(&DocumentFormat::PowerPoint));
        assert!(is_format_supported(&DocumentFormat::Excel));
        assert!(is_format_supported(&DocumentFormat::Image));
        // DocumentFormat::Unknown 不存在，移除此测试
    }



    #[tokio::test]
    async fn test_file_size_formatting() {
        // 由于format_file_size函数不存在，我们测试文件大小的基本操作
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("format_test.txt");
        
        // 创建不同大小的测试文件
        let small_content = "small";
        std::fs::write(&test_file, small_content).expect("Failed to write test file");
        
        let size = get_file_size(test_file.to_str().unwrap()).unwrap();
        assert_eq!(size, small_content.len() as u64);
    }

    #[tokio::test]
    async fn test_file_size_operations() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("size_test.txt");
        let test_content = "This is test content for size testing.";
        
        std::fs::write(&test_file, test_content).expect("Failed to write test file");
        
        let size = get_file_size(test_file.to_str().unwrap()).unwrap();
        assert_eq!(size, test_content.len() as u64);
        
        // 测试不存在的文件
        let result = get_file_size("/nonexistent/file.txt");
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod validation_utils_tests {
    use crate::utils::*;
    use crate::models::*;

    #[tokio::test]
    async fn test_file_extension_validation() {
        // 测试文件扩展名获取
        assert_eq!(get_file_extension("document.pdf"), Some("pdf".to_string()));
        assert_eq!(get_file_extension("presentation.pptx"), Some("pptx".to_string()));
        assert_eq!(get_file_extension("image_001.png"), Some("png".to_string()));
        assert_eq!(get_file_extension("report-2023.docx"), Some("docx".to_string()));
        assert_eq!(get_file_extension("中文文档.pdf"), Some("pdf".to_string()));
        
        // 无效的文件名
        assert_eq!(get_file_extension(""), None);
        assert_eq!(get_file_extension("noextension"), None);
    }

    #[tokio::test]
    async fn test_format_detection_validation() {
        // 测试格式检测
        assert!(detect_format_from_path("document.pdf").is_ok());
        assert!(detect_format_from_path("presentation.pptx").is_ok());
        assert!(detect_format_from_path("image.png").is_ok());
        
        // 无扩展名应该失败
        assert!(detect_format_from_path("noextension").is_err());
        assert!(detect_format_from_path("").is_err());
    }

    #[tokio::test]
    async fn test_format_support_validation() {
        // 测试格式支持检查
        assert!(is_format_supported(&DocumentFormat::PDF));
        assert!(is_format_supported(&DocumentFormat::Word));
        assert!(is_format_supported(&DocumentFormat::Excel));
        assert!(is_format_supported(&DocumentFormat::PowerPoint));
        assert!(is_format_supported(&DocumentFormat::Image));
        // DocumentFormat::Unknown 不存在，移除此测试
    }
}

#[cfg(test)]
mod time_utils_tests {
    use chrono::Utc;

    #[tokio::test]
    async fn test_current_timestamp() {
        let timestamp = Utc::now();
        
        // 验证时间戳是合理的（不是默认值）
        assert!(timestamp.timestamp() > 0);
        
        // 验证时间戳是最近的（在过去1分钟内）
        let now = Utc::now();
        let diff = now.signed_duration_since(timestamp);
        assert!(diff.num_seconds() < 60);
    }
}



#[cfg(test)]
mod config_utils_tests {
    use crate::utils::*;
    use crate::models::*;

    #[tokio::test]
    async fn test_env_var_operations() {
        // 设置环境变量
        unsafe {
            std::env::set_var("TEST_VAR", "test_value");
        }
        
        let value = std::env::var("TEST_VAR");
        assert!(value.is_ok());
        assert_eq!(value.unwrap(), "test_value");
        
        // 清理环境变量
        unsafe {
            std::env::remove_var("TEST_VAR");
        }
        
        // 验证变量已被移除
        let missing_value = std::env::var("TEST_VAR");
        assert!(missing_value.is_err());
    }
}

#[cfg(test)]
mod error_utils_tests {
    use crate::utils::*;
    use crate::models::*;

    #[tokio::test]
    async fn test_error_handling() {
        let error = anyhow::anyhow!("Test error message");
        let error_string = error.to_string();
        
        assert!(error_string.contains("Test error message"));
        assert!(!error_string.is_empty());
    }
}

#[cfg(test)]
mod integration_utils_tests {
    use chrono::Utc;
    use tempfile::TempDir;
    use crate::utils::*;
    use crate::models::*;

    #[tokio::test]
    async fn test_file_operations_integration() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("integration_test.txt");
        
        let test_content = "Integration test content";
        
        // 1. 创建文件
        std::fs::write(&test_file, test_content).expect("Failed to write test file");
        
        // 2. 获取文件大小
        let size = get_file_size(test_file.to_str().unwrap()).unwrap();
        assert_eq!(size, test_content.len() as u64);
        
        // 3. 获取文件扩展名
        let extension = get_file_extension(test_file.to_str().unwrap());
        assert_eq!(extension, Some("txt".to_string()));
        
        // 4. 验证文件存在
        assert!(file_exists(test_file.to_str().unwrap()));
    }

    #[tokio::test]
    async fn test_validation_integration() {
        let filename = "test_document.pdf";
        
        // 检测文档格式
        let format = detect_format_from_path(filename).unwrap();
        assert_eq!(format, DocumentFormat::PDF);
        assert!(is_format_supported(&format));
    }

    #[tokio::test]
    async fn test_time_integration() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("time_test.txt");
        
        let test_content = "Content for time testing";
        std::fs::write(&test_file, test_content).expect("Failed to write test file");
        
        // 时间操作
        let now = Utc::now();
        let later = Utc::now();
        
        // 验证时间戳是递增的
        assert!(later >= now);
    }
}