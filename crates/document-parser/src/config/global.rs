//! 全局状态：GLOBAL_CONFIG / GLOBAL_CUDA_STATUS 的初始化与读取、文件大小与
//! CUDA 相关便捷函数。

use super::*;

/// 全局配置实例
static GLOBAL_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// 初始化全局配置
pub fn init_global_config(config: AppConfig) -> Result<(), ConfigError> {
    // 检查是否已经初始化
    if GLOBAL_CONFIG.get().is_some() {
        return Ok(()); // 已经初始化过了，直接返回成功
    }

    GLOBAL_CONFIG
        .set(config)
        .map_err(|_| ConfigError::Validation {
            field: "global_config".to_string(),
            message: "全局配置已经初始化过了".to_string(),
        })?;
    Ok(())
}

/// 获取全局配置引用
pub fn get_global_config() -> &'static AppConfig {
    GLOBAL_CONFIG
        .get()
        .expect("全局配置尚未初始化，请先调用 init_global_config")
}

/// 获取全局文件大小配置
pub fn get_global_file_size_config() -> &'static GlobalFileSizeConfig {
    let max_file_sieze = &get_global_config().file_size_config;
    info!("Global file size configuration: {:?}", max_file_sieze);
    max_file_sieze
}

/// 便捷函数：获取指定用途的文件大小限制
pub fn get_file_size_limit(purpose: &FileSizePurpose) -> &'static FileSize {
    get_global_file_size_config().get_file_size_limit(purpose)
}

/// 便捷函数：检查文件大小是否允许
pub fn is_file_size_allowed(file_size: u64, purpose: FileSizePurpose) -> bool {
    get_global_file_size_config().is_size_allowed(file_size, purpose)
}

/// 便捷函数：检查是否为大文档
pub fn is_large_document(file_size: u64) -> bool {
    get_global_file_size_config().is_large_document(file_size)
}

/// 便捷函数：获取大文档阈值
pub fn get_large_document_threshold() -> u64 {
    get_global_file_size_config().get_large_document_threshold()
}

/// 便捷函数：检查CUDA是否可用
pub fn is_cuda_available() -> bool {
    get_global_cuda_status_clone().available
}

/// 便捷函数：获取推荐的CUDA设备
pub fn get_recommended_cuda_device() -> Option<String> {
    get_global_cuda_status_clone().recommended_device
}

/// 全局CUDA状态管理，使用线程安全的Arc<RwLock<>>
static GLOBAL_CUDA_STATUS: OnceLock<Arc<RwLock<CudaStatus>>> = OnceLock::new();

/// 初始化全局CUDA状态
pub fn init_global_cuda_status(cuda_status: CudaStatus) -> Result<(), ConfigError> {
    GLOBAL_CUDA_STATUS
        .set(Arc::new(RwLock::new(cuda_status)))
        .map_err(|_| ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "全局CUDA状态已经初始化过了".to_string(),
        })?;
    Ok(())
}

/// 更新全局CUDA状态
pub fn update_global_cuda_status(cuda_status: CudaStatus) -> Result<(), ConfigError> {
    if let Some(global_status) = GLOBAL_CUDA_STATUS.get() {
        let mut status = global_status.write().map_err(|_| ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "无法获取CUDA状态写锁".to_string(),
        })?;
        *status = cuda_status;
        Ok(())
    } else {
        Err(ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "全局CUDA状态尚未初始化".to_string(),
        })
    }
}

/// 获取全局CUDA状态的克隆
pub fn get_global_cuda_status_clone() -> CudaStatus {
    if let Some(global_status) = GLOBAL_CUDA_STATUS.get()
        && let Ok(status) = global_status.read()
    {
        return status.clone();
    }
    warn!("The global CUDA state has not been initialized and returns to the default value.");
    CudaStatus::default()
}
