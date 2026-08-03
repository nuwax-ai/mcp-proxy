use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("file prefix must contain only ASCII letters, digits, '.', '_' or '-'")]
    InvalidPrefix,
    #[error("failed to create cache directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to set permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cache directory {path} is not a real directory")]
    InvalidDirectoryType { path: PathBuf },
    #[error("existing cache target {path} is not a regular file")]
    InvalidExistingType { path: PathBuf },
    #[error("failed to create temporary cache file in {path}: {source}")]
    CreateTemporary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write temporary cache file for {path}: {source}")]
    WriteTemporary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to persist cache file {path}: {source}")]
    Persist {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("existing content-addressed file {path} does not match its expected content")]
    ExistingContentMismatch { path: PathBuf },
    #[error("failed to verify existing cache file {path}: {source}")]
    VerifyExisting {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to sync cache directory {path}: {source}")]
    SyncDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn validate_prefix(prefix: &str) -> Result<(), CacheError> {
    if prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CacheError::InvalidPrefix);
    }
    Ok(())
}

fn ensure_cache_dir(path: &Path) -> Result<(), CacheError> {
    fs::create_dir_all(path).map_err(|source| CacheError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|source| CacheError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_dir() {
        return Err(CacheError::InvalidDirectoryType {
            path: path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            CacheError::SetPermissions {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    Ok(())
}

fn verify_existing(path: &Path, expected: &[u8]) -> Result<(), CacheError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| CacheError::VerifyExisting {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.file_type().is_file() {
        return Err(CacheError::InvalidExistingType {
            path: path.to_path_buf(),
        });
    }
    let existing = fs::read(path).map_err(|source| CacheError::VerifyExisting {
        path: path.to_path_buf(),
        source,
    })?;
    if existing != expected {
        return Err(CacheError::ExistingContentMismatch {
            path: path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            CacheError::SetPermissions {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

pub(crate) fn write_content_addressed(
    cache_dir: &Path,
    prefix: &str,
    kind: &'static str,
    bytes: &[u8],
) -> Result<(PathBuf, bool), CacheError> {
    validate_prefix(prefix)?;
    ensure_cache_dir(cache_dir)?;

    let digest = Sha256::digest(bytes);
    let hash = format!("{digest:x}");
    let target = cache_dir.join(format!("{prefix}-{kind}-{hash}.json"));

    if target.exists() {
        verify_existing(&target, bytes)?;
        return Ok((target, false));
    }

    let mut temporary =
        NamedTempFile::new_in(cache_dir).map_err(|source| CacheError::CreateTemporary {
            path: cache_dir.to_path_buf(),
            source,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| CacheError::SetPermissions {
                path: target.clone(),
                source,
            })?;
    }

    temporary
        .as_file_mut()
        .write_all(bytes)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|source| CacheError::WriteTemporary {
            path: target.clone(),
            source,
        })?;

    match temporary.persist_noclobber(&target) {
        Ok(_) => {
            #[cfg(unix)]
            fs::File::open(cache_dir)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| CacheError::SyncDirectory {
                    path: cache_dir.to_path_buf(),
                    source,
                })?;
            Ok((target, true))
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_existing(&target, bytes)?;
            Ok((target, false))
        }
        Err(error) => Err(CacheError::Persist {
            path: target,
            source: error.error,
        }),
    }
}
