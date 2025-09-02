/// Task ID generation utilities
use uuid::Uuid;

/// Generate a clean task ID with only alphanumeric characters
/// Format: "task" + 32-character hexadecimal string (from UUID v7)
/// 
/// # Examples
/// 
/// ```
/// let task_id = generate_task_id();
/// assert!(task_id.starts_with("task"));
/// assert!(!task_id.contains('-'));
/// assert!(!task_id.contains('_'));
/// assert_eq!(task_id.len(), 36); // "task" + 32 hex chars
/// ```
pub fn generate_task_id() -> String {
    let uuid = Uuid::now_v7().to_string();
    // 移除所有连字符和下划线，只保留字母和数字
    let cleaned_uuid = uuid.replace(['-', '_'], "");
    format!("task{}", cleaned_uuid)
}

/// Generate a task ID with timestamp and process ID (legacy format)
/// This is kept for backward compatibility but new code should use generate_task_id()
/// 
/// # Examples
/// 
/// ```
/// let task_id = generate_legacy_task_id();
/// assert!(task_id.starts_with("task_"));
/// ```
#[deprecated(since = "0.1.0", note = "use `generate_task_id` instead")]
pub fn generate_legacy_task_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    
    format!(
        "task_{}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        std::process::id()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_task_id_format() {
        let task_id = generate_task_id();
        
        // Check format
        assert!(task_id.starts_with("task"));
        assert!(!task_id.contains('-'));
        assert!(!task_id.contains('_'));
        
        // Check length (task + 32 hex chars)
        assert_eq!(task_id.len(), 36);
        
        // Check that it's all alphanumeric after "task" prefix
        let suffix = &task_id[4..];
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_task_id_uniqueness() {
        let id1 = generate_task_id();
        let id2 = generate_task_id();
        
        // Should be different (due to timestamp in UUID v7)
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_generate_legacy_task_id() {
        let task_id = generate_legacy_task_id();
        
        // Check format
        assert!(task_id.starts_with("task_"));
        
        // Should contain underscore and timestamp
        assert!(task_id.contains('_'));
    }
}