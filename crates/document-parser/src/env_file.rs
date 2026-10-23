//! Load `.document-parser.env` into the process environment before config resolution.
//!
//! Existing environment variables are never overwritten (systemd `EnvironmentFile=` / shell
//! exports take precedence). Missing files are ignored.

use log::{debug, info};
use std::path::{Path, PathBuf};

const ENV_FILENAME: &str = ".document-parser.env";

/// Resolve candidate paths for the dotenv file given an optional `--config` path.
pub fn candidate_env_paths(config: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(cfg) = config
        && let Some(parent) = cfg.parent()
    {
        let beside = parent.join(ENV_FILENAME);
        if !paths.iter().any(|p| p == &beside) {
            paths.push(beside);
        }
    }
    let cwd = PathBuf::from(ENV_FILENAME);
    if !paths.iter().any(|p| p == &cwd) {
        paths.push(cwd);
    }
    paths
}

/// Load the first existing dotenv candidate. Does not override existing env vars.
///
/// Returns the path that was loaded, if any.
pub fn load_document_parser_env(config: Option<&Path>) -> Option<PathBuf> {
    for path in candidate_env_paths(config) {
        if !path.is_file() {
            debug!(
                "document-parser env file not found, skipping: {}",
                path.display()
            );
            continue;
        }
        // dotenvy::from_path sets a var only when it is not already present.
        match dotenvy::from_path(&path) {
            Ok(()) => {
                info!("loaded document-parser env file: {}", path.display());
                return Some(path);
            }
            Err(e) => {
                debug!(
                    "failed to load document-parser env file {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    /// 测试改写共享环境变量，cargo test 默认并行——串行化防互踩
    /// （mcp-common/process_compat 同款）
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Drop 守卫恢复 env：断言失败 panic 时也把变量清掉，不泄漏给同进程其他测试
    struct EnvRestore {
        keys: &'static [&'static str],
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for key in self.keys {
                // SAFETY: test-only env mutation
                unsafe {
                    std::env::remove_var(key);
                }
            }
        }
    }

    #[test]
    fn candidates_prefer_config_dir() {
        let paths = candidate_env_paths(Some(Path::new("/opt/dp/config.yml")));
        assert_eq!(paths[0], PathBuf::from("/opt/dp/.document-parser.env"));
        assert_eq!(paths[1], PathBuf::from(".document-parser.env"));
    }

    #[test]
    fn load_sets_missing_vars_only() {
        let _env = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _restore = EnvRestore {
            keys: &["OSS_ACCESS_KEY_ID", "OSS_ACCESS_KEY_SECRET"],
        };
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(ENV_FILENAME);
        fs::write(
            &env_path,
            "OSS_ACCESS_KEY_ID=from-file\nOSS_ACCESS_KEY_SECRET=secret-from-file\n",
        )
        .unwrap();

        // SAFETY: test-only env mutation（env_lock 串行 + EnvRestore 兜底清理）
        unsafe {
            std::env::remove_var("OSS_ACCESS_KEY_ID");
            std::env::set_var("OSS_ACCESS_KEY_SECRET", "already-set");
        }

        let cfg = dir.path().join("config.yml");
        fs::write(&cfg, "server:\n  port: 1\n").unwrap();
        let loaded = load_document_parser_env(Some(&cfg));
        assert_eq!(loaded.as_deref(), Some(env_path.as_path()));
        assert_eq!(std::env::var("OSS_ACCESS_KEY_ID").unwrap(), "from-file");
        assert_eq!(
            std::env::var("OSS_ACCESS_KEY_SECRET").unwrap(),
            "already-set"
        );
    }
}
