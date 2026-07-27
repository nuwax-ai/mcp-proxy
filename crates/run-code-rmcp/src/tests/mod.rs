#[cfg(test)]
pub mod test_utils {
    use log::LevelFilter;
    use log::info;
    use std::fs;
    use std::path::Path;
    use std::sync::Once;

    // 使用 Once 确保 env_logger 只初始化一次
    static INIT: Once = Once::new();
    // 控制是否清理缓存的环境变量名
    const CLEAN_CACHE_ENV: &str = "CLEAN_TEST_CACHE";

    pub fn setup() {
        INIT.call_once(|| {
            // 尝试初始化日志，如果失败则忽略
            let _ = std::panic::catch_unwind(|| {
                env_logger::builder().filter_level(LevelFilter::Info).init();
            });
        });

        // 只有当环境变量设置为"1"时才清理缓存
        if let Ok(clean_cache) = std::env::var(CLEAN_CACHE_ENV)
            && clean_cache == "1"
        {
            clean_cache_dir();
        }
    }

    // 清理缓存目录函数
    fn clean_cache_dir() {
        let cache_dir = Path::new("/tmp/code_cache");

        // 如果缓存目录存在，清理其中的文件
        if cache_dir.exists() {
            info!("正在清理缓存目录: {cache_dir:?}");

            match fs::read_dir(cache_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            if let Err(e) = fs::remove_file(&path) {
                                info!("删除文件失败 {path:?}: {e}");
                            } else {
                                info!("已删除缓存文件: {path:?}");
                            }
                        }
                    }
                    info!("缓存目录清理完成");
                }
                Err(e) => {
                    info!("读取缓存目录失败: {e}");
                }
            }
        } else {
            // 如果缓存目录不存在，创建它
            info!("缓存目录不存在，创建目录: {cache_dir:?}");
            if let Err(e) = fs::create_dir_all(cache_dir) {
                info!("创建缓存目录失败: {e}");
            } else {
                info!("缓存目录创建成功");
            }
        }

        // 确保缓存目录存在且可写
        if !cache_dir.exists()
            && let Err(e) = fs::create_dir_all(cache_dir)
        {
            info!("创建缓存目录失败: {e}");
        }

        // 设置目录权限为777（所有用户可读写执行）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(cache_dir, fs::Permissions::from_mode(0o775)) {
                info!("设置缓存目录权限失败: {e}");
            } else {
                info!("设置缓存目录权限成功");
            }
        }
    }
}

pub mod js_tests;
pub mod python_tests;
pub mod ts_tests;
