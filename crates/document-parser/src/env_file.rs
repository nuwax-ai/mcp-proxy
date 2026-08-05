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
    use tempfile::tempdir;

    #[test]
    fn candidates_prefer_config_dir() {
        let paths = candidate_env_paths(Some(Path::new("/opt/dp/config.yml")));
        assert_eq!(paths[0], PathBuf::from("/opt/dp/.document-parser.env"));
        assert_eq!(paths[1], PathBuf::from(".document-parser.env"));
    }

    #[test]
    fn load_sets_missing_vars_only() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(ENV_FILENAME);
        fs::write(
            &env_path,
            "OSS_ACCESS_KEY_ID=from-file\nOSS_ACCESS_KEY_SECRET=secret-from-file\n",
        )
        .unwrap();

        // SAFETY: test-only env mutation
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

        unsafe {
            std::env::remove_var("OSS_ACCESS_KEY_ID");
            std::env::remove_var("OSS_ACCESS_KEY_SECRET");
        }
    }
}
